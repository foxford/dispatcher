use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::{DateTime, Utc};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::Acquire;
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::clients::{conference::ConferenceRoomResponse, event::EventRoomResponse};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::class::Object as Class;
use crate::db::recording::Segments;

use super::{extract_id, extract_param, validate_token, AppResult};

#[derive(Serialize)]
struct WebinarObject {
    id: String,
    real_time: RealTimeObject,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    on_demand: Vec<WebinarVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

impl WebinarObject {
    pub fn add_version(&mut self, version: WebinarVersion) {
        self.on_demand.push(version);
    }

    pub fn set_status(&mut self, status: &str) {
        self.status = Some(status.to_owned());
    }

    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.real_time.set_rtc_id(rtc_id);
    }
}

#[derive(Serialize)]
struct WebinarVersion {
    version: &'static str,
    event_room_id: Uuid,
    // TODO: this is deprecated and should be removed eventually
    // right now its necessary to generate HLS links
    stream_id: Uuid,
    tags: Option<JsonValue>,
}

#[derive(Serialize)]
struct RealTimeObject {
    conference_room_id: Uuid,
    event_room_id: Uuid,
    fallback_uri: Option<String>,
    rtc_id: Option<Uuid>,
}

impl RealTimeObject {
    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.rtc_id = Some(rtc_id);
    }
}

impl From<Class> for WebinarObject {
    fn from(obj: Class) -> WebinarObject {
        WebinarObject {
            id: obj.scope(),
            real_time: RealTimeObject {
                fallback_uri: None,
                conference_room_id: obj.conference_room_id(),
                event_room_id: obj.event_room_id(),
                rtc_id: None,
            },
            on_demand: vec![],
            status: None,
        }
    }
}

pub async fn read(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    read_inner(req).await.or_else(|e| Ok(e.to_tide_response()))
}
async fn read_inner(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let webinar = find_webinar(&req)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["webinars", &webinar.id().to_string()]).into();
    state
        .authz()
        .authorize(
            webinar.audience(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await?;

    let mut conn = req
        .state()
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    let recording = crate::db::recording::RecordingReadQuery::new(webinar.id())
        .execute(&mut conn)
        .await
        .context("Failed to find recording")
        .error(AppErrorKind::DbQueryFailed)?;

    let mut webinar_obj: WebinarObject = webinar.clone().into();

    if let Some(recording) = recording {
        if let Some(og_event_id) = webinar.original_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: webinar.tags(),
            });
        }

        if recording.transcoded_at().is_some() {
            if let Some(md_event_id) = webinar.modified_event_room_id() {
                webinar_obj.add_version(WebinarVersion {
                    version: "modified",
                    stream_id: recording.rtc_id(),
                    event_room_id: md_event_id,
                    tags: webinar.tags(),
                });
            }

            webinar_obj.set_status("transcoded");
        } else if recording.adjusted_at().is_some() {
            webinar_obj.set_status("adjusted");
        } else {
            webinar_obj.set_status("finished");
        }
    } else {
        webinar_obj.set_status("real-time");
    }

    let body = serde_json::to_string(&webinar_obj)
        .context("Failed to serialize webinar")
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

    let webinar = match find_webinar_by_scope(&req).await {
        Ok(webinar) => webinar,
        Err(e) => {
            error!(crate::LOG, "Failed to find a webinar, err = {:?}", e);
            return Ok(tide::Response::builder(404).body("Not found").build());
        }
    };

    let object = AuthzObject::new(&["webinars", &webinar.id().to_string()]).into();

    state
        .authz()
        .authorize(
            webinar.audience(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await?;

    let mut conn = req
        .state()
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    let recording = crate::db::recording::RecordingReadQuery::new(webinar.id())
        .execute(&mut conn)
        .await
        .context("Failed to find recording")
        .error(AppErrorKind::DbQueryFailed)?;

    let mut webinar_obj: WebinarObject = webinar.clone().into();

    if let Some(recording) = recording {
        if let Some(og_event_id) = webinar.original_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: webinar.tags(),
            });
        }

        if let Some(md_event_id) = webinar.modified_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "modified",
                stream_id: recording.rtc_id(),
                event_room_id: md_event_id,
                tags: webinar.tags(),
            });
        }

        webinar_obj.set_rtc_id(recording.rtc_id());

        if recording.transcoded_at().is_some() {
            webinar_obj.set_status("transcoded");
        } else if recording.adjusted_at().is_some() {
            webinar_obj.set_status("adjusted");
        } else {
            webinar_obj.set_status("finished");
        }
    } else {
        webinar_obj.set_status("real-time");
    }

    let body = serde_json::to_string(&webinar_obj)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();
    Ok(response)
}

pub async fn options(_req: Request<Arc<dyn AppContext>>) -> tide::Result {
    Ok(Response::builder(200).build())
}

#[derive(Deserialize)]
struct Webinar {
    scope: String,
    audience: String,
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
    let body: Webinar = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let object = AuthzObject::new(&["webinars"]).into();

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
        Some("shared".into()),
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

    let query = crate::db::class::WebinarInsertQuery::new(
        body.scope,
        body.audience,
        body.time.into(),
        conference_room_id,
        event_room_id,
    );

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };

    let query = if let Some(reserve) = body.reserve {
        query.reserve(reserve)
    } else {
        query
    };

    let mut conn = req
        .state()
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    let webinar = query
        .execute(&mut conn)
        .await
        .context("Failed to insert webinar")
        .error(AppErrorKind::DbQueryFailed)?;

    if body.locked_chat {
        if let Err(e) = req.state().event_client().lock_chat(event_room_id).await {
            error!(
                crate::LOG,
                "Failed to lock chat in event room, id = {:?}, err = {:?}", event_room_id, e
            );
        }
    }

    let body = serde_json::to_string_pretty(&webinar)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct WebinarUpdate {
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
}

pub async fn update(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    update_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn update_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: WebinarUpdate = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let webinar = find_webinar(&req)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["webinars", &webinar.id().to_string()]).into();

    state
        .authz()
        .authorize(
            webinar.audience(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await?;

    let conference_time = match body.time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), body.time.1),
        Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
    };
    let conference_fut = req
        .state()
        .conference_client()
        .update_room(webinar.id(), conference_time);

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = req
        .state()
        .event_client()
        .update_room(webinar.id(), event_time);

    event_fut
        .try_join(conference_fut)
        .await
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    let query = crate::db::class::WebinarTimeUpdateQuery::new(webinar.id(), body.time.into());

    let mut conn = req
        .state()
        .get_conn()
        .await
        .error(AppErrorKind::DbQueryFailed)?;
    let webinar = query
        .execute(&mut conn)
        .await
        .context("Failed to update webinar")
        .error(AppErrorKind::DbQueryFailed)?;

    let body = serde_json::to_string(&webinar)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct WebinarConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    conference_room_id: Uuid,
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    tags: Option<serde_json::Value>,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
    recording: Option<RecordingConvertObject>,
}

#[derive(Deserialize)]
struct RecordingConvertObject {
    stream_id: Uuid,
    segments: Segments,
    modified_segments: Segments,
    uri: String,
}

pub async fn convert(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    convert_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}
async fn convert_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body = req
        .body_json::<WebinarConvertObject>()
        .await
        .error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let object = AuthzObject::new(&["webinars"]).into();

    state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "convert".into(),
        )
        .await?;

    let (time, tags) = match (body.time, body.tags) {
        // if we have both time and tags - lets use them
        (Some(t), Some(tags)) => (t, Some(tags)),
        // otherwise we try to fetch room time from conference and event
        _ => {
            let conference_fut = req
                .state()
                .conference_client()
                .read_room(body.conference_room_id);
            let event_fut = req.state().event_client().read_room(body.event_room_id);
            match event_fut.try_join(conference_fut).await {
                // if we got times back correctly lets pick the overlap of event and conf times
                Ok((
                    EventRoomResponse {
                        time: ev_time,
                        tags,
                        ..
                    },
                    ConferenceRoomResponse {
                        time: conf_time, ..
                    },
                )) => (times_overlap(ev_time, conf_time), tags),
                // if there was an error we actually dont care much about the time being correct
                Err(_) => ((Bound::Unbounded, Bound::Unbounded), None),
            }
        }
    };

    let query = crate::db::class::WebinarInsertQuery::new(
        body.scope,
        body.audience,
        time.into(),
        body.conference_room_id,
        body.event_room_id,
    );

    let query = if let Some(tags) = tags {
        query.tags(tags)
    } else {
        query
    };

    let query = if let Some(id) = body.original_event_room_id {
        query.original_event_room_id(id)
    } else {
        query
    };

    let query = if let Some(id) = body.modified_event_room_id {
        query.modified_event_room_id(id)
    } else {
        query
    };

    let webinar = {
        let mut conn = req
            .state()
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;

        let mut txn = conn
            .begin()
            .await
            .context("Failed to acquire transaction")
            .error(AppErrorKind::DbQueryFailed)?;
        let webinar = query
            .execute(&mut txn)
            .await
            .context("Failed to find recording")
            .error(AppErrorKind::DbQueryFailed)?;

        if let Some(recording) = body.recording {
            crate::db::recording::RecordingConvertInsertQuery::new(
                webinar.id(),
                recording.stream_id,
                recording.segments,
                recording.modified_segments,
                recording.uri,
            )
            .execute(&mut txn)
            .await
            .context("Failed to insert recording")
            .error(AppErrorKind::DbQueryFailed)?;
        }
        txn.commit()
            .await
            .context("Convert transaction failed")
            .error(AppErrorKind::DbQueryFailed)?;
        webinar
    };

    let body = serde_json::to_string(&webinar)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

async fn find_webinar(
    req: &Request<Arc<dyn AppContext>>,
) -> anyhow::Result<crate::db::class::Object> {
    let id = extract_id(req)?;

    let webinar = {
        let mut conn = req.state().get_conn().await?;
        crate::db::class::WebinarReadQuery::by_id(id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find webinar"))?
    };
    Ok(webinar)
}

async fn find_webinar_by_scope(
    req: &Request<Arc<dyn AppContext>>,
) -> anyhow::Result<crate::db::class::Object> {
    let audience = extract_param(req, "audience")?.to_owned();
    let scope = extract_param(req, "scope")?.to_owned();

    let webinar = {
        let mut conn = req.state().get_conn().await?;
        crate::db::class::WebinarReadQuery::by_scope(audience.clone(), scope.clone())
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find webinar by scope"))?
    };
    Ok(webinar)
}

fn extract_bound(t: Bound<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match t {
        Bound::Included(t) => Some(t),
        Bound::Excluded(t) => Some(t),
        Bound::Unbounded => None,
    }
}

fn dt_options_cmp_max(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(std::cmp::max(a, b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn dt_options_cmp_min(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(std::cmp::min(a, b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn times_overlap(t1: BoundedDateTimeTuple, t2: BoundedDateTimeTuple) -> BoundedDateTimeTuple {
    let st1 = extract_bound(t1.0);
    let end1 = extract_bound(t1.1);
    let st2 = extract_bound(t2.0);
    let end2 = extract_bound(t2.1);

    let st = dt_options_cmp_max(st1, st2);
    let st = st.map(Bound::Included).unwrap_or(Bound::Unbounded);
    let end = dt_options_cmp_min(end1, end2);
    let end = end.map(Bound::Excluded).unwrap_or(Bound::Unbounded);

    (st, end)
}

pub use download::download;
pub use recreate::recreate;

mod download;
mod recreate;
