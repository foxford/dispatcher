use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Path};
use hyper::{Body, Response};
use serde_derive::Deserialize;
use sqlx::Acquire;
use svc_authn::AccountId;
use svc_utils::extractors::AccountIdExtractor;
use uuid::Uuid;

use super::{find, AppResult};
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::http::Json;
use crate::app::services::lock_interaction;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::clients::event::LockedTypes;
use crate::db::class;
use crate::db::class::AsClassType;
use crate::db::class::BoundedDateTimeTuple;

#[derive(Deserialize)]
pub struct ClassRecreatePayload {
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    #[serde(default = "class::default_locked_chat")]
    locked_chat: bool,
    #[serde(default = "class::default_locked_questions")]
    locked_questions: bool,
}

impl ClassRecreatePayload {
    fn locked_types(&self) -> LockedTypes {
        LockedTypes {
            message: self.locked_chat,
            reaction: self.locked_chat,
            question: self.locked_questions,
            question_reaction: self.locked_questions,
        }
    }
}

pub async fn recreate<T: AsClassType>(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Json(body): Json<ClassRecreatePayload>,
) -> AppResult {
    do_recreate::<T>(ctx.as_ref(), &account_id, id, body).await
}

async fn do_recreate<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    id: Uuid,
    body: ClassRecreatePayload,
) -> AppResult {
    let webinar = find::<T>(state, id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

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
        crate::app::services::create_event_and_conference_rooms(state, &webinar, &time).await?;

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

    let locked_types = body.locked_types();
    if locked_types.any_locked() {
        lock_interaction(state, event_room_id, locked_types).await;
    }

    let body = serde_json::to_string(&webinar)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder().body(Body::from(body)).unwrap();

    Ok(response)
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
                recreated_conference_room_id
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
                .expect_update_locked_types()
                .with(
                    pred::eq(event_room_id),
                    pred::eq(LockedTypes::default().chat()),
                )
                .returning(move |_room_id, _locked_types| Ok(()));

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
