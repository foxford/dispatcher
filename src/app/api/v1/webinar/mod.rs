use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use svc_agent::AccountId;
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
struct WebinarCreatePayload {
    scope: String,
    audience: String,
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
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
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let body = req.body_json().await.error(AppErrorKind::InvalidPayload)?;
    let state = req.state();

    do_create(state.as_ref(), &account_id, body).await
}

async fn do_create(
    state: &dyn AppContext,
    account_id: &AccountId,
    body: WebinarCreatePayload,
) -> AppResult {
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

    let conference_time = match body.time.map(|t| t.0) {
        Some(Bound::Included(t)) | Some(Bound::Excluded(t)) => {
            (Bound::Included(t), Bound::Unbounded)
        }
        Some(Bound::Unbounded) | None => (Bound::Included(Utc::now()), Bound::Unbounded),
    };
    let conference_fut = state.conference_client().create_room(
        conference_time,
        body.audience.clone(),
        Some("shared".into()),
        body.reserve,
        body.tags.clone(),
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = state.event_client().create_room(
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
        body.time
            .unwrap_or((Bound::Unbounded, Bound::Unbounded))
            .into(),
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

    let mut conn = state
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    let webinar = query
        .execute(&mut conn)
        .await
        .context("Failed to insert webinar")
        .error(AppErrorKind::DbQueryFailed)?;

    if body.locked_chat {
        if let Err(e) = state.event_client().lock_chat(event_room_id).await {
            error!(
                crate::LOG,
                "Failed to lock chat in event room, id = {:?}, err = {:?}", event_room_id, e
            );
        }
    }

    crate::app::services::update_classroom_id(
        state,
        webinar.id(),
        webinar.event_room_id(),
        Some(webinar.conference_room_id()),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

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
pub use download::download;
pub use recreate::recreate;

mod convert;
mod download;
mod recreate;

#[cfg(test)]
mod tests {
    mod create {
        use super::super::*;
        use crate::{db::class::WebinarReadQuery, test_helpers::prelude::*};
        use chrono::Duration;
        use mockall::predicate as pred;

        #[async_std::test]
        async fn create_webinar_no_time() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let mut authz = TestAuthz::new();
            authz.allow(agent.account_id(), vec!["classrooms"], "create");

            let mut state = TestState::new(authz).await;
            let event_room_id = Uuid::new_v4();
            let conference_room_id = Uuid::new_v4();

            create_webinar_mocks(&mut state, event_room_id, conference_room_id);

            let scope = random_string();

            let state = Arc::new(state);
            let body = WebinarCreatePayload {
                scope: scope.clone(),
                audience: USR_AUDIENCE.to_string(),
                time: None,
                tags: None,
                reserve: Some(10),
                locked_chat: true,
            };

            let r = do_create(state.as_ref(), agent.account_id(), body).await;
            r.expect("Failed to create webinar");

            // Assert DB changes.
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let new_webinar = WebinarReadQuery::by_scope(USR_AUDIENCE.to_string(), scope)
                .execute(&mut conn)
                .await
                .expect("Failed to fetch webinar")
                .expect("Webinar not found");

            assert_eq!(new_webinar.reserve(), Some(10),);
        }

        #[async_std::test]
        async fn create_webinar_with_time() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let mut authz = TestAuthz::new();
            authz.allow(agent.account_id(), vec!["classrooms"], "create");

            let mut state = TestState::new(authz).await;
            let event_room_id = Uuid::new_v4();
            let conference_room_id = Uuid::new_v4();

            create_webinar_mocks(&mut state, event_room_id, conference_room_id);

            let scope = random_string();

            let now = Utc::now();
            let time = (
                Bound::Included(now + Duration::hours(1)),
                Bound::Excluded(now + Duration::hours(5)),
            );

            let state = Arc::new(state);
            let body = WebinarCreatePayload {
                scope: scope.clone(),
                audience: USR_AUDIENCE.to_string(),
                time: Some(time),
                tags: None,
                reserve: Some(10),
                locked_chat: true,
            };

            let r = do_create(state.as_ref(), agent.account_id(), body).await;
            r.expect("Failed to create webinar");

            // Assert DB changes.
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let new_webinar = WebinarReadQuery::by_scope(USR_AUDIENCE.to_string(), scope)
                .execute(&mut conn)
                .await
                .expect("Failed to fetch webinar")
                .expect("Webinar not found");

            assert_eq!(new_webinar.reserve(), Some(10),);
        }

        #[async_std::test]
        async fn create_webinar_unauthorized() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let state = TestState::new(TestAuthz::new()).await;
            let event_room_id = Uuid::new_v4();
            let conference_room_id = Uuid::new_v4();

            let scope = random_string();

            let state = Arc::new(state);
            let body = WebinarCreatePayload {
                scope: scope.clone(),
                audience: USR_AUDIENCE.to_string(),
                time: None,
                tags: None,
                reserve: Some(10),
                locked_chat: true,
            };

            do_create(state.as_ref(), agent.account_id(), body)
                .await
                .expect_err("Unexpectedly succeeded");
        }

        fn create_webinar_mocks(
            state: &mut TestState,
            event_room_id: Uuid,
            conference_room_id: Uuid,
        ) {
            state
                .event_client_mock()
                .expect_create_room()
                .with(
                    pred::always(),
                    pred::always(),
                    pred::always(),
                    pred::always(),
                )
                .returning(move |_, _, _, _| Ok(event_room_id));

            state
                .event_client_mock()
                .expect_lock_chat()
                .with(pred::eq(event_room_id))
                .returning(move |_room_id| Ok(()));

            state
                .event_client_mock()
                .expect_update_room()
                .with(pred::eq(event_room_id), pred::always())
                .returning(move |_room_id, _| Ok(()));

            state
                .conference_client_mock()
                .expect_create_room()
                .withf(move |_time, audience, policy, reserve, _tags| {
                    assert_eq!(*policy, Some(String::from("shared")));
                    assert_eq!(*reserve, Some(10));
                    true
                })
                .returning(move |_, _, _, _, _| Ok(conference_room_id));

            state
                .conference_client_mock()
                .expect_update_room()
                .with(pred::eq(conference_room_id), pred::always())
                .returning(move |_room_id, _| Ok(()));
        }
    }
}
