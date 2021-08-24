use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::{DateTime, Utc};
use serde_derive::Deserialize;
use sqlx::Acquire;
use svc_agent::AccountId;
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::clients::{conference::ConferenceRoomResponse, event::EventRoomResponse};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::recording::Segments;

use super::{validate_token, AppResult};

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
    #[serde(deserialize_with = "crate::db::recording::serde::segments::deserialize")]
    segments: Segments,
    #[serde(deserialize_with = "crate::db::recording::serde::segments::deserialize")]
    modified_segments: Segments,
    uri: String,
}

pub async fn convert(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let body = req.body_json().await.error(AppErrorKind::InvalidPayload)?;
    let state = req.state();

    do_convert(state.as_ref(), &account_id, body).await
}
async fn do_convert(
    state: &dyn AppContext,
    account_id: &AccountId,
    body: WebinarConvertObject,
) -> AppResult {
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

    let (time, tags) = match (body.time, body.tags) {
        // if we have both time and tags - lets use them
        (Some(t), Some(tags)) => (t, Some(tags)),
        // otherwise we try to fetch room time from conference and event
        _ => {
            let conference_fut = state.conference_client().read_room(body.conference_room_id);
            let event_fut = state.event_client().read_room(body.event_room_id);
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
        let mut conn = state
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
                svc_agent::AgentId::new("portal", account_id.to_owned()),
            )
            .execute(&mut txn)
            .await
            .context("Failed to insert recording")
            .error(AppErrorKind::DbQueryFailed)?;
        }

        let event_id = webinar
            .modified_event_room_id()
            .unwrap_or_else(|| webinar.event_room_id());
        crate::app::services::update_classroom_id(
            state,
            webinar.id(),
            event_id,
            webinar.conference_room_id(),
        )
        .await
        .error(AppErrorKind::MqttRequestFailed)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::prelude::*;
    use mockall::predicate as pred;
    use serde_json::{json, Value};

    #[async_std::test]
    async fn convert_webinar_unauthorized() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let state = TestState::new(TestAuthz::new()).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let scope = random_string();

        let state = Arc::new(state);
        let body = WebinarConvertObject {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            time: None,
            tags: None,
            event_room_id,
            conference_room_id,
            original_event_room_id: None,
            modified_event_room_id: None,
            recording: None,
        };

        do_convert(state.as_ref(), agent.account_id(), body)
            .await
            .expect_err("Unexpectedly succeeded");
    }

    #[async_std::test]
    async fn convert_webinar() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let mut authz = TestAuthz::new();
        authz.allow(agent.account_id(), vec!["classrooms"], "convert");

        let mut state = TestState::new(authz).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();
        convert_webinar_mocks(&mut state, event_room_id, conference_room_id);

        let scope = random_string();

        let state = Arc::new(state);
        let body = WebinarConvertObject {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            time: Some((Bound::Unbounded, Bound::Unbounded)),
            tags: Some(json!({"scope": "whatever"})),
            event_room_id,
            conference_room_id,
            original_event_room_id: None,
            modified_event_room_id: None,
            recording: None,
        };

        let mut r = do_convert(state.as_ref(), agent.account_id(), body)
            .await
            .expect("Failed to convert webinar");

        let r = r
            .take_body()
            .into_string()
            .await
            .expect("Failed to get body");
        let v = serde_json::from_str::<Value>(&r).expect("Failed to parse json");
        assert_eq!(
            v.get("event_room_id").and_then(|v| v.as_str()),
            Some(event_room_id.to_string()).as_deref()
        );
        assert_eq!(
            v.get("conference_room_id").and_then(|v| v.as_str()),
            Some(conference_room_id.to_string()).as_deref()
        );
    }

    #[async_std::test]
    async fn convert_webinar_with_recording() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let mut authz = TestAuthz::new();
        authz.allow(agent.account_id(), vec!["classrooms"], "convert");

        let mut state = TestState::new(authz).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();
        convert_webinar_mocks(&mut state, event_room_id, conference_room_id);

        let scope = random_string();

        let state = Arc::new(state);
        let body = WebinarConvertObject {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            time: Some((Bound::Unbounded, Bound::Unbounded)),
            tags: Some(json!({"scope": "whatever"})),
            event_room_id,
            conference_room_id,
            original_event_room_id: None,
            modified_event_room_id: None,
            recording: Some(RecordingConvertObject {
                stream_id: Uuid::new_v4(),
                uri: "s3://some.bucket/foo.mp4".to_string(),
                segments: Segments::from(vec![(Bound::Included(0), Bound::Excluded(10000))]),
                modified_segments: Segments::from(vec![(
                    Bound::Included(0),
                    Bound::Excluded(10000),
                )]),
            }),
        };

        let mut r = do_convert(state.as_ref(), agent.account_id(), body)
            .await
            .expect("Failed to convert webinar");

        let r = r
            .take_body()
            .into_string()
            .await
            .expect("Failed to get body");
        let v = serde_json::from_str::<Value>(&r).expect("Failed to parse json");
        assert_eq!(
            v.get("event_room_id").and_then(|v| v.as_str()),
            Some(event_room_id.to_string()).as_deref()
        );
        assert_eq!(
            v.get("conference_room_id").and_then(|v| v.as_str()),
            Some(conference_room_id.to_string()).as_deref()
        );
    }

    #[async_std::test]
    async fn convert_webinar_unspecified_time() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let mut authz = TestAuthz::new();
        authz.allow(agent.account_id(), vec!["classrooms"], "convert");

        let mut state = TestState::new(authz).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();
        convert_unspecified_time_webinar_mocks(&mut state, event_room_id, conference_room_id);

        let scope = random_string();

        let state = Arc::new(state);
        let body = WebinarConvertObject {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            time: None,
            tags: None,
            event_room_id,
            conference_room_id,
            original_event_room_id: None,
            modified_event_room_id: None,
            recording: None,
        };

        let mut r = do_convert(state.as_ref(), agent.account_id(), body)
            .await
            .expect("Failed to convert webinar");

        let r = r
            .take_body()
            .into_string()
            .await
            .expect("Failed to get body");
        let v = serde_json::from_str::<Value>(&r).expect("Failed to parse json");
        assert_eq!(
            v.get("event_room_id").and_then(|v| v.as_str()),
            Some(event_room_id.to_string()).as_deref()
        );
        assert_eq!(
            v.get("conference_room_id").and_then(|v| v.as_str()),
            Some(conference_room_id.to_string()).as_deref()
        );
    }

    fn convert_unspecified_time_webinar_mocks(
        state: &mut TestState,
        event_room_id: Uuid,
        conference_room_id: Uuid,
    ) {
        convert_webinar_mocks(state, event_room_id, conference_room_id);

        state
            .conference_client_mock()
            .expect_read_room()
            .with(pred::eq(conference_room_id))
            .returning(move |id| {
                Ok(ConferenceRoomResponse {
                    id,
                    time: (Bound::Included(Utc::now()), Bound::Unbounded),
                })
            });

        state
            .event_client_mock()
            .expect_read_room()
            .with(pred::eq(event_room_id))
            .returning(move |id| {
                Ok(EventRoomResponse {
                    id,
                    time: (Bound::Unbounded, Bound::Unbounded),
                    tags: Some(json!({"scope": "foobar"})),
                })
            });
    }

    fn convert_webinar_mocks(state: &mut TestState, event_room_id: Uuid, conference_room_id: Uuid) {
        state
            .event_client_mock()
            .expect_update_room()
            .with(pred::eq(event_room_id), pred::always())
            .returning(move |_room_id, _| Ok(()));

        state
            .conference_client_mock()
            .expect_update_room()
            .with(pred::eq(conference_room_id), pred::always())
            .returning(move |_room_id, _| Ok(()));
    }
}
