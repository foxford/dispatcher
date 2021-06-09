use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::Deserialize;
use svc_authn::AccountId;
use tide::{Request, Response};
use uuid::Uuid;

use super::{extract_id, find, validate_token, AppResult};
use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::clients::{
    conference::RoomUpdate as ConfRoomUpdate, event::RoomUpdate as EventRoomUpdate,
};
use crate::db::class::{AsClassType, BoundedDateTimeTuple};

#[derive(Deserialize)]
struct ClassUpdate {
    #[serde(with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    reserve: Option<i32>,
}

pub async fn update<T: AsClassType>(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: ClassUpdate = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();
    let id = extract_id(&req).error(AppErrorKind::InvalidParameter)?;

    do_update::<T>(state.as_ref(), &account_id, id, body).await
}

async fn do_update<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    id: Uuid,
    body: ClassUpdate,
) -> AppResult {
    let class = find::<T>(state, id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();

    state
        .authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await?;

    if let Some(time) = &body.time {
        let conference_time = match time.0 {
            Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), time.1),
            Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
        };
        let conference_fut = state.conference_client().update_room(
            class.conference_room_id(),
            ConfRoomUpdate {
                time: Some(conference_time),
                reserve: body.reserve,
                classroom_id: None,
            },
        );

        let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
        let event_fut = state.event_client().update_room(
            class.event_room_id(),
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
    }

    let mut query = crate::db::class::TimeUpdateQuery::new(class.id());
    if let Some(t) = body.time {
        query = query.time(t.into());
    }

    if let Some(r) = body.reserve {
        query = query.reserve(r);
    }

    let mut conn = state.get_conn().await.error(AppErrorKind::DbQueryFailed)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::class::{WebinarReadQuery, WebinarType},
        test_helpers::prelude::*,
    };
    use chrono::Duration;
    use mockall::predicate as pred;
    use uuid::Uuid;

    #[async_std::test]
    async fn update_webinar_unauthorized() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let state = TestState::new(TestAuthz::new()).await;
        let webinar = {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                event_room_id,
                conference_room_id,
            )
            .insert(&mut conn)
            .await
        };

        let state = Arc::new(state);
        let body = ClassUpdate {
            time: Some((
                Bound::Included(Utc::now() + Duration::hours(2)),
                Bound::Unbounded,
            )),
            reserve: None,
        };

        do_update::<WebinarType>(state.as_ref(), agent.account_id(), webinar.id(), body)
            .await
            .expect_err("Unexpectedly succeeded");
    }

    #[async_std::test]
    async fn update_webinar() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let db_pool = TestDb::new().await;

        let webinar = {
            let mut conn = db_pool.get_conn().await;
            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                conference_room_id,
                event_room_id,
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

        update_webinar_mocks(&mut state, event_room_id, conference_room_id);

        let state = Arc::new(state);
        let body = ClassUpdate {
            time: Some((
                Bound::Included(Utc::now() + Duration::hours(2)),
                Bound::Unbounded,
            )),
            reserve: None,
        };

        do_update::<WebinarType>(state.as_ref(), agent.account_id(), webinar.id(), body)
            .await
            .expect("Failed to update");

        let mut conn = state.get_conn().await.expect("Failed to get conn");
        let updated_webinar = WebinarReadQuery::by_id(webinar.id())
            .execute(&mut conn)
            .await
            .expect("Failed to fetch webinar")
            .expect("Webinar not found");

        let time: BoundedDateTimeTuple = updated_webinar.time().to_owned().into();
        assert!(matches!(time.0, Bound::Included(_)));
    }

    fn update_webinar_mocks(state: &mut TestState, event_room_id: Uuid, conference_room_id: Uuid) {
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
