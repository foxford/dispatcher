use std::sync::Arc;

use anyhow::Context;
use chrono::Utc;
use serde_derive::Serialize;
use serde_json::Value as JsonValue;
use svc_authn::AccountId;
use tide::{Request, Response};
use uuid::Uuid;

use super::*;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::{AsClassType, Object as Class};

#[derive(Serialize)]
struct ClassResponseBody {
    id: String,
    real_time: RealTimeObject,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    on_demand: Vec<ClassroomVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<ClassStatus>,
    timeouted: bool,
}

impl ClassResponseBody {
    pub fn add_version(&mut self, version: ClassroomVersion) {
        self.on_demand.push(version);
    }

    pub fn set_status(&mut self, status: ClassStatus) {
        self.status = Some(status);
    }

    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.real_time.set_rtc_id(rtc_id);
    }
}

impl From<&Class> for ClassResponseBody {
    fn from(obj: &Class) -> Self {
        Self {
            id: obj.scope().to_owned(),
            real_time: RealTimeObject {
                conference_room_id: obj.conference_room_id(),
                event_room_id: obj.event_room_id(),
                rtc_id: None,
                fallback_uri: None,
            },
            on_demand: vec![],
            status: None,
            timeouted: obj.timeouted(),
        }
    }
}

#[derive(Serialize)]
pub struct ClassroomVersion {
    version: &'static str,
    event_room_id: Uuid,
    // TODO: this is deprecated and should be removed eventually
    // right now its necessary to generate HLS links
    stream_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    room_events_uri: Option<String>,
}

#[derive(Serialize)]
pub struct RealTimeObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    conference_room_id: Option<Uuid>,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtc_id: Option<Uuid>,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum ClassStatus {
    Transcoded,
    Adjusted,
    Finished,
    RealTime,
    Closed,
}

impl RealTimeObject {
    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.rtc_id = Some(rtc_id);
    }
}

pub async fn read<T: AsClassType>(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();
    let id = extract_id(&req).error(AppErrorKind::InvalidParameter)?;

    do_read::<T>(state.as_ref(), &account_id, id).await
}

async fn do_read<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    id: Uuid,
) -> AppResult {
    let class = find::<T>(state, id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    do_read_inner::<T>(state, account_id, class).await
}

pub async fn read_by_scope<T: AsClassType>(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let audience = extract_param(&req, "audience").error(AppErrorKind::InvalidParameter)?;
    let scope = extract_param(&req, "scope").error(AppErrorKind::InvalidParameter)?;
    let state = req.state();

    do_read_by_scope::<T>(state.as_ref(), &account_id, audience, scope).await
}

async fn do_read_by_scope<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    audience: &str,
    scope: &str,
) -> AppResult {
    let class = match find_by_scope::<T>(state, audience, scope).await {
        Ok(class) => class,
        Err(e) => {
            error!(crate::LOG, "Failed to find a minigroup, err = {:?}", e);
            return Ok(tide::Response::builder(404).body("Not found").build());
        }
    };

    do_read_inner::<T>(state, account_id, class).await
}

async fn do_read_inner<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    class: Class,
) -> AppResult {
    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();
    state
        .authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await
        .measure()?;
    let recordings = {
        let mut conn = state
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;
        crate::db::recording::RecordingListQuery::new(class.id())
            .execute(&mut conn)
            .await
            .context("Failed to find recording")
            .error(AppErrorKind::DbQueryFailed)?
    };
    let mut class_body: ClassResponseBody = (&class).into();

    let class_end = class.time().end();
    if let Some(recording) = recordings.first() {
        // BEWARE: the order is significant
        // as of now its expected that modified version is second
        if let Some(og_event_id) = class.original_event_room_id() {
            class_body.add_version(ClassroomVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: class.tags().map(ToOwned::to_owned),
                room_events_uri: None,
            });
        }

        class_body.set_rtc_id(recording.rtc_id());

        if recording.transcoded_at().is_some() {
            if let Some(md_event_id) = class.modified_event_room_id() {
                class_body.add_version(ClassroomVersion {
                    version: "modified",
                    stream_id: recording.rtc_id(),
                    event_room_id: md_event_id,
                    tags: class.tags().map(ToOwned::to_owned),
                    room_events_uri: class.room_events_uri().cloned(),
                });
            }

            class_body.set_status(ClassStatus::Transcoded);
        } else if recording.adjusted_at().is_some() {
            class_body.set_status(ClassStatus::Adjusted);
        } else {
            class_body.set_status(ClassStatus::Finished);
        }
    } else if class_end.map(|t| Utc::now() > *t).unwrap_or(false) {
        class_body.set_status(ClassStatus::Closed);
    } else {
        class_body.set_status(ClassStatus::RealTime);
    }

    let body = serde_json::to_string(&class_body)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;
    let response = Response::builder(200).body(body).build();
    Ok(response)
}
