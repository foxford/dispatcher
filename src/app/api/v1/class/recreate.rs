use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, Path};
use chrono::Utc;
use hyper::{Body, Response};
use serde_derive::Deserialize;
use sqlx::Acquire;
use svc_agent::Authenticable;
use svc_authn::AccountId;
use svc_utils::extractors::AuthnExtractor;
use tracing::error;
use uuid::Uuid;

use super::{find, AppResult};
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class;
use crate::db::class::BoundedDateTimeTuple;
use crate::db::class::ClassType;
use crate::db::class::Object as WebinarObject;
use crate::{app::api::v1::AppError, db::class::AsClassType};

#[derive(Deserialize)]
pub struct ClassRecreatePayload {
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    #[serde(default = "class::default_locked_chat")]
    locked_chat: bool,
}

pub async fn recreate<T: AsClassType>(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    AuthnExtractor(agent_id): AuthnExtractor,
    Json(body): Json<ClassRecreatePayload>,
) -> AppResult {
    do_recreate::<T>(ctx.as_ref(), agent_id.as_account_id(), id, body).await
}

async fn do_recreate<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    id: Uuid,
    body: ClassRecreatePayload,
) -> AppResult {
    let webinar = find::<T>(state, id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();

    let time = body.time.unwrap_or((Bound::Unbounded, Bound::Unbounded));

    state
        .authz()
        .authorize(
            webinar.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let (event_room_id, conference_room_id) =
        create_event_and_conference::<T>(state, &webinar, &time).await?;

    let query = crate::db::class::RecreateQuery::new(
        webinar.id(),
        time.into(),
        event_room_id,
        conference_room_id,
    );

    let webinar = {
        let mut conn = state.get_conn().await.error(AppErrorKind::DbQueryFailed)?;
        let mut txn = conn
            .begin()
            .await
            .context("Failed to acquire transaction")
            .error(AppErrorKind::DbQueryFailed)?;

        let webinar = query
            .execute(&mut txn)
            .await
            .with_context(|| format!("Failed to update {}", T::as_str()))
            .error(AppErrorKind::DbQueryFailed)?;

        crate::db::recording::DeleteQuery::new(webinar.id())
            .execute(&mut txn)
            .await
            .context("Failed to delete recording")
            .error(AppErrorKind::DbQueryFailed)?;

        txn.commit()
            .await
            .context("Convert transaction failed")
            .error(AppErrorKind::DbQueryFailed)?;

        webinar
    };

    if body.locked_chat {
        if let Err(e) = state.event_client().lock_chat(event_room_id).await {
            error!(
                %event_room_id,
                "Failed to lock chat in event room, err = {:?}", e
            );
        }
    }

    let body = serde_json::to_string(&webinar)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder().body(Body::from(body)).unwrap();

    Ok(response)
}

async fn create_event_and_conference<T: AsClassType>(
    state: &dyn AppContext,
    webinar: &WebinarObject,
    time: &BoundedDateTimeTuple,
) -> Result<(Uuid, Uuid), AppError> {
    let conference_time = match time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), Bound::Unbounded),
        Bound::Unbounded => (Bound::Included(Utc::now()), Bound::Unbounded),
    };

    let policy = match T::as_class_type() {
        ClassType::Webinar => Some("shared".to_string()),
        ClassType::Minigroup => Some("owned".to_string()),
        ClassType::P2P => None,
    };
    let conference_fut = state.conference_client().create_room(
        conference_time,
        webinar.audience().to_owned(),
        policy,
        webinar.reserve(),
        webinar.tags().map(ToOwned::to_owned),
        Some(webinar.id()),
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = state.event_client().create_room(
        event_time,
        webinar.audience().to_owned(),
        Some(true),
        webinar.tags().map(ToOwned::to_owned),
        Some(webinar.id()),
    );

    let (event_room_id, conference_room_id) = tokio::try_join!(event_fut, conference_fut)
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    Ok((event_room_id, conference_room_id))
}

#[cfg(test)]
mod tests {
    mod recreate {
        use super::super::*;
        use crate::{
            db::class::{WebinarReadQuery, WebinarType},
            test_helpers::prelude::*,
        };
        use mockall::predicate as pred;
        use uuid::Uuid;

        #[tokio::test]
        async fn recreate_webinar_unauthorized() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let db_pool = TestDb::new().await;

            let webinar = {
                let mut conn = db_pool.get_conn().await;
                factory::Webinar::new(
                    random_string(),
                    USR_AUDIENCE.to_string(),
                    (Bound::Unbounded, Bound::Unbounded).into(),
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                )
                .insert(&mut conn)
                .await
            };

            let authz = TestAuthz::new();

            let state = TestState::new_with_pool(db_pool, authz);

            let state = Arc::new(state);
            // Default value
            let body: ClassRecreatePayload = serde_json::from_str("{}").unwrap();

            do_recreate::<WebinarType>(state.as_ref(), agent.account_id(), webinar.id(), body)
                .await
                .expect_err("Unexpected success, should fail due to authz");
        }

        #[tokio::test]
        async fn recreate_webinar() {
            let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

            let recreated_event_room_id = Uuid::new_v4();
            let recreated_conference_room_id = Uuid::new_v4();

            let db_pool = TestDb::new().await;

            let webinar = {
                let mut conn = db_pool.get_conn().await;
                factory::Webinar::new(
                    random_string(),
                    USR_AUDIENCE.to_string(),
                    (Bound::Unbounded, Bound::Unbounded).into(),
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                )
                .insert(&mut conn)
                .await
            };

            let mut authz = TestAuthz::new();
            authz.allow(
                agent.account_id(),
                vec!["classrooms", &webinar.id().to_string()],
                "update",
            );

            let mut state = TestState::new_with_pool(db_pool, authz);

            create_mocks(
                &mut state,
                webinar.id(),
                recreated_event_room_id,
                recreated_conference_room_id,
            );

            let state = Arc::new(state);
            // Default value
            let body: ClassRecreatePayload = serde_json::from_str("{}").unwrap();

            do_recreate::<WebinarType>(state.as_ref(), agent.account_id(), webinar.id(), body)
                .await
                .expect("Failed to recreate");

            // Assert DB changes.
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let new_webinar = WebinarReadQuery::by_scope(USR_AUDIENCE, &webinar.scope())
                .execute(&mut conn)
                .await
                .expect("Failed to fetch webinar")
                .expect("Webinar not found");

            assert_eq!(new_webinar.event_room_id(), recreated_event_room_id);
            assert_eq!(
                new_webinar.conference_room_id(),
                Some(recreated_conference_room_id)
            );
        }

        fn create_mocks(
            state: &mut TestState,
            classroom_id: Uuid,
            event_room_id: Uuid,
            conference_room_id: Uuid,
        ) {
            state
                .event_client_mock()
                .expect_create_room()
                .withf(move |_time, _audience, _, _tags, cid| {
                    assert_eq!(*cid, Some(classroom_id));
                    true
                })
                .returning(move |_, _, _, _, _| Ok(event_room_id));

            state
                .event_client_mock()
                .expect_lock_chat()
                .with(pred::eq(event_room_id))
                .returning(move |_room_id| Ok(()));

            state
                .conference_client_mock()
                .expect_create_room()
                .withf(move |_time, _audience, _policy, _reserve, _tags, cid| {
                    assert_eq!(*cid, Some(classroom_id));
                    true
                })
                .returning(move |_, _, _, _, _, _| Ok(conference_room_id));
        }
    }
}
