use chrono::Duration;
use sqlx::PgConnection;
use svc_authn::AccountId;
use uuid::Uuid;

pub struct Object {
    position_secs: i32,
}

impl Object {
    pub fn position_secs(&self) -> i32 {
        self.position_secs
    }
}

pub struct UpsertQuery {
    class_id: Uuid,
    account_id: AccountId,
    position: Duration,
}

impl UpsertQuery {
    pub fn new(class_id: Uuid, account_id: AccountId, position: Duration) -> Self {
        Self {
            class_id,
            account_id,
            position,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<usize> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO record_timestamp (
                class_id, account_id, position_secs
            )
            VALUES ($1, $2, $3)
            ON CONFLICT (class_id, account_id)
            DO UPDATE
            SET position_secs = EXCLUDED.position_secs, updated_at = NOW()
            "#,
            self.class_id,
            self.account_id as AccountId,
            clamp_secs(self.position.num_seconds())
        )
        .execute(conn)
        .await
        .map(|r| r.rows_affected() as usize)
    }
}

fn clamp_secs(n: i64) -> i32 {
    use std::cmp::*;

    min(max(0, n), 3600 * 24) as i32
}

pub struct FindQuery {
    class_id: Uuid,
    account_id: AccountId,
}

impl FindQuery {
    pub fn new(class_id: Uuid, account_id: AccountId) -> Self {
        Self {
            class_id,
            account_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT position_secs
            FROM record_timestamp
            WHERE class_id = $1
            AND account_id = $2
            LIMIT 1;
            "#,
            self.class_id,
            self.account_id as AccountId,
        )
        .fetch_optional(conn)
        .await
    }
}
