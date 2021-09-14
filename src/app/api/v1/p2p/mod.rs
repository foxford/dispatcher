use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::Deserialize;
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{
    api::v1::class::{read as read_generic, read_by_scope as read_by_scope_generic},
    metrics::AuthorizeMetrics,
};
use crate::{app::authz::AuthzObject, db::class::P2PType};

use super::{validate_token, AppResult};

pub async fn read(req: Request<Arc<dyn AppContext>>) -> AppResult {
    read_generic::<P2PType>(req).await
}

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> AppResult {
    read_by_scope_generic::<P2PType>(req).await
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
        .await
        .measure()?;

    info!(log, "Authorized p2p create");

    let conference_fut = req.state().conference_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        None,
        None,
        body.tags.clone(),
        None
    );

    let event_fut = req.state().event_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        Some(false),
        body.tags.clone(),
        None
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
    let p2p = {
        let mut conn = req
            .state()
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;
        query
            .execute(&mut conn)
            .await
            .context("Failed to insert p2p")
            .error(AppErrorKind::DbQueryFailed)?
    };
    info!(log, "Inserted p2p into db, id = {}", p2p.id());

    crate::app::services::update_classroom_id(
        req.state().as_ref(),
        p2p.id(),
        p2p.event_room_id(),
        p2p.conference_room_id(),
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
        .await
        .measure()?;

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
        p2p.conference_room_id(),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string(&p2p)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}
