use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::{Deserialize, Serialize};
use svc_authn::AccountId;
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::class::BoundedDateTimeTuple;
use crate::db::class::Object as Class;

use super::{extract_id, extract_param, validate_token, AppResult};

#[derive(Serialize)]
struct MinigroupObject<'a> {
    id: String,
    real_time: RealTimeObject,
    host: &'a AccountId
}

#[derive(Serialize)]
struct RealTimeObject {
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtc_id: Option<Uuid>,
}

impl<'a> From<&'a Class> for MinigroupObject<'a> {
    fn from(obj: &'a Class) -> Self {
        Self {
            id: obj.scope().to_owned(),
            real_time: RealTimeObject {
                conference_room_id: obj.conference_room_id(),
                event_room_id: obj.event_room_id(),
                rtc_id: None,
            },
            // Minigroups have host enforced by db
            host: obj.host().unwrap()
        }
    }
}

pub async fn read(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    read_inner(req).await.or_else(|e| Ok(e.to_tide_response()))
}
async fn read_inner(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let minigroup = find_minigroup(&req)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["minigroups", &minigroup.id().to_string()]).into();
    state
        .authz()
        .authorize(
            minigroup.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await?;

    let minigroup_obj: MinigroupObject = (&minigroup).into();

    let body = serde_json::to_string(&minigroup_obj)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;
    let response = Response::builder(200).body(body).build();
    Ok(response)
}

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    read_by_scope_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn read_by_scope_inner(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let state = req.state();
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;

    let minigroup = match find_minigroup_by_scope(&req).await {
        Ok(minigroup) => minigroup,
        Err(e) => {
            error!(crate::LOG, "Failed to find a minigroup, err = {:?}", e);
            return Ok(tide::Response::builder(404).body("Not found").build());
        }
    };

    let object = AuthzObject::new(&["minigroups", &minigroup.id().to_string()]).into();

    state
        .authz()
        .authorize(
            minigroup.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await?;

    let minigroup_obj: MinigroupObject = (&minigroup).into();

    let body = serde_json::to_string(&minigroup_obj)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();
    Ok(response)
}

#[derive(Deserialize)]
struct Minigroup {
    scope: String,
    audience: String,
    host: AccountId,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
    tags: Option<serde_json::Value>,
    reserve: Option<i32>,
    #[serde(default)]
    locked_chat: bool,
}

pub async fn create(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    create_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn create_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: Minigroup = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let object = AuthzObject::new(&["minigroups"]).into();

    state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "create".into(),
        )
        .await?;

    let conference_time = match body.time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), Bound::Unbounded),
        Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
    };
    let conference_fut = req.state().conference_client().create_room(
        conference_time,
        body.audience.clone(),
        Some("owned".into()),
        body.reserve,
        body.tags.clone(),
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = req.state().event_client().create_room(
        event_time,
        body.audience.clone(),
        Some(true),
        body.tags.clone(),
    );

    let (event_room_id, conference_room_id) = event_fut
        .try_join(conference_fut)
        .await
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    let query = crate::db::class::MinigroupInsertQuery::new(
        body.scope,
        body.audience,
        body.time.into(),
        body.host,
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
    let minigroup = query
        .execute(&mut conn)
        .await
        .context("Failed to insert minigroup")
        .error(AppErrorKind::DbQueryFailed)?;

    if body.locked_chat {
        if let Err(e) = req.state().event_client().lock_chat(event_room_id).await {
            error!(
                crate::LOG,
                "Failed to lock chat in event room, id = {:?}, err = {:?}", event_room_id, e
            );
        }
    }

    let body = serde_json::to_string_pretty(&minigroup)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct MinigroupUpdate {
    #[serde(with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    host: Option<AccountId>,
}

pub async fn update(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    update_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn update_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: MinigroupUpdate = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let minigroup = find_minigroup(&req)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["minigroups", &minigroup.id().to_string()]).into();

    state
        .authz()
        .authorize(
            minigroup.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await?;

    if let Some(time) = &body.time {
        let conference_time = match time.0 {
            Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), time.1),
            Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
        };
        let conference_fut = req
            .state()
            .conference_client()
            .update_room(minigroup.id(), conference_time);

        let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
        let event_fut = req
            .state()
            .event_client()
            .update_room(minigroup.id(), event_time);

        event_fut
            .try_join(conference_fut)
            .await
            .context("Services requests")
            .error(AppErrorKind::MqttRequestFailed)?;
    }

    let mut query = crate::db::class::MinigroupTimeUpdateQuery::new(minigroup.id());
    if let Some(t) = body.time {
        query.time(t.into());
    }
    if let Some(h) = body.host {
        query.host(h);
    }

    let mut conn = req
        .state()
        .get_conn()
        .await
        .error(AppErrorKind::DbQueryFailed)?;
    let minigroup = query
        .execute(&mut conn)
        .await
        .context("Failed to update minigroup")
        .error(AppErrorKind::DbQueryFailed)?;

    let body = serde_json::to_string(&minigroup)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();

    Ok(response)
}

async fn find_minigroup(
    req: &Request<Arc<dyn AppContext>>,
) -> anyhow::Result<crate::db::class::Object> {
    let id = extract_id(req)?;

    let minigroup = {
        let mut conn = req.state().get_conn().await?;
        crate::db::class::MinigroupReadQuery::by_id(id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find minigroup"))?
    };
    Ok(minigroup)
}

async fn find_minigroup_by_scope(
    req: &Request<Arc<dyn AppContext>>,
) -> anyhow::Result<crate::db::class::Object> {
    let audience = extract_param(req, "audience")?.to_owned();
    let scope = extract_param(req, "scope")?.to_owned();

    let minigroup = {
        let mut conn = req.state().get_conn().await?;
        crate::db::class::MinigroupReadQuery::by_scope(audience.clone(), scope.clone())
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find minigroup by scope"))?
    };
    Ok(minigroup)
}
