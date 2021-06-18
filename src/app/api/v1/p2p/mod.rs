use std::future::Future;
use std::ops::Bound;
use std::sync::Arc;

use anyhow::{Context, Result as AnyResult};
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::{Deserialize, Serialize};
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::class::Object as Class;
use crate::{app::authz::AuthzObject, db::class::P2PType};

use super::{extract_id, extract_param, find, find_by_scope, validate_token, AppResult};

#[derive(Serialize)]
struct P2PObject {
    id: String,
    real_time: RealTimeObject,
}

#[derive(Serialize)]
struct RealTimeObject {
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_uri: Option<String>,
}

impl From<Class> for P2PObject {
    fn from(obj: Class) -> P2PObject {
        P2PObject {
            id: obj.scope().to_owned(),
            real_time: RealTimeObject {
                fallback_uri: None,
                conference_room_id: obj.conference_room_id(),
                event_room_id: obj.event_room_id(),
            },
        }
    }
}

pub async fn read_p2p(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let state = req.state();
    let id = extract_id(&req).error(AppErrorKind::InvalidParameter)?;

    read(&req, find::<P2PType>(state.as_ref(), id)).await
}

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let audience = extract_param(&req, "audience").error(AppErrorKind::InvalidParameter)?;
    let scope = extract_param(&req, "scope").error(AppErrorKind::InvalidParameter)?;
    let state = req.state();

    read(
        &req,
        find_by_scope::<P2PType>(state.as_ref(), audience, scope),
    )
    .await
}

pub async fn read(
    req: &Request<Arc<dyn AppContext>>,
    finder: impl Future<Output = AnyResult<Class>>,
) -> AppResult {
    let state = req.state();
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;

    let p2p = match finder.await {
        Ok(p2p) => p2p,
        Err(e) => {
            error!(crate::LOG, "Failed to find a p2p, err = {:?}", e);
            return Ok(tide::Response::builder(404).body("Not found").build());
        }
    };

    let object = AuthzObject::new(&["classrooms", &p2p.id().to_string()]).into();

    state
        .authz()
        .authorize(
            p2p.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await?;

    let p2p_obj: P2PObject = p2p.into();

    let body = serde_json::to_string(&p2p_obj)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();
    Ok(response)
}

#[derive(Deserialize)]
struct P2P {
    scope: String,
    audience: String,
    tags: Option<serde_json::Value>,
}

pub async fn create(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: P2P = req.body_json().await.error(AppErrorKind::InvalidPayload)?;
    let log = crate::LOG.new(slog::o!(
        "audience" => body.audience.clone(),
        "scope" => body.scope.clone(),
    ));
    info!(
        log,
        "Creating p2p, audience = {}, scope = {}", body.audience, body.scope
    );

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
        .await?;

    info!(log, "Authorized p2p create");

    let conference_fut = req.state().conference_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        None,
        None,
        body.tags.clone(),
    );

    let event_fut = req.state().event_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        Some(false),
        body.tags.clone(),
    );

    let (event_room_id, conference_room_id) = event_fut
        .try_join(conference_fut)
        .await
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    info!(
        log,
        "Created event room = {}, conference room = {}", event_room_id, conference_room_id
    );

    let query = crate::db::class::P2PInsertQuery::new(
        body.scope,
        body.audience,
        conference_room_id,
        event_room_id,
    );

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
    let p2p = query
        .execute(&mut conn)
        .await
        .context("Failed to insert p2p")
        .error(AppErrorKind::DbQueryFailed)?;

    info!(log, "Inserted p2p into db, id = {}", p2p.id());

    crate::app::services::update_classroom_id(
        req.state().as_ref(),
        p2p.id(),
        p2p.event_room_id(),
        Some(p2p.conference_room_id()),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    info!(log, "Successfully updated classroom room id");

    let body = serde_json::to_string_pretty(&p2p)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct P2PConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    conference_room_id: Uuid,
    tags: Option<serde_json::Value>,
}

pub async fn convert(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body = req
        .body_json::<P2PConvertObject>()
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
        .await?;

    let query = crate::db::class::P2PInsertQuery::new(
        body.scope,
        body.audience,
        body.conference_room_id,
        body.event_room_id,
    );

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };

    let p2p = {
        let mut conn = req
            .state()
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;

        let p2p = query
            .execute(&mut conn)
            .await
            .context("Failed to find recording")
            .error(AppErrorKind::DbQueryFailed)?;

        p2p
    };

    crate::app::services::update_classroom_id(
        req.state().as_ref(),
        p2p.id(),
        p2p.event_room_id(),
        Some(p2p.conference_room_id()),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string(&p2p)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}
