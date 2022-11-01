use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Path};
use svc_authn::AccountId;
use svc_utils::extractors::AccountIdExtractor;
use uuid::Uuid;

use super::*;
use crate::app::api::v1::find_class;
use crate::app::api::IntoJsonResponse;
use crate::app::error::Error;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::http::Json;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::KeyValueProperties;

pub async fn read_property(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path((class_id, property_id)): Path<(Uuid, String)>,
    AccountIdExtractor(account_id): AccountIdExtractor,
) -> AppResult {
    ReadProperty {
        state: ctx.as_ref(),
        account_id: &account_id,
        class_id,
        property_id,
    }
    .run()
    .await
    .and_then(|prop| {
        prop.into_json_response("Failed to serialize class property", http::StatusCode::OK)
    })
}

struct ReadProperty<'a> {
    state: &'a dyn AppContext,
    account_id: &'a AccountId,
    class_id: Uuid,
    property_id: String,
}

impl ReadProperty<'_> {
    async fn run(self) -> Result<serde_json::Value, Error> {
        let class = find_class(self.state, self.class_id)
            .await
            .error(AppErrorKind::ClassNotFound)?;

        ClassAction {
            state: self.state,
            account_id: self.account_id,
            class: &class,
            op: "read",
        }
        .authorize()
        .await?;

        let property = class.properties().get(&self.property_id).ok_or_else(|| {
            Error::new(
                AppErrorKind::ClassPropertyNotFound,
                anyhow!("missing class property"),
            )
        })?;

        Ok(property.clone())
    }
}

pub async fn update_property(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path((class_id, property_id)): Path<(Uuid, String)>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Json(payload): Json<serde_json::Value>,
) -> AppResult {
    UpdateProperty {
        state: ctx.as_ref(),
        account_id: &account_id,
        class_id,
        property_id,
        payload,
    }
    .run()
    .await
    .and_then(|props| {
        props.into_json_response("Failed to serialize class properties", http::StatusCode::OK)
    })
}

struct UpdateProperty<'a> {
    state: &'a dyn AppContext,
    account_id: &'a AccountId,
    class_id: Uuid,
    property_id: String,
    payload: serde_json::Value,
}

impl UpdateProperty<'_> {
    async fn run(self) -> Result<KeyValueProperties, Error> {
        let class = find_class(self.state, self.class_id)
            .await
            .error(AppErrorKind::ClassNotFound)?;

        ClassAction {
            state: self.state,
            account_id: self.account_id,
            class: &class,
            op: "update",
        }
        .authorize()
        .await?;

        let mut properties = class.properties().clone();
        properties.insert(self.property_id, self.payload);

        let query =
            crate::db::class::ClassUpdateQuery::new(class.id()).properties(properties.clone());

        let mut conn = self
            .state
            .get_conn()
            .await
            .error(AppErrorKind::DbQueryFailed)?;
        query
            .execute(&mut conn)
            .await
            .context("Failed to update class properties")
            .error(AppErrorKind::DbQueryFailed)?;

        Ok(properties)
    }
}

struct ClassAction<'a> {
    state: &'a dyn AppContext,
    account_id: &'a AccountId,
    class: &'a class::Object,
    op: &'static str,
}

impl ClassAction<'_> {
    async fn authorize(self) -> Result<(), Error> {
        let object = AuthzObject::new(&["classrooms", &self.class.id().to_string()]).into();
        self.state
            .authz()
            .authorize(
                self.class.audience().to_owned(),
                self.account_id.clone(),
                object,
                self.op.into(),
            )
            .await
            .measure()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::iter::FromIterator;

    use super::*;
    use crate::{db::class::WebinarReadQuery, test_helpers::prelude::*};
    use mockall::predicate as pred;
    use uuid::Uuid;

    #[tokio::test]
    async fn read_property_unauthorized() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let state = TestState::new(TestAuthz::new()).await;
        let webinar = {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");

            let properties = serde_json::Map::from_iter(
                vec![("test1".to_owned(), serde_json::json!("test2"))].into_iter(),
            )
            .into();

            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                event_room_id,
                conference_room_id,
            )
            .properties(properties)
            .insert(&mut conn)
            .await
        };

        let state = Arc::new(state);

        ReadProperty {
            state: state.as_ref(),
            account_id: agent.account_id(),
            class_id: webinar.id(),
            property_id: "test1".to_owned(),
        }
        .run()
        .await
        .expect_err("Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn read_property() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let db_pool = TestDb::new().await;
        let webinar = {
            let mut conn = db_pool.get_conn().await;

            let properties = serde_json::Map::from_iter(
                vec![("test1".to_owned(), serde_json::json!("test2"))].into_iter(),
            )
            .into();

            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                event_room_id,
                conference_room_id,
            )
            .properties(properties)
            .insert(&mut conn)
            .await
        };

        let mut authz = TestAuthz::new();
        authz.allow(
            agent.account_id(),
            vec!["classrooms", &webinar.id().to_string()],
            "read",
        );

        let mut state = TestState::new_with_pool(db_pool, authz);
        update_webinar_mocks(&mut state, event_room_id, conference_room_id);

        let state = Arc::new(state);

        let property_value = ReadProperty {
            state: state.as_ref(),
            account_id: agent.account_id(),
            class_id: webinar.id(),
            property_id: "test1".to_owned(),
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, serde_json::json!("test2"));
    }

    #[tokio::test]
    async fn update_property_unauthorized() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let state = TestState::new(TestAuthz::new()).await;
        let webinar = {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");

            let properties = serde_json::Map::from_iter(
                vec![("test1".to_owned(), serde_json::json!("test2"))].into_iter(),
            )
            .into();

            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                event_room_id,
                conference_room_id,
            )
            .properties(properties)
            .insert(&mut conn)
            .await
        };

        let state = Arc::new(state);

        UpdateProperty {
            state: state.as_ref(),
            account_id: agent.account_id(),
            class_id: webinar.id(),
            property_id: "test1".to_owned(),
            payload: serde_json::json!("test3"),
        }
        .run()
        .await
        .expect_err("Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn update_property() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        let db_pool = TestDb::new().await;

        let webinar = {
            let mut conn = db_pool.get_conn().await;
            let properties = serde_json::Map::from_iter(
                vec![("test1".to_owned(), serde_json::json!("test2"))].into_iter(),
            )
            .into();

            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                event_room_id,
                conference_room_id,
            )
            .properties(properties)
            .reserve(20)
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
        let response = UpdateProperty {
            state: state.as_ref(),
            account_id: agent.account_id(),
            class_id: webinar.id(),
            property_id: "test1".to_owned(),
            payload: serde_json::json!("test3"),
        }
        .run()
        .await
        .expect("Failed to update webinar property");

        let mut conn = state.get_conn().await.expect("Failed to get conn");
        let updated_webinar = WebinarReadQuery::by_id(webinar.id())
            .execute(&mut conn)
            .await
            .expect("Failed to fetch webinar")
            .expect("Webinar not found");

        let should_be_props: KeyValueProperties = serde_json::json!({
            "test1": "test3"
        })
        .as_object()
        .unwrap()
        .clone()
        .into();

        assert_eq!(should_be_props, *updated_webinar.properties());
        assert_eq!(should_be_props, response);
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
