use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use serde_derive::{Deserialize, Serialize};
use svc_authn::AccountId;
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::chat::Object as Chat;

type AppResult = Result<tide::Response, crate::app::error::Error>;

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

impl From<Chat> for ChatObject {
    fn from(obj: Chat) -> Self {
        Self {
            id: obj.scope(),
            real_time: RealTimeObject {
                fallback_uri: None,
                event_room_id: obj.event_room_id(),
            },
        }
    }
}

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    read_by_scope_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn read_by_scope_inner(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let state = req.state();
    let account_id = fetch_token(&req).error(AppErrorKind::Unauthorized)?;

    let chat = match find_chat_by_scope(&req).await {
        Ok(chat) => chat,
        Err(e) => {
            error!(crate::LOG, "Failed to find a chat, err = {:?}", e);
            return Ok(tide::Response::builder(404).body("Not found").build());
        }
    };

    let object = AuthzObject::new(&["chats", &chat.id().to_string()]).into();

    state
        .authz()
        .authorize(chat.audience(), account_id.clone(), object, "read".into())
        .await?;

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

pub async fn create(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    create_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn create_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: ChatPayload = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = fetch_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let object = AuthzObject::new(&["chats"]).into();

    state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "create".into(),
        )
        .await?;

    let event_room_id = req
        .state()
        .event_client()
        .create_room(
            (Bound::Unbounded, Bound::Unbounded),
            body.audience.clone(),
            Some(true),
            body.tags.clone(),
        )
        .await
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    let query = crate::db::chat::ChatInsertQuery::new(body.scope, body.audience, event_room_id);

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };

    let mut conn = req
        .state()
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    let chat = query
        .execute(&mut conn)
        .await
        .context("Failed to insert chat")
        .error(AppErrorKind::DbQueryFailed)?;

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

pub async fn convert(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    convert_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn convert_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body = req
        .body_json::<ChatConvertObject>()
        .await
        .error(AppErrorKind::InvalidPayload)?;

    let account_id = fetch_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let object = AuthzObject::new(&["chats"]).into();

    state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "convert".into(),
        )
        .await?;

    let query =
        crate::db::chat::ChatInsertQuery::new(body.scope, body.audience, body.event_room_id);

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

    let body = serde_json::to_string(&chat)
        .context("Failed to serialize chat")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

fn fetch_token<T: std::ops::Deref<Target = dyn AppContext>>(
    req: &Request<T>,
) -> anyhow::Result<AccountId> {
    let token = req
        .header("Authorization")
        .and_then(|h| h.get(0))
        .map(|header| header.to_string());

    let state = req.state();
    let account_id = state
        .validate_token(token.as_deref())
        .context("Token authentication failed")?;

    Ok(account_id)
}

fn extract_param<'a>(req: &'a Request<Arc<dyn AppContext>>, key: &str) -> anyhow::Result<&'a str> {
    req.param(key)
        .map_err(|e| anyhow!("Failed to get {}, reason = {:?}", key, e))
}

async fn find_chat_by_scope(req: &Request<Arc<dyn AppContext>>) -> anyhow::Result<Chat> {
    let audience = extract_param(req, "audience")?.to_owned();
    let scope = extract_param(req, "scope")?.to_owned();

    let chat = {
        let mut conn = req.state().get_conn().await?;
        crate::db::chat::ChatReadQuery::by_scope(audience.clone(), scope.clone())
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find chat by scope"))?
    };
    Ok(chat)
}
