use chrono::{DateTime, Utc};
use svc_authn::AccountId;
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq)]
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
            WITH i AS (
                INSERT INTO ban_history (class_id, target_account, banned_operation_id)
                VALUES ($1, $2, $3)
                -- this allows us to run this query many times idempotently
                ON CONFLICT (banned_operation_id) DO NOTHING
                RETURNING *
            )
            -- gets row if there was no conflict
            SELECT
                target_account AS "target_account!: AccountId",
                class_id AS "class_id!: _",
                banned_at AS "banned_at!: _",
                banned_operation_id AS "banned_operation_id!: _",
                unbanned_at,
                unbanned_operation_id
            FROM i
            -- or selects original row if there was a conflict
            UNION
                SELECT
                    target_account AS "target_account: AccountId",
                    class_id,
                    banned_at,
                    banned_operation_id,
                    unbanned_at,
                    unbanned_operation_id
                FROM ban_history
                WHERE
                    banned_operation_id = $3
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::AppContext;
    use crate::test_helpers::prelude::*;

    #[sqlx::test]
    async fn finishes_ban(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;
        let mut conn = state.get_conn().await.expect("Failed to fetch connection");

        let minigroup = factory::Minigroup::new(
            random_string(),
            USR_AUDIENCE.to_string(),
            (Bound::Unbounded, Bound::Unbounded).into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .insert(&mut conn)
        .await;

        let entry_start = InsertQuery::new(minigroup.id(), &AccountId::new("test", "test"), 0)
            .execute(&mut conn)
            .await
            .expect("failed to insert ban history entry");

        let entry_finish = FinishBanQuery::new(minigroup.id(), &entry_start.target_account, 1)
            .execute(&mut conn)
            .await
            .expect("failed to run finish ban query")
            .expect("failed to find entry");

        assert_eq!(entry_start.target_account, entry_finish.target_account);
        assert_eq!(entry_start.class_id, entry_finish.class_id);
        assert_eq!(entry_start.banned_at, entry_finish.banned_at);
        assert_eq!(
            entry_start.banned_operation_id,
            entry_finish.banned_operation_id
        );
        assert_eq!(entry_finish.unbanned_operation_id, Some(1));
    }

    #[sqlx::test]
    async fn inserts_idempotently(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;
        let mut conn = state.get_conn().await.expect("Failed to fetch connection");

        let minigroup1 = factory::Minigroup::new(
            random_string(),
            USR_AUDIENCE.to_string(),
            (Bound::Unbounded, Bound::Unbounded).into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .insert(&mut conn)
        .await;

        let entry1 = InsertQuery::new(minigroup1.id(), &AccountId::new("test", "test"), 0)
            .execute(&mut conn)
            .await
            .expect("failed to insert ban history entry");

        let minigroup2 = factory::Minigroup::new(
            random_string(),
            USR_AUDIENCE.to_string(),
            (Bound::Unbounded, Bound::Unbounded).into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .insert(&mut conn)
        .await;

        let entry2 = InsertQuery::new(
            minigroup2.id(),
            &AccountId::new("test-another", "test-another"),
            0,
        )
        .execute(&mut conn)
        .await
        .expect("failed to insert ban history entry");

        // so we just got original entry back
        assert_eq!(entry1, entry2);
    }
}
