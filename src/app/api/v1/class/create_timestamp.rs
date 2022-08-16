use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, Path};
use hyper::{Body, Response};
use serde_derive::Deserialize;
use svc_utils::extractors::AccountIdExtractor;
use uuid::Uuid;

use super::{find, AppResult};
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::AsClassType;

#[derive(Deserialize)]
pub struct TimestampPayload {
    #[serde(with = "crate::serde::duration_seconds")]
    position: chrono::Duration,
}

pub async fn create_timestamp<T: AsClassType>(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Json(body): Json<TimestampPayload>,
) -> AppResult {
    let class = find::<T>(ctx.as_ref(), id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();

    ctx.authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await
        .measure()?;

    let query = crate::db::record_timestamp::UpsertQuery::new(
        class.id(),
        account_id.clone(),
        body.position,
    );

    let mut conn = ctx.get_conn().await.error(AppErrorKind::DbQueryFailed)?;

    query
        .execute(&mut conn)
        .await
        .with_context(|| format!("Failed to update {}", T::as_str()))
        .error(AppErrorKind::DbQueryFailed)?;

    let response = Response::builder()
        .status(http::StatusCode::CREATED)
        .body(Body::empty())
        .unwrap();

    Ok(response)
}

#[cfg(test)]
mod create_timestamp_tests {
    use super::*;
    use crate::{db::class::WebinarType, test_helpers::prelude::*};
    use chrono::{Duration, Utc};
    use serde_json::Value as JsonValue;
    use std::ops::Bound;
    use uuid::Uuid;

    #[tokio::test]
    async fn create_timestamp_unauthorized() {
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
        let body: TimestampPayload = serde_json::from_str(r#"{ "position": 100 }"#).unwrap();

        create_timestamp::<WebinarType>(
            Extension(state),
            Path(webinar.id()),
            AccountIdExtractor(agent.account_id().to_owned()),
            Json(body),
        )
        .await
        .expect_err("Unexpected success, should fail due to authz");
    }

    #[tokio::test]
    async fn create_webinar_timestamp() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let db_pool = TestDb::new().await;

        let webinar = {
            let mut conn = db_pool.get_conn().await;
            let webinar = factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (
                    Bound::Unbounded,
                    Bound::Excluded(Utc::now() - Duration::seconds(10)),
                )
                    .into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await;

            factory::Recording::new(webinar.id(), Uuid::new_v4(), agent.agent_id().clone())
                .transcoded_at(Utc::now())
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
        let body: TimestampPayload = serde_json::from_str(r#"{ "position": 50 }"#).unwrap();

        create_timestamp::<WebinarType>(
            Extension(state.clone()),
            Path(webinar.id()),
            AccountIdExtractor(agent.account_id().to_owned()),
            Json(body),
        )
        .await
        .expect("Failed to save timestamp");

        // Assert DB changes.
        let response = super::super::read::<WebinarType>(
            Extension(state),
            Path(webinar.id()),
            axum::extract::RawQuery(None),
            AccountIdExtractor(agent.account_id().to_owned()),
        )
        .await
        .expect("Failed to read webinar");

        let body = response.into_body();
        let body = hyper::body::to_bytes(body).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        let body = serde_json::from_str::<JsonValue>(&body).unwrap();
        let position = body.get("position").unwrap().as_i64().unwrap();
        assert_eq!(position, 50);
    }
}
