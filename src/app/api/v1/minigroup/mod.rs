use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::Extension;
use axum::extract::Path;
use hyper::{Body, Response};
use serde_derive::Deserialize;
use svc_agent::AccountId;
use svc_utils::extractors::AccountIdExtractor;
use tracing::{error, info, instrument};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::http::Json;
use crate::app::metrics::AuthorizeMetrics;
use crate::app::services;
use crate::app::services::lock_interaction;
use crate::app::AppContext;
use crate::clients::event::LockedTypes;
use crate::clients::tq::Priority;
use crate::db::class::ClassType;
use crate::db::class::KeyValueProperties;
use crate::db::class::{self, BoundedDateTimeTuple};

use super::AppError;
use super::AppResult;

#[derive(Deserialize)]
pub struct MinigroupCreatePayload {
    scope: String,
    audience: String,
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    tags: Option<serde_json::Value>,
    #[serde(default)]
    properties: KeyValueProperties,
    reserve: Option<i32>,
    #[serde(default = "class::default_locked_chat")]
    locked_chat: bool,
    #[serde(default = "class::default_locked_questions")]
    locked_questions: bool,
}

impl MinigroupCreatePayload {
    fn locked_types(&self) -> LockedTypes {
        LockedTypes {
            message: self.locked_chat,
            reaction: self.locked_chat,
            question: self.locked_questions,
            question_reaction: self.locked_questions,
        }
    }
}

#[instrument(
    skip_all,
    fields(
        audience = ?body.audience,
        scope = ?body.scope
    )
)]
pub async fn create(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Json(body): Json<MinigroupCreatePayload>,
) -> AppResult {
    info!("Creating minigroup");
    let r = do_create(ctx.as_ref(), &account_id, body).await;
    if let Err(e) = &r {
        error!(error = ?e, "Failed to create minigroup");
    }
    r
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

    info!("Authorized minigroup create");

    let dummy = insert_minigroup_dummy(state, &body).await?;

    let time = body.time.unwrap_or((Bound::Unbounded, Bound::Unbounded));
    let result = services::create_event_and_conference_rooms(state, &dummy, &time).await;
    let mut conn = state
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    let event_room_id = match result {
        Ok((event_id, conference_id)) => {
            info!(?event_id, ?conference_id, "Created rooms",);

            class::EstablishQuery::new(dummy.id(), event_id, conference_id)
                .execute(&mut conn)
                .await
                .context("Failed to establish webinar dummy")
                .error(AppErrorKind::DbQueryFailed)?;
            event_id
        }
        Err(e) => {
            info!("Failed to create rooms");

            class::DeleteQuery::new(dummy.id())
                .execute(&mut conn)
                .await
                .context("Failed to delete webinar dummy")
                .error(AppErrorKind::DbQueryFailed)?;
            return Err(e);
        }
    };

    let locked_types = body.locked_types();
    if locked_types.any_locked() {
        lock_interaction(state, event_room_id, locked_types).await;
    }

    let body = serde_json::to_string_pretty(&dummy)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

async fn insert_minigroup_dummy(
    state: &dyn AppContext,
    body: &MinigroupCreatePayload,
) -> Result<class::Dummy, AppError> {
    let query = class::InsertQuery::new(
        ClassType::Minigroup,
        body.scope.clone(),
        body.audience.clone(),
        body.time
            .unwrap_or((Bound::Unbounded, Bound::Unbounded))
            .into(),
    )
    .properties(body.properties.clone())
    .preserve_history(true);

    let query = if let Some(ref tags) = body.tags {
        query.tags(tags.clone())
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
    query
        .execute(&mut conn)
        .await
        .context("Failed to insert minigroup")
        .error(AppErrorKind::DbQueryFailed)?
        .ok_or_else(|| AppError::from(AppErrorKind::ClassAlreadyEstablished))
}

#[derive(Debug, Deserialize)]
pub struct RestartTranscodingPayload {
    priority: Priority,
}

impl Default for RestartTranscodingPayload {
    fn default() -> Self {
        Self {
            priority: Priority::Normal,
        }
    }
}

pub async fn restart_transcoding(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Path(id): Path<Uuid>,
    payload: Option<Json<RestartTranscodingPayload>>,
) -> AppResult {
    let mut conn = ctx.get_conn().await.error(AppErrorKind::InternalFailure)?;

    let minigroup = crate::db::class::ReadQuery::by_id(id)
        .execute(&mut conn)
        .await
        .error(AppErrorKind::InternalFailure)?
        .ok_or_else(|| AppError::from(AppErrorKind::ClassNotFound))?;

    let object = AuthzObject::new(&["classrooms", &id.to_string()]).into();
    ctx.authz()
        .authorize(
            minigroup.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let payload = match payload {
        Some(Json(payload)) => payload,
        None => RestartTranscodingPayload::default(),
    };

    let r = crate::app::postprocessing_strategy::restart_minigroup_transcoding(
        ctx,
        minigroup,
        payload.priority,
    )
    .await;

    match r {
        Ok(_) => Ok(Response::new(Body::from(""))),
        Err(err) => Err(err).error(AppErrorKind::InternalFailure),
    }
}

#[cfg(test)]
mod tests {
    mod create {
        use super::super::*;
        use crate::{db::class::MinigroupReadQuery, test_helpers::prelude::*};
        use chrono::{Duration, Utc};
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
                properties: KeyValueProperties::default(),
                reserve: Some(10),
                locked_chat: true,
                locked_questions: true,
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
                properties: KeyValueProperties::default(),
                reserve: Some(10),
                locked_chat: true,
                locked_questions: true,
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
                properties: KeyValueProperties::default(),
                reserve: Some(10),
                locked_chat: true,
                locked_questions: true,
            };

            do_create(state.as_ref(), agent.account_id(), body)
                .await
                .expect_err("Unexpectedly succeeded");
        }

        #[tokio::test]
        async fn create_minigroup_with_properties() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let mut authz = TestAuthz::new();
            authz.allow(agent.account_id(), vec!["classrooms"], "create");

            let mut state = TestState::new(authz).await;
            let event_room_id = Uuid::new_v4();
            let conference_room_id = Uuid::new_v4();

            create_minigroup_mocks(&mut state, event_room_id, conference_room_id);

            let scope = random_string();

            let mut properties: KeyValueProperties = serde_json::Map::new().into();
            properties.insert("is_adult".into(), true.into());

            let state = Arc::new(state);
            let body = MinigroupCreatePayload {
                scope: scope.clone(),
                audience: USR_AUDIENCE.to_string(),
                time: None,
                tags: None,
                properties: properties.clone(),
                reserve: Some(10),
                locked_chat: true,
                locked_questions: true,
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

            assert_eq!(new_minigroup.reserve(), Some(10));
            assert_eq!(*new_minigroup.properties(), properties);
        }

        fn create_minigroup_mocks(
            state: &mut TestState,
            event_room_id: Uuid,
            conference_room_id: Uuid,
        ) {
            state
                .event_client_mock()
                .expect_create_room()
                .with(pred::always())
                .returning(move |_| Ok(event_room_id));

            state
                .event_client_mock()
                .expect_update_locked_types()
                .with(pred::eq(event_room_id), pred::always())
                .returning(move |_room_id, _locked_types| Ok(()));

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
mod whiteboard;

pub use download::download;
pub use whiteboard::create_whiteboard;
