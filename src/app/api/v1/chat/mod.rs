use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, TypedHeader};
use chrono::Utc;
use headers::{authorization::Bearer, Authorization};
use hyper::{Body, Response};
use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{error::ErrorExt, metrics::AuthorizeMetrics};
use crate::db::class::{ChatInsertQuery, Object as Class};

use super::{validate_token, AppResult};

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

#[derive(Deserialize)]
pub struct ChatPayload {
    scope: String,
    audience: String,
    tags: Option<serde_json::Value>,
}

pub async fn create(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(body): Json<ChatPayload>,
) -> AppResult {
    let account_id =
        validate_token(ctx.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;

    let object = AuthzObject::new(&["classrooms"]).into();

    ctx.authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "create".into(),
        )
        .await
        .measure()?;

    let event_room_id = ctx
        .event_client()
        .create_room(
            (Bound::Included(Utc::now()), Bound::Unbounded),
            body.audience.clone(),
            Some(true),
            body.tags.clone(),
            None,
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
        let mut conn = ctx
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;
        query
            .execute(&mut conn)
            .await
            .context("Failed to insert chat")
            .error(AppErrorKind::DbQueryFailed)?
    };
    crate::app::services::update_classroom_id(ctx.as_ref(), chat.id(), chat.event_room_id(), None)
        .await
        .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string_pretty(&chat)
        .context("Failed to serialize chat")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

#[derive(Deserialize)]
pub struct ChatConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    tags: Option<serde_json::Value>,
}

pub async fn convert(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(body): Json<ChatConvertObject>,
) -> AppResult {
    let account_id =
        validate_token(ctx.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;

    let object = AuthzObject::new(&["classrooms"]).into();

    ctx.authz()
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
        let mut conn = ctx
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

    crate::app::services::update_classroom_id(ctx.as_ref(), chat.id(), chat.event_room_id(), None)
        .await
        .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string(&chat)
        .context("Failed to serialize chat")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}
