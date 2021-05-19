use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::clients::{
    conference::RoomUpdate as ConfRoomUpdate, event::RoomUpdate as EventRoomUpdate,
};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::class::Object as Class;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
}

#[derive(Serialize)]
struct RealTimeObject {
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
            id: obj.scope().to_owned(),
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

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();
    state
        .authz()
        .authorize(
            webinar.audience().to_owned(),
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

    let recordings = crate::db::recording::RecordingListQuery::new(webinar.id())
        .execute(&mut conn)
        .await
        .context("Failed to find recording")
        .error(AppErrorKind::DbQueryFailed)?;

    let mut webinar_obj: WebinarObject = webinar.clone().into();

    if let Some(recording) = recordings.first() {
        // BEWARE: the order is significant
        // as of now its expected that modified version is second
        if let Some(og_event_id) = webinar.original_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: webinar.tags().map(ToOwned::to_owned),
            });
        }

        if recording.transcoded_at().is_some() {
            if let Some(md_event_id) = webinar.modified_event_room_id() {
                webinar_obj.add_version(WebinarVersion {
                    version: "modified",
                    stream_id: recording.rtc_id(),
                    event_room_id: md_event_id,
                    tags: webinar.tags().map(ToOwned::to_owned),
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

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();

    state
        .authz()
        .authorize(
            webinar.audience().to_owned(),
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

    let recordings = crate::db::recording::RecordingListQuery::new(webinar.id())
        .execute(&mut conn)
        .await
        .context("Failed to find recording")
        .error(AppErrorKind::DbQueryFailed)?;

    let mut webinar_obj: WebinarObject = webinar.clone().into();

    if let Some(recording) = recordings.first() {
        if let Some(og_event_id) = webinar.original_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: webinar.tags().map(ToOwned::to_owned),
            });
        }

        if let Some(md_event_id) = webinar.modified_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "modified",
                stream_id: recording.rtc_id(),
                event_room_id: md_event_id,
                tags: webinar.tags().map(ToOwned::to_owned),
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

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();

    state
        .authz()
        .authorize(
            webinar.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await?;

    let conference_time = match body.time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), body.time.1),
        Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
    };
    let conference_fut = req.state().conference_client().update_room(
        webinar.conference_room_id(),
        ConfRoomUpdate {
            time: Some(conference_time),
            classroom_id: None,
        },
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = req.state().event_client().update_room(
        webinar.event_room_id(),
        EventRoomUpdate {
            time: Some(event_time),
            classroom_id: None,
        },
    );

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

pub use convert::convert;
pub use create::create;
pub use download::download;
pub use recreate::recreate;

mod convert;
mod create;
mod download;
mod recreate;
