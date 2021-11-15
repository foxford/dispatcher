use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, TypedHeader};
use chrono::Utc;
use headers::{authorization::Bearer, Authorization};
use hyper::{Body, Response};
use serde_derive::Deserialize;
use svc_agent::AccountId;
use tracing::error;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::metrics::AuthorizeMetrics;
use crate::app::AppContext;
use crate::db::class::{self, BoundedDateTimeTuple};

use super::{validate_token, AppResult};

#[derive(Deserialize)]
pub struct MinigroupCreatePayload {
    scope: String,
    audience: String,
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    tags: Option<serde_json::Value>,
    reserve: Option<i32>,
    #[serde(default = "class::default_locked_chat")]
    locked_chat: bool,
}

pub async fn create(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(body): Json<MinigroupCreatePayload>,
) -> AppResult {
    let account_id =
        validate_token(ctx.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;
    do_create(ctx.as_ref(), &account_id, body).await
}

async fn do_create(
    state: &dyn AppContext,
    account_id: &AccountId,
    body: MinigroupCreatePayload,
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
        .await
        .measure()?;

    let conference_time = match body.time.map(|t| t.0) {
        Some(Bound::Included(t)) | Some(Bound::Excluded(t)) => {
            (Bound::Included(t), Bound::Unbounded)
        }
        Some(Bound::Unbounded) | None => (Bound::Included(Utc::now()), Bound::Unbounded),
    };
    let conference_fut = state.conference_client().create_room(
        conference_time,
        body.audience.clone(),
        Some("owned".into()),
        body.reserve,
        body.tags.clone(),
        None,
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = state.event_client().create_room(
        event_time,
        body.audience.clone(),
        Some(true),
        body.tags.clone(),
        None,
    );

    let (event_room_id, conference_room_id) = tokio::try_join!(event_fut, conference_fut)
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    let query = crate::db::class::MinigroupInsertQuery::new(
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
    let minigroup = {
        let mut conn = state
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;
        query
            .execute(&mut conn)
            .await
            .context("Failed to insert minigroup")
            .error(AppErrorKind::DbQueryFailed)?
    };
    if body.locked_chat {
        if let Err(e) = state.event_client().lock_chat(event_room_id).await {
            error!(
                %event_room_id,
                "Failed to lock chat in event room, err = {:?}",
                e
            );
        }
    }

    crate::app::services::update_classroom_id(
        state,
        minigroup.id(),
        minigroup.event_room_id(),
        minigroup.conference_room_id(),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string_pretty(&minigroup)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

#[cfg(test)]
mod tests {
    mod create {
        use super::super::*;
        use crate::{db::class::MinigroupReadQuery, test_helpers::prelude::*};
        use chrono::Duration;
        use mockall::predicate as pred;
        use uuid::Uuid;

        #[tokio::test]
        async fn create_minigroup_no_time() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let mut authz = TestAuthz::new();
            authz.allow(agent.account_id(), vec!["classrooms"], "create");

            let mut state = TestState::new(authz).await;
            let event_room_id = Uuid::new_v4();
            let conference_room_id = Uuid::new_v4();

            create_minigroup_mocks(&mut state, event_room_id, conference_room_id);

            let scope = random_string();

            let state = Arc::new(state);
            let body = MinigroupCreatePayload {
                scope: scope.clone(),
                audience: USR_AUDIENCE.to_string(),
                time: None,
                tags: None,
                reserve: Some(10),
                locked_chat: true,
            };

            let r = do_create(state.as_ref(), agent.account_id(), body).await;
            r.expect("Failed to create minigroup");

            // Assert DB changes.
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let new_minigroup = MinigroupReadQuery::by_scope(USR_AUDIENCE, &scope)
                .execute(&mut conn)
                .await
                .expect("Failed to fetch minigroup")
                .expect("Mebinar not found");

            assert_eq!(new_minigroup.reserve(), Some(10),);
        }

        #[tokio::test]
        async fn create_minigroup_with_time() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let mut authz = TestAuthz::new();
            authz.allow(agent.account_id(), vec!["classrooms"], "create");

            let mut state = TestState::new(authz).await;
            let event_room_id = Uuid::new_v4();
            let conference_room_id = Uuid::new_v4();

            create_minigroup_mocks(&mut state, event_room_id, conference_room_id);

            let scope = random_string();

            let now = Utc::now();
            let time = (
                Bound::Included(now + Duration::hours(1)),
                Bound::Excluded(now + Duration::hours(5)),
            );

            let state = Arc::new(state);
            let body = MinigroupCreatePayload {
                scope: scope.clone(),
                audience: USR_AUDIENCE.to_string(),
                time: Some(time),
                tags: None,
                reserve: Some(10),
                locked_chat: true,
            };

            let r = do_create(state.as_ref(), agent.account_id(), body).await;
            r.expect("Failed to create minigroup");

            // Assert DB changes.
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let new_minigroup = MinigroupReadQuery::by_scope(USR_AUDIENCE, &scope)
                .execute(&mut conn)
                .await
                .expect("Failed to fetch minigroup")
                .expect("Minigroup not found");

            assert_eq!(new_minigroup.reserve(), Some(10),);
        }

        #[tokio::test]
        async fn create_minigroup_unauthorized() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let state = TestState::new(TestAuthz::new()).await;

            let scope = random_string();

            let state = Arc::new(state);
            let body = MinigroupCreatePayload {
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

        fn create_minigroup_mocks(
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
                    pred::always(),
                )
                .returning(move |_, _, _, _, _| Ok(event_room_id));

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
                .withf(move |_time, _audience, policy, reserve, _tags, _cid| {
                    assert_eq!(*policy, Some(String::from("owned")));
                    assert_eq!(*reserve, Some(10));
                    true
                })
                .returning(move |_, _, _, _, _, _| Ok(conference_room_id));

            state
                .conference_client_mock()
                .expect_update_room()
                .with(pred::eq(conference_room_id), pred::always())
                .returning(move |_room_id, _| Ok(()));
        }
    }
}

mod download;

pub use download::download;
