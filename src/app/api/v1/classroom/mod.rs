use std::future::Future;
use std::ops::Bound;
use std::sync::Arc;

use anyhow::{Context, Result as AnyResult};
use async_std::prelude::FutureExt;
use serde_derive::{Deserialize, Serialize};
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::class::Object as Class;

use super::{extract_id, extract_param, validate_token, AppResult};

#[derive(Serialize)]
struct ClassroomObject {
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

impl From<Class> for ClassroomObject {
    fn from(obj: Class) -> ClassroomObject {
        ClassroomObject {
            id: obj.scope().to_owned(),
            real_time: RealTimeObject {
                fallback_uri: None,
                conference_room_id: obj.conference_room_id(),
                event_room_id: obj.event_room_id(),
            },
        }
    }
}

pub async fn read(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    read_inner(&req, find_classroom(&req))
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    read_inner(&req, find_classroom_by_scope(&req))
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}

async fn read_inner(
    req: &Request<Arc<dyn AppContext>>,
    finder: impl Future<Output = AnyResult<Class>>,
) -> AppResult {
    let state = req.state();
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;

    let classroom = match finder.await {
        Ok(classroom) => classroom,
        Err(e) => {
            error!(crate::LOG, "Failed to find a classroom, err = {:?}", e);
            return Ok(tide::Response::builder(404).body("Not found").build());
        }
    };

    let object = AuthzObject::new(&["classrooms", &classroom.id().to_string()]).into();

    state
        .authz()
        .authorize(
            classroom.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await?;

    let classroom_obj: ClassroomObject = classroom.into();

    let body = serde_json::to_string(&classroom_obj)
        .context("Failed to serialize classroom")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();
    Ok(response)
}

#[derive(Deserialize)]
struct Classroom {
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
    let body: Classroom = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

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

    let conference_fut = req.state().conference_client().create_room(
        (Bound::Unbounded, Bound::Unbounded),
        body.audience.clone(),
        None,
        None,
        body.tags.clone(),
    );

    let event_fut = req.state().event_client().create_room(
        (Bound::Unbounded, Bound::Unbounded),
        body.audience.clone(),
        Some(false),
        body.tags.clone(),
    );

    let (event_room_id, conference_room_id) = event_fut
        .try_join(conference_fut)
        .await
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    let query = crate::db::class::ClassroomInsertQuery::new(
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
    let classroom = query
        .execute(&mut conn)
        .await
        .context("Failed to insert classroom")
        .error(AppErrorKind::DbQueryFailed)?;

    let body = serde_json::to_string_pretty(&classroom)
        .context("Failed to serialize classroom")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct ClassroomConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    conference_room_id: Uuid,
    tags: Option<serde_json::Value>,
}

pub async fn convert(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    convert_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn convert_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body = req
        .body_json::<ClassroomConvertObject>()
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

    let query = crate::db::class::ClassroomInsertQuery::new(
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

    let classroom = {
        let mut conn = req
            .state()
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;

        let classroom = query
            .execute(&mut conn)
            .await
            .context("Failed to find recording")
            .error(AppErrorKind::DbQueryFailed)?;

        classroom
    };

    let body = serde_json::to_string(&classroom)
        .context("Failed to serialize classroom")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

async fn find_classroom(req: &Request<Arc<dyn AppContext>>) -> AnyResult<crate::db::class::Object> {
    let id = extract_id(req)?;

    let classroom = {
        let mut conn = req.state().get_conn().await?;
        crate::db::class::ClassroomReadQuery::by_id(id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find classroom by scope"))?
    };
    Ok(classroom)
}

async fn find_classroom_by_scope(
    req: &Request<Arc<dyn AppContext>>,
) -> AnyResult<crate::db::class::Object> {
    let audience = extract_param(req, "audience")?.to_owned();
    let scope = extract_param(req, "scope")?.to_owned();

    let classroom = {
        let mut conn = req.state().get_conn().await?;
        crate::db::class::ClassroomReadQuery::by_scope(audience.clone(), scope.clone())
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find classroom by scope"))?
    };
    Ok(classroom)
}
