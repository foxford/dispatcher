use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::Serialize;
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::class::BoundedDateTimeTuple;
use crate::db::class::Object as Class;
use crate::db::class::WebinarType;

use super::{
    extract_id, extract_param, find, find_by_scope, validate_token, AppResult, ClassroomVersion,
    RealTimeObject,
};

#[derive(Serialize)]
struct WebinarObject {
    id: String,
    real_time: RealTimeObject,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    on_demand: Vec<ClassroomVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

impl WebinarObject {
    pub fn add_version(&mut self, version: ClassroomVersion) {
        self.on_demand.push(version);
    }

    pub fn set_status(&mut self, status: &str) {
        self.status = Some(status.to_owned());
    }

    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.real_time.set_rtc_id(rtc_id);
    }
}

impl From<&Class> for WebinarObject {
    fn from(obj: &Class) -> WebinarObject {
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

pub async fn read(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();
    let id = extract_id(&req).error(AppErrorKind::InvalidParameter)?;

    let webinar = find::<WebinarType>(state.as_ref(), id)
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

    let mut webinar_obj: WebinarObject = (&webinar).into();

    if let Some(recording) = recordings.first() {
        // BEWARE: the order is significant
        // as of now its expected that modified version is second
        if let Some(og_event_id) = webinar.original_event_room_id() {
            webinar_obj.add_version(ClassroomVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: webinar.tags().map(ToOwned::to_owned),
                room_events_uri: None,
            });
        }

        if recording.transcoded_at().is_some() {
            if let Some(md_event_id) = webinar.modified_event_room_id() {
                webinar_obj.add_version(ClassroomVersion {
                    version: "modified",
                    stream_id: recording.rtc_id(),
                    event_room_id: md_event_id,
                    tags: webinar.tags().map(ToOwned::to_owned),
                    room_events_uri: webinar.room_events_uri().cloned(),
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

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let state = req.state();
    let audience = extract_param(&req, "audience").error(AppErrorKind::InvalidParameter)?;
    let scope = extract_param(&req, "scope").error(AppErrorKind::InvalidParameter)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;

    let webinar = match find_by_scope::<WebinarType>(state.as_ref(), audience, scope).await {
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

    let mut webinar_obj: WebinarObject = (&webinar).into();

    if let Some(recording) = recordings.first() {
        if let Some(og_event_id) = webinar.original_event_room_id() {
            webinar_obj.add_version(ClassroomVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: webinar.tags().map(ToOwned::to_owned),
                room_events_uri: None,
            });
        }

        if let Some(md_event_id) = webinar.modified_event_room_id() {
            webinar_obj.add_version(ClassroomVersion {
                version: "modified",
                stream_id: recording.rtc_id(),
                event_room_id: md_event_id,
                tags: webinar.tags().map(ToOwned::to_owned),
                room_events_uri: webinar.room_events_uri().cloned(),
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

pub use convert::convert;
pub use create::create;
pub use download::download;
pub use recreate::recreate;
pub use update::update;

mod convert;
mod create;
mod download;
mod recreate;
mod update;
