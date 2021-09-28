use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, TypedHeader};
use chrono::Utc;
use headers::{authorization::Bearer, Authorization};
use hyper::{Body, Response};
use serde_derive::Deserialize;
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::metrics::AuthorizeMetrics;
use crate::app::AppContext;

use super::{validate_token, AppResult};

#[derive(Deserialize)]
pub struct P2P {
    scope: String,
    audience: String,
    tags: Option<serde_json::Value>,
}

pub async fn create(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(body): Json<P2P>,
) -> AppResult {
    let log = crate::LOG.new(slog::o!(
        "audience" => body.audience.clone(),
        "scope" => body.scope.clone(),
    ));
    info!(
        log,
        "Creating p2p, audience = {}, scope = {}", body.audience, body.scope
    );

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

    info!(log, "Authorized p2p create");

    let conference_fut = ctx.conference_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        None,
        None,
        body.tags.clone(),
        None,
    );

    let event_fut = ctx.event_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        Some(false),
        body.tags.clone(),
        None,
    );

    let (event_room_id, conference_room_id) = tokio::try_join!(event_fut, conference_fut)
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
        let mut conn = ctx
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
        ctx.as_ref(),
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

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

#[derive(Deserialize)]
pub struct P2PConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    conference_room_id: Uuid,
    tags: Option<serde_json::Value>,
}

pub async fn convert(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(body): Json<P2PConvertObject>,
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
        let mut conn = ctx
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
        ctx.as_ref(),
        p2p.id(),
        p2p.event_room_id(),
        p2p.conference_room_id(),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string(&p2p)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}
