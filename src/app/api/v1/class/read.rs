use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Path};
use chrono::Utc;
use hyper::{Body, Response};
use svc_agent::Authenticable;
use svc_authn::AccountId;
use svc_utils::extractors::AuthnExtractor;
use tracing::error;
use uuid::Uuid;

use super::*;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::{AsClassType, Object as Class};

pub async fn read<T: AsClassType>(
    ctx: Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    AuthnExtractor(agent_id): AuthnExtractor,
) -> AppResult {
    do_read::<T>(ctx.0.as_ref(), agent_id.as_account_id(), id).await
}

async fn do_read<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    id: Uuid,
) -> AppResult {
    let class = find::<T>(state, id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

    do_read_inner::<T>(state, account_id, class).await
}

pub async fn read_by_scope<T: AsClassType>(
    ctx: Extension<Arc<dyn AppContext>>,
    Path((audience, scope)): Path<(String, String)>,
    AuthnExtractor(agent_id): AuthnExtractor,
) -> AppResult {
    do_read_by_scope::<T>(ctx.0.as_ref(), agent_id.as_account_id(), &audience, &scope).await
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
            error!("Failed to find a {}, err = {:?}", T::as_str(), e);
            return Ok(Response::builder()
                .status(404)
                .body(Body::from("Not found"))
                .unwrap());
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

    if let Some(turn) = state.turn_host_selector().get(&class) {
        class_body.set_turn_host(turn);
    }

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

            {
                let mut conn = state
                    .get_conn()
                    .await
                    .error(AppErrorKind::DbConnAcquisitionFailed)?;
                let position =
                    crate::db::record_timestamp::FindQuery::new(class.id(), account_id.clone())
                        .execute(&mut conn)
                        .await
                        .context("Failed to find recording timestamp")
                        .error(AppErrorKind::DbQueryFailed)?
                        .map(|v| v.position_secs());
                if let Some(pos) = position {
                    class_body.set_position(pos);
                }
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
    let response = Response::builder().body(Body::from(body)).unwrap();
    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{
        db::class::{P2PType, WebinarType},
        test_helpers::prelude::*,
    };
    use serde_json::Value;

    #[tokio::test]
    async fn read_webinar_unauthorized() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let state = TestState::new(TestAuthz::new()).await;

        let state = Arc::new(state);

        do_read::<WebinarType>(state.as_ref(), agent.account_id(), Uuid::new_v4())
            .await
            .expect_err("Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn read_webinar() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let db_pool = TestDb::new().await;

        let webinar = {
            let mut conn = db_pool.get_conn().await;
            let webinar = factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await;

            webinar
        };

        let mut authz = TestAuthz::new();
        authz.allow(
            agent.account_id(),
            vec!["classrooms", &webinar.id().to_string()],
            "read",
        );

        let mut state = TestState::new_with_pool(db_pool, authz);
        state.set_turn_hosts(&["turn0"]);
        let state = Arc::new(state);

        let r = do_read::<WebinarType>(state.as_ref(), agent.account_id(), webinar.id())
            .await
            .expect("Failed to read webinar");

        let r = hyper::body::to_bytes(r.into_body()).await.unwrap();
        let v = serde_json::from_slice::<Value>(&r[..]).expect("Failed to parse json");
        assert_eq!(v.get("turn_host").unwrap().as_str(), Some("turn0"));
    }

    #[tokio::test]
    async fn read_webinar_without_turn_hosts() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let db_pool = TestDb::new().await;

        let webinar = {
            let mut conn = db_pool.get_conn().await;
            let webinar = factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await;

            webinar
        };

        let mut authz = TestAuthz::new();
        authz.allow(
            agent.account_id(),
            vec!["classrooms", &webinar.id().to_string()],
            "read",
        );

        let state = TestState::new_with_pool(db_pool, authz);
        let state = Arc::new(state);

        let r = do_read::<WebinarType>(state.as_ref(), agent.account_id(), webinar.id())
            .await
            .expect("Failed to read webinar");

        let r = hyper::body::to_bytes(r.into_body()).await.unwrap();
        let v = serde_json::from_slice::<Value>(&r[..]).expect("Failed to parse json");
        assert_eq!(v.get("turn_host"), None);
    }

    #[tokio::test]
    async fn read_p2p() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let db_pool = TestDb::new().await;

        let p2p = {
            let mut conn = db_pool.get_conn().await;
            let p2p = factory::P2P::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await;

            p2p
        };

        let mut authz = TestAuthz::new();
        authz.allow(
            agent.account_id(),
            vec!["classrooms", &p2p.id().to_string()],
            "read",
        );

        let mut state = TestState::new_with_pool(db_pool, authz);
        state.set_turn_hosts(&["turn0", "turn1", "turn2", "turn3"]);
        let state = Arc::new(state);

        let mut turns = vec![];
        for _ in 0..5 {
            let r = do_read::<P2PType>(state.as_ref(), agent.account_id(), p2p.id())
                .await
                .expect("Failed to read p2p");

            let r = hyper::body::to_bytes(r.into_body()).await.unwrap();
            let v = serde_json::from_slice::<Value>(&r[..]).expect("Failed to parse json");
            turns.push(v.get("turn_host").unwrap().as_str().unwrap().to_string());
        }

        assert_eq!(turns.into_iter().collect::<HashSet<_>>().len(), 1);
    }
}
