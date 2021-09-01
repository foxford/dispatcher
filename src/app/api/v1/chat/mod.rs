use std::future::Future;
use std::ops::Bound;
use std::sync::Arc;

use anyhow::{Context, Result as AnyResult};
use chrono::Utc;
use serde_derive::{Deserialize, Serialize};
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{error::ErrorExt, metrics::AuthorizeMetrics};
use crate::db::class::{ChatInsertQuery, GenericReadQuery, Object as Class};
use crate::{app::authz::AuthzObject, db::class::ChatType};

use super::{extract_id, extract_param, validate_token, AppResult};

#[derive(Serialize)]
struct ChatObject {
    id: String,
    real_time: RealTimeObject,
}

#[derive(Serialize)]
struct RealTimeObject {
    event_room_id: Uuid,
    fallback_uri: Option<String>,
}

impl From<Class> for ChatObject {
    fn from(obj: Class) -> Self {
        Self {
            id: obj.scope().to_owned(),
            real_time: RealTimeObject {
                fallback_uri: None,
                event_room_id: obj.event_room_id(),
            },
        }
    }
}

pub async fn read_chat(req: Request<Arc<dyn AppContext>>) -> AppResult {
    read(&req, find_chat(&req)).await
}

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> AppResult {
    read(&req, find_chat_by_scope(&req)).await
}

async fn read(
    req: &Request<Arc<dyn AppContext>>,
    finder: impl Future<Output = AnyResult<Class>>,
) -> AppResult {
    let state = req.state();
    let account_id = validate_token(req).error(AppErrorKind::Unauthorized)?;

    let chat = match finder.await {
        Ok(chat) => chat,
        Err(e) => {
            error!(crate::LOG, "Failed to find a chat, err = {:?}", e);
            return Ok(tide::Response::builder(404).body("Not found").build());
        }
    };

    let object = AuthzObject::new(&["classrooms", &chat.id().to_string()]).into();

    state
        .authz()
        .authorize(
            chat.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await
        .measure()?;

    let chat_obj: ChatObject = chat.into();

    let body = serde_json::to_string(&chat_obj)
        .context("Failed to serialize chat")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();
    Ok(response)
}

#[derive(Deserialize)]
struct ChatPayload {
    scope: String,
    audience: String,
    tags: Option<serde_json::Value>,
}

pub async fn create(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: ChatPayload = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let object = AuthzObject::new(&["classrooms"]).into();

    state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "create".into(),
        )
        .await
        .measure()?;

    let event_room_id = req
        .state()
        .event_client()
        .create_room(
            (Bound::Included(Utc::now()), Bound::Unbounded),
            body.audience.clone(),
            Some(true),
            body.tags.clone(),
        )
        .await
        .map_err(|e| anyhow!("Failed to create event room, reason = {:?}", e))
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    let query = ChatInsertQuery::new(body.scope, body.audience, event_room_id);

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };
    let chat = {
        let mut conn = req
            .state()
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;
        query
            .execute(&mut conn)
            .await
            .context("Failed to insert chat")
            .error(AppErrorKind::DbQueryFailed)?
    };
    crate::app::services::update_classroom_id(
        req.state().as_ref(),
        chat.id(),
        chat.event_room_id(),
        None,
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string_pretty(&chat)
        .context("Failed to serialize chat")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct ChatConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    tags: Option<serde_json::Value>,
}

pub async fn convert(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body = req
        .body_json::<ChatConvertObject>()
        .await
        .error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let object = AuthzObject::new(&["classrooms"]).into();

    state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "convert".into(),
        )
        .await
        .measure()?;

    let query = ChatInsertQuery::new(body.scope, body.audience, body.event_room_id);

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };

    let chat = {
        let mut conn = req
            .state()
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;

        let chat = query
            .execute(&mut conn)
            .await
            .context("Failed to find recording")
            .error(AppErrorKind::DbQueryFailed)?;

        chat
    };

    crate::app::services::update_classroom_id(
        req.state().as_ref(),
        chat.id(),
        chat.event_room_id(),
        None,
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string(&chat)
        .context("Failed to serialize chat")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

async fn find_chat(req: &Request<Arc<dyn AppContext>>) -> anyhow::Result<Class> {
    let id = extract_id(req)?;

    let chat = {
        let mut conn = req.state().get_conn().await?;
        GenericReadQuery::<ChatType>::by_id(id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find chat by scope"))?
    };
    Ok(chat)
}

async fn find_chat_by_scope(req: &Request<Arc<dyn AppContext>>) -> anyhow::Result<Class> {
    let audience = extract_param(req, "audience")?.to_owned();
    let scope = extract_param(req, "scope")?.to_owned();

    let chat = {
        let mut conn = req.state().get_conn().await?;
        GenericReadQuery::<ChatType>::by_scope(&audience, &scope)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find chat by scope"))?
    };
    Ok(chat)
}
