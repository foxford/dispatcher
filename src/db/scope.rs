use chrono::{serde::ts_seconds, DateTime, Utc};
use serde_derive::{Deserialize, Serialize};
use sqlx::postgres::PgConnection;

///////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Object {
    pub id: i64,
    pub scope: String,
    pub app: String,
    pub frontend_id: i64,
    #[serde(with = "ts_seconds")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub(crate) struct ListQuery {}

impl ListQuery {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Vec<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT *
            FROM scope
            "#,
        )
        .fetch_all(conn)
        .await
    }
}

#[derive(Debug)]
pub(crate) struct DeleteQuery {
    scope: String,
}

impl DeleteQuery {
    pub(crate) fn new(scope: String) -> Self {
        Self { scope }
    }

    pub(crate) async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM scope WHERE scope = $1", self.scope)
            .execute(conn)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[derive(Debug)]
    pub(crate) struct InsertQuery {
        scope: String,
        frontend_id: i64,
        app: String,
    }

    impl InsertQuery {
        pub(crate) fn new(scope: String, frontend_id: i64, app: String) -> Self {
            Self {
                scope,
                frontend_id,
                app,
            }
        }

        pub(crate) async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
            sqlx::query_as!(
                Object,
                r#"
                INSERT INTO scope (scope, frontend_id, app)
                VALUES ($1, $2, $3)
                RETURNING id, scope, frontend_id, created_at, app
            "#,
                self.scope,
                self.frontend_id,
                self.app
            )
            .fetch_one(conn)
            .await
        }
    }
}
