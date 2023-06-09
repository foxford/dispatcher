use chrono::{serde::ts_seconds, DateTime, Utc};
use serde_derive::{Deserialize, Serialize};
use sqlx::postgres::PgConnection;

///////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Object {
    pub id: i64,
    pub url: String,
    #[serde(with = "ts_seconds")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct ListQuery {}

impl ListQuery {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Vec<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT *
            FROM frontend
            "#,
        )
        .fetch_all(conn)
        .await
    }
}

#[derive(Debug)]
pub(crate) struct FrontendByScopeQuery {
    scope: String,
    app: String,
}

impl FrontendByScopeQuery {
    pub fn new(scope: String, app: String) -> Self {
        Self { scope, app }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT fe.*
            FROM frontend fe
            INNER JOIN scope s
            ON s.frontend_id = fe.id
            WHERE s.scope = $1 AND s.app = $2
            "#,
            self.scope,
            self.app
        )
        .fetch_optional(conn)
        .await
    }
}
