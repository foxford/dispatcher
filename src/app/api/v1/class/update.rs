use std::ops::{Bound, Not};
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::Deserialize;
use svc_agent::AgentId;
use svc_authn::AccountId;
use tide::{Request, Response};
use uuid::Uuid;

use super::{extract_id, find, validate_token, AppResult, ClassResponseBody};
use crate::app::{api::v1::extract_param, error::ErrorKind as AppErrorKind};
use crate::app::{api::v1::find_by_scope, error::ErrorExt};
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::app::{error, AppContext};
use crate::clients::{
    conference::RoomUpdate as ConfRoomUpdate, event::RoomUpdate as EventRoomUpdate,
};
use crate::db::class;
use crate::db::class::{AsClassType, BoundedDateTimeTuple};

#[derive(Deserialize)]
struct ClassUpdate {
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    reserve: Option<i32>,
    host: Option<AgentId>,
}

pub async fn update<T: AsClassType>(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: ClassUpdate = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();
    let id = extract_id(&req).error(AppErrorKind::InvalidParameter)?;
    let class = find::<T>(state.as_ref(), id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;
    let updated_class = do_update::<T>(state.as_ref(), &account_id, class, body).await?;
    Ok(Response::builder(200)
        .body(
            serde_json::to_string(&updated_class)
                .context("Failed to serialize minigroup")
                .error(AppErrorKind::SerializationFailed)?,
        )
        .build())
}

pub async fn update_by_scope<T: AsClassType>(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: ClassUpdate = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let audience = extract_param(&req, "audience").error(AppErrorKind::InvalidParameter)?;
    let scope = extract_param(&req, "scope").error(AppErrorKind::InvalidParameter)?;
    let state = req.state();
    let class = find_by_scope::<T>(state.as_ref(), audience, scope)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let updated_class = do_update::<T>(state.as_ref(), &account_id, class, body).await?;
    let response: ClassResponseBody = (&updated_class).into();
    Ok(Response::builder(200)
        .body(
            serde_json::to_string(&response)
                .context("Failed to serialize minigroup")
                .error(AppErrorKind::SerializationFailed)?,
        )
        .build())
}

async fn do_update<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    class: crate::db::class::Object,
    body: ClassUpdate,
) -> Result<class::Object, error::Error> {
    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();

    state
        .authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;
    let event_update = get_event_update(&class, &body)
        .map(|(id, update)| state.event_client().update_room(id, update));
    let conference_update = get_coneference_update(&class, &body)
        .map(|(id, update)| state.conference_client().update_room(id, update));
    match (event_update, conference_update) {
        (None, None) => Ok(()),
        (None, Some(c)) => c.await,
        (Some(e), None) => e.await,
        (Some(e), Some(c)) => e.try_join(c).await.map(|_| ()),
    }
    .context("Services requests")
    .error(AppErrorKind::MqttRequestFailed)?;

    let mut query = crate::db::class::ClassUpdateQuery::new(class.id());
    if let Some(t) = body.time {
        query = query.time(t.into());
    }

    if let Some(r) = body.reserve {
        query = query.reserve(r);
    }

    if let Some(host) = body.host {
        query = query.host(host);
    }

    let mut conn = state.get_conn().await.error(AppErrorKind::DbQueryFailed)?;
    let class = query
        .execute(&mut conn)
        .await
        .context("Failed to update webinar")
        .error(AppErrorKind::DbQueryFailed)?;

    Ok(class)
}

fn get_coneference_update(
    class: &class::Object,
    update: &ClassUpdate,
) -> Option<(Uuid, ConfRoomUpdate)> {
    let conf_room_id = class.conference_room_id()?;
    let conf_update = ConfRoomUpdate {
        time: update.time.map(|(start, end)| match start {
            Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), end),
            Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
        }),
        reserve: update.reserve,
        classroom_id: None,
        host: update.host.clone(),
    };
    conf_update
        .is_empty_update()
        .not()
        .then(|| (conf_room_id, conf_update))
}

fn get_event_update(
    class: &class::Object,
    update: &ClassUpdate,
) -> Option<(Uuid, EventRoomUpdate)> {
    let update = EventRoomUpdate {
        time: update
            .time
            .map(|_| (Bound::Included(Utc::now()), Bound::Unbounded)),
        classroom_id: None,
    };

    update
        .is_empty_update()
        .not()
        .then(|| (class.event_room_id(), update))
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

    #[test]
    fn update_serde_test() {
        let update = "{}";
        let update: ClassUpdate = serde_json::from_str(update).unwrap();
        assert!(update.time.is_none());
        let update = "{\"reserve\": 10}";
        let _update: ClassUpdate = serde_json::from_str(update).unwrap();
    }

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
            host: None,
        };

        do_update::<WebinarType>(state.as_ref(), agent.account_id(), webinar, body)
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
            host: None,
        };

        let response =
            do_update::<WebinarType>(state.as_ref(), agent.account_id(), webinar.clone(), body)
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
        assert_eq!(updated_webinar.id(), response.id());
        assert_eq!(updated_webinar.time(), response.time());
        assert_eq!(updated_webinar.host(), response.host());
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
