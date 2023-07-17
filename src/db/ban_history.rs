use chrono::{DateTime, Utc};
use svc_authn::AccountId;
use uuid::Uuid;

#[derive(Debug)]
pub struct Object {
    #[allow(unused)]
    target_account: AccountId,
    #[allow(unused)]
    class_id: Uuid,
    #[allow(unused)]
    banned_at: DateTime<Utc>,
    #[allow(unused)]
    banned_operation_id: i64,
    #[allow(unused)]
    unbanned_at: Option<DateTime<Utc>>,
    #[allow(unused)]
    unbanned_operation_id: Option<i64>,
}

pub struct InsertQuery<'a> {
    class_id: Uuid,
    target_account: &'a AccountId,
    operation_id: i64,
}

impl<'a> InsertQuery<'a> {
    pub fn new(class_id: Uuid, target_account: &'a AccountId, operation_id: i64) -> Self {
        Self {
            class_id,
            target_account,
            operation_id,
        }
    }

    pub async fn execute(&self, conn: &mut sqlx::PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO ban_history (class_id, target_account, banned_operation_id)
            VALUES ($1, $2, $3)
            -- this allows us to run this query many times idempotently
            ON CONFLICT (banned_operation_id) DO NOTHING
            RETURNING
                target_account AS "target_account: AccountId",
                class_id,
                banned_at,
                banned_operation_id,
                unbanned_at,
                unbanned_operation_id
            "#,
            self.class_id,
            self.target_account as &AccountId,
            self.operation_id,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct FinishBanQuery<'a> {
    class_id: Uuid,
    target_account: &'a AccountId,
    operation_id: i64,
}

impl<'a> FinishBanQuery<'a> {
    pub fn new(class_id: Uuid, target_account: &'a AccountId, operation_id: i64) -> Self {
        Self {
            class_id,
            target_account,
            operation_id,
        }
    }

    pub async fn execute(&self, conn: &mut sqlx::PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE ban_history
            SET
                unbanned_at = NOW(),
                unbanned_operation_id = $3
            WHERE
                class_id = $1
            AND target_account = $2
            AND unbanned_at IS NULL
            RETURNING
                target_account AS "target_account: AccountId",
                class_id,
                banned_at,
                banned_operation_id,
                unbanned_at,
                unbanned_operation_id
            "#,
            self.class_id,
            self.target_account as &AccountId,
            self.operation_id,
        )
        .fetch_optional(conn)
        .await
    }
}
