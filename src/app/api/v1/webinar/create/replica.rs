use crate::app::{
    api::v1::{AppError, AppResult},
    authz::AuthzObject,
    error::ErrorExt,
    error::ErrorKind as AppErrorKind,
    metrics::AuthorizeMetrics,
    services, AppContext,
};
use crate::db::class::{self, BoundedDateTimeTuple, ClassType};

use std::sync::Arc;

use crate::app::api::v1::find_class;
use crate::db::class::Object;
use anyhow::Context;
use axum::extract::{Extension, Json, Path};
use hyper::{Body, Response};
use serde_derive::Deserialize;
use svc_agent::{AccountId, Authenticable};
use svc_utils::extractors::AuthnExtractor;
use tracing::{error, info, instrument};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ReplicaCreatePayload {
    scope: String,
    audience: String,
}

#[instrument(
    skip_all,
    fields(
        audience = ?payload.audience,
        scope = ?payload.scope
    )
)]
pub async fn create(
    ctx: Extension<Arc<dyn AppContext>>,
    AuthnExtractor(agent_id): AuthnExtractor,
    Path(class_id): Path<Uuid>,
    Json(payload): Json<ReplicaCreatePayload>,
) -> AppResult {
    info!("Creating a replica of webinar");
    let r = do_create(ctx.as_ref(), agent_id.as_account_id(), class_id, payload).await;
    if let Err(e) = &r {
        error!(error = ?e, "Failed to create a replica of webinar");
    }
    r
}

async fn do_create(
    state: &dyn AppContext,
    account_id: &AccountId,
    class_id: Uuid,
    body: ReplicaCreatePayload,
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

    info!("Creation of a replica of webinar is authorized");

    let original_class = find_class(state, class_id)
        .await
        .error(AppErrorKind::ClassNotFound)?;
    let replica_class = insert_replica_dummy(state, body.scope, &original_class).await?;

    let time: BoundedDateTimeTuple = replica_class.time().into();
    let result = services::create_conference_room(state, &replica_class, &time).await;
    let mut conn = state
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    match result {
        Ok(conference_room_id) => {
            info!(?conference_room_id, "Conference room created");

            class::EstablishQuery::new(
                replica_class.id(),
                original_class.event_room_id(),
                conference_room_id,
                original_class.id(),
            )
            .execute(&mut conn)
            .await
            .context("Failed to establish webinar dummy")
            .error(AppErrorKind::DbQueryFailed)?;
        }
        Err(e) => {
            info!("Failed to create conference room");

            class::DeleteQuery::new(replica_class.id())
                .execute(&mut conn)
                .await
                .context("Failed to delete a replica of webinar (dummy)")
                .error(AppErrorKind::DbQueryFailed)?;
            return Err(e);
        }
    };

    let body = serde_json::to_string_pretty(&replica_class)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

async fn insert_replica_dummy(
    state: &dyn AppContext,
    scope: String,
    original_class: &Object,
) -> Result<class::Dummy, AppError> {
    let mut query = class::InsertQuery::new(
        ClassType::Webinar,
        scope.clone(),
        original_class.audience().to_string(),
        original_class.time().clone(),
    )
    .original_class_id(original_class.id())
    .properties(original_class.properties().clone())
    .preserve_history(true);

    if let Some(tags) = original_class.tags() {
        query = query.tags(tags.clone())
    }

    if let Some(reserve) = original_class.reserve() {
        query = query.reserve(reserve)
    }

    let mut conn = state
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    query
        .execute(&mut conn)
        .await
        .context("Failed to insert a replica of webinar")
        .error(AppErrorKind::DbQueryFailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::class::WebinarReadQuery;
    use crate::test_helpers::prelude::*;
    use mockall::predicate as pred;
    use uuid::Uuid;

    #[tokio::test]
    async fn create_replica_unauthorized() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let state = TestState::new(TestAuthz::new()).await;
        let scope = random_string();
        let class_id = Uuid::new_v4();

        let state = Arc::new(state);
        let body = ReplicaCreatePayload {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
        };

        do_create(state.as_ref(), agent.account_id(), class_id, body)
            .await
            .expect_err("Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn create_replica_original_webinar_not_found() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let mut authz = TestAuthz::new();
        authz.allow(agent.account_id(), vec!["classrooms"], "create");

        let mut state = TestState::new(authz).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();
        let class_id = Uuid::new_v4();

        create_webinar_mocks(&mut state, event_room_id, conference_room_id);

        let scope = random_string();

        let state = Arc::new(state);
        let body = ReplicaCreatePayload {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
        };

        do_create(state.as_ref(), agent.account_id(), class_id, body)
            .await
            .expect_err("Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn create_replica_from_original_webinar() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let mut authz = TestAuthz::new();
        authz.allow(agent.account_id(), vec!["classrooms"], "create");

        let mut state = TestState::new(authz).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let original_webinar = {
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                event_room_id,
            )
            .insert(&mut conn)
            .await
        };

        create_webinar_mocks(&mut state, event_room_id, conference_room_id);

        let scope = random_string();

        let state = Arc::new(state);
        let body = ReplicaCreatePayload {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
        };

        do_create(
            state.as_ref(),
            agent.account_id(),
            original_webinar.id(),
            body,
        )
        .await
        .expect("Failed to create a replica of webinar");

        // Assert DB changes.
        let mut conn = state.get_conn().await.expect("Failed to get conn");

        let replica_webinar = WebinarReadQuery::by_scope(USR_AUDIENCE, &scope)
            .execute(&mut conn)
            .await
            .expect("Failed to fetch a replica of webinar")
            .expect("Webinar not found");

        assert_eq!(replica_webinar.conference_room_id(), conference_room_id);
        assert_eq!(
            replica_webinar.event_room_id(),
            original_webinar.event_room_id()
        );
        assert_eq!(
            replica_webinar
                .original_class_id()
                .expect("Original class ID must exist"),
            original_webinar.id()
        );
    }

    fn create_webinar_mocks(state: &mut TestState, event_room_id: Uuid, conference_room_id: Uuid) {
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
            .withf(move |_time, _audience, policy, _reserve, _tags, _cid| {
                assert_eq!(*policy, Some(String::from("shared")));
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
