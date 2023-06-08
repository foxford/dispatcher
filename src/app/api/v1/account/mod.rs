use std::sync::Arc;

use axum::{extract::Path, Extension};
use svc_agent::AccountId;
use svc_utils::extractors::AccountIdExtractor;

pub mod ban;

use crate::{
    app::{
        api::IntoJsonResponse,
        error::{Error, ErrorExt, ErrorKind},
        http::Json,
        AppContext,
    },
    db::class::KeyValueProperties,
};

use super::AppResult;

pub async fn read_property(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(property_id): Path<String>,
    AccountIdExtractor(account_id): AccountIdExtractor,
) -> AppResult {
    ReadProperty {
        ctx: ctx.as_ref(),
        account_id: &account_id,
        property_id,
    }
    .run()
    .await
    .and_then(|prop| {
        prop.into_json_response("Failed to serialize account property", http::StatusCode::OK)
    })
}

struct ReadProperty<'a> {
    ctx: &'a dyn AppContext,
    account_id: &'a AccountId,
    property_id: String,
}

impl ReadProperty<'_> {
    async fn run(self) -> Result<serde_json::Value, Error> {
        let account = read_account(self.ctx, self.account_id)
            .await
            .error(ErrorKind::AccountNotFound)?;

        let property = account
            .properties()
            .get(&self.property_id)
            .ok_or_else(|| Error::from(ErrorKind::AccountPropertyNotFound))?;

        Ok(property.clone())
    }
}

pub async fn update_property(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(property_id): Path<String>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Json(payload): Json<serde_json::Value>,
) -> AppResult {
    UpdateProperty {
        ctx: ctx.as_ref(),
        account_id: &account_id,
        property_id,
        payload,
    }
    .run()
    .await
    .and_then(|props| {
        props.into_json_response(
            "Failed to serialize account properties",
            http::StatusCode::OK,
        )
    })
}

struct UpdateProperty<'a> {
    ctx: &'a dyn AppContext,
    account_id: &'a AccountId,
    property_id: String,
    payload: serde_json::Value,
}

impl UpdateProperty<'_> {
    async fn run(self) -> Result<KeyValueProperties, Error> {
        let mut properties = KeyValueProperties::new();
        properties.insert(self.property_id, self.payload);

        let account = upsert_account(self.ctx, self.account_id, properties)
            .await
            .error(ErrorKind::DbQueryFailed)?;
        Ok(account.into_properties())
    }
}

pub async fn read_account(
    state: &dyn AppContext,
    id: &AccountId,
) -> anyhow::Result<crate::db::account::Object> {
    let mut conn = state.get_conn().await?;
    let account = crate::db::account::ReadQuery::by_id(id)
        .execute(&mut conn)
        .await?
        .ok_or_else(|| anyhow!("Failed to find account"))?;
    Ok(account)
}

pub async fn upsert_account(
    state: &dyn AppContext,
    id: &AccountId,
    properties: KeyValueProperties,
) -> anyhow::Result<crate::db::account::Object> {
    let mut conn = state.get_conn().await?;
    let account = crate::db::account::UpsertQuery::new(id, properties)
        .execute(&mut conn)
        .await?;
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::prelude::*;

    #[tokio::test]
    async fn update_property() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let db_pool = TestDb::new().await;
        let authz = TestAuthz::new();
        let state = TestState::new_with_pool(db_pool, authz);
        let state = Arc::new(state);

        let property_id = "test1".to_owned();
        let expected_property_value = serde_json::json!("test2");

        let props = UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: property_id.clone(),
            payload: expected_property_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        assert!(props.contains_key(&property_id));
    }

    #[tokio::test]
    async fn read_property() {
        let agent = TestAgent::new("web", "user2", USR_AUDIENCE);

        let db_pool = TestDb::new().await;
        let authz = TestAuthz::new();
        let state = TestState::new_with_pool(db_pool, authz);
        let state = Arc::new(state);

        let property_id = "test1".to_owned();
        let expected_property_value = serde_json::json!("test2");

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: property_id.clone(),
            payload: expected_property_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let property_value = ReadProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id,
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, expected_property_value);
    }

    #[tokio::test]
    async fn merge_properties() {
        let agent = TestAgent::new("web", "user3", USR_AUDIENCE);

        let db_pool = TestDb::new().await;
        let authz = TestAuthz::new();
        let state = TestState::new_with_pool(db_pool, authz);
        let state = Arc::new(state);

        let first_prop_id = "test1".to_owned();
        let first_prop_value = serde_json::json!("test2");

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
            payload: first_prop_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let second_prop_id = "test3".to_owned();
        let second_prop_value = serde_json::json!("test4");

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: second_prop_id.clone(),
            payload: second_prop_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let property_value = ReadProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, first_prop_value);

        let property_value = ReadProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: second_prop_id.clone(),
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, second_prop_value);
    }

    #[tokio::test]
    async fn overwrite_property() {
        let agent = TestAgent::new("web", "user4", USR_AUDIENCE);

        let db_pool = TestDb::new().await;
        let authz = TestAuthz::new();
        let state = TestState::new_with_pool(db_pool, authz);
        let state = Arc::new(state);

        let first_prop_id = "test1".to_owned();
        let first_prop_value = serde_json::json!("test2");

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
            payload: first_prop_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let property_value = ReadProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, first_prop_value);

        let second_prop_value = serde_json::json!("test4");

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
            payload: second_prop_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let property_value = ReadProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, second_prop_value);
    }

    #[tokio::test]
    async fn overwrite_property_complex() {
        let agent = TestAgent::new("web", "user5", USR_AUDIENCE);

        let db_pool = TestDb::new().await;
        let authz = TestAuthz::new();
        let state = TestState::new_with_pool(db_pool, authz);
        let state = Arc::new(state);

        let first_prop_id = "test1".to_owned();
        let first_prop_value = serde_json::json!({
            "test": "tset",
            "first": "second"
        });

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
            payload: first_prop_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let second_prop_id = "test2".to_owned();
        let second_prop_value = serde_json::json!("simple");

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: second_prop_id.clone(),
            payload: second_prop_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let property_value = ReadProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, first_prop_value);

        let second_prop_value = serde_json::json!({
            "tata": "tata"
        });

        UpdateProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
            payload: second_prop_value.clone(),
        }
        .run()
        .await
        .expect("Failed to set property");

        let property_value = ReadProperty {
            ctx: state.as_ref(),
            account_id: agent.account_id(),
            property_id: first_prop_id.clone(),
        }
        .run()
        .await
        .expect("Failed to read property");

        assert_eq!(property_value, second_prop_value);
    }
}
