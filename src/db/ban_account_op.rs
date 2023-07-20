use sqlx::PgConnection;
use svc_authn::AccountId;

#[derive(Debug)]
pub struct Object {
    pub user_account: AccountId,
    pub last_op_id: i64,
    pub is_video_streaming_banned: bool,
    pub is_collaboration_banned: bool,
}

impl Object {
    pub fn is_completed(&self) -> bool {
        self.is_video_streaming_banned && self.is_collaboration_banned
    }
}

pub struct ReadQuery<'a> {
    user_account: &'a AccountId,
}

impl<'a> ReadQuery<'a> {
    pub fn by_account_id(user_account: &'a AccountId) -> Self {
        Self { user_account }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT
                user_account AS "user_account: _",
                last_op_id AS "last_op_id: _",
                is_video_streaming_banned,
                is_collaboration_banned
            FROM ban_account_op
            WHERE
                user_account = $1
            LIMIT 1;
            "#,
            self.user_account as &AccountId,
        )
        .fetch_optional(conn)
        .await
    }
}

pub struct NextSeqId {
    pub value: i64,
}

pub async fn get_next_seq_id(conn: &mut PgConnection) -> sqlx::Result<NextSeqId> {
    sqlx::query_as!(
        NextSeqId,
        r#"SELECT nextval('ban_entity_seq_id') as "value!: i64";"#
    )
    .fetch_one(conn)
    .await
}

/// Upsert works only if provided `last_op_id` equals to the one stored
/// in database or `last_op_id` equals to `new_op_id`.
/// Returns `None` if `last_op_id` in database differs.
pub struct UpsertQuery {
    user_account: AccountId,
    last_op_id: i64,
    new_op_id: i64,
}

impl UpsertQuery {
    pub fn new_operation(user_account: AccountId, last_op_id: i64, new_op_id: i64) -> Self {
        Self {
            user_account,
            last_op_id,
            new_op_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO ban_account_op (user_account, last_op_id)
            VALUES ($1, $2)
            ON CONFLICT (user_account) DO UPDATE
            SET
                -- to reset sub-operation trackers only if op_id has changed
                is_video_streaming_banned = ($2 = $3 AND ban_account_op.is_video_streaming_banned),
                is_collaboration_banned   = ($2 = $3 AND ban_account_op.is_collaboration_banned),
                last_op_id                = EXCLUDED.last_op_id
            WHERE
                -- allow to 'complete' operation if there's no change in last_op_id
                -- or allow to do upsert without real changes so we can process
                -- the same message twice
                ban_account_op.last_op_id = EXCLUDED.last_op_id OR
                -- allow change op id if the previous operation is completed
                (
                    ban_account_op.last_op_id                = $3 AND
                    ban_account_op.is_video_streaming_banned = true AND
                    ban_account_op.is_collaboration_banned   = true
                )
            RETURNING
                user_account AS "user_account: _",
                last_op_id,
                is_video_streaming_banned,
                is_collaboration_banned
            "#,
            self.user_account as AccountId,
            self.new_op_id,
            self.last_op_id
        )
        .fetch_optional(conn)
        .await
    }
}

pub struct UpdateQuery {
    user_account: AccountId,
    is_video_streaming_banned: Option<bool>,
    is_collaboration_banned: Option<bool>,
    last_op_id: i64,
}

impl UpdateQuery {
    pub fn new_video_streaming_banned(user_account: AccountId, op_id: i64) -> Self {
        Self {
            user_account,
            is_video_streaming_banned: Some(true),
            is_collaboration_banned: None,
            last_op_id: op_id,
        }
    }

    pub fn new_collaboration_banned(user_account: AccountId, op_id: i64) -> Self {
        Self {
            user_account,
            is_video_streaming_banned: None,
            is_collaboration_banned: Some(true),
            last_op_id: op_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE ban_account_op
            SET
                is_video_streaming_banned = COALESCE($2, ban_account_op.is_video_streaming_banned),
                is_collaboration_banned   = COALESCE($3, ban_account_op.is_collaboration_banned)
            WHERE
                ban_account_op.last_op_id = $4
            AND ban_account_op.user_account = $1
            RETURNING
                user_account AS "user_account: _",
                last_op_id,
                is_video_streaming_banned,
                is_collaboration_banned
            "#,
            self.user_account as AccountId,
            self.is_video_streaming_banned,
            self.is_collaboration_banned,
            self.last_op_id
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
    async fn doesnt_allow_to_change_op_id_until_operation_is_completed(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;
        let mut conn = state.get_conn().await.expect("Failed to fetch connection");

        let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);

        let r = UpsertQuery::new_operation(agent1.account_id().clone(), 0, 0)
            .execute(&mut conn)
            .await
            .expect("failed to start new ban operation");
        assert!(r.is_some());

        async fn test_possible_failure_states(agent: &TestAgent, conn: &mut sqlx::PgConnection) {
            // should fail b/c previous operation is not completed yet
            let r = UpsertQuery::new_operation(agent.account_id().clone(), 0, 100)
                .execute(conn)
                .await
                .expect("failed to start new ban operation");
            assert!(r.is_none());

            // should fail b/c we passed the wrong previous operation id
            let r = UpsertQuery::new_operation(agent.account_id().clone(), 50, 100)
                .execute(conn)
                .await
                .expect("failed to start new ban operation");
            assert!(r.is_none());

            // should be ok since we didn't change anything (idempotency)
            let r = UpsertQuery::new_operation(agent.account_id().clone(), 0, 0)
                .execute(conn)
                .await
                .expect("failed to start new ban operation");
            assert!(r.is_some());
        }

        test_possible_failure_states(&agent1, &mut conn).await;

        // should fail b/c we passed to wrong operation id
        let r = UpdateQuery::new_collaboration_banned(agent1.account_id().clone(), 10)
            .execute(&mut conn)
            .await
            .expect("failed to complete collaboration ban");
        assert!(r.is_none());

        let r = UpdateQuery::new_collaboration_banned(agent1.account_id().clone(), 0)
            .execute(&mut conn)
            .await
            .expect("failed to complete collaboration ban");
        assert!(r.is_some());
        assert!(matches!(
            r,
            Some(Object {
                is_collaboration_banned: true,
                last_op_id: 0,
                ..
            })
        ));

        // should be ok to run twice (idempotency)
        let r = UpdateQuery::new_collaboration_banned(agent1.account_id().clone(), 0)
            .execute(&mut conn)
            .await
            .expect("failed to complete collaboration ban");
        assert!(r.is_some());

        test_possible_failure_states(&agent1, &mut conn).await;

        // should fail b/c we passed to wrong operation id
        let r = UpdateQuery::new_video_streaming_banned(agent1.account_id().clone(), 10)
            .execute(&mut conn)
            .await
            .expect("failed to complete video streaming ban");
        assert!(r.is_none());

        let r = UpdateQuery::new_video_streaming_banned(agent1.account_id().clone(), 0)
            .execute(&mut conn)
            .await
            .expect("failed to complete video streaming ban");
        assert!(r.is_some());
        assert!(matches!(
            r,
            Some(Object {
                is_video_streaming_banned: true,
                is_collaboration_banned: true,
                last_op_id: 0,
                ..
            })
        ));

        // should be ok to run twice (idempotency)
        let r = UpdateQuery::new_video_streaming_banned(agent1.account_id().clone(), 0)
            .execute(&mut conn)
            .await
            .expect("failed to complete video streaming ban");
        assert!(r.is_some());

        // should fail b/c we passed the wrong previous operation id
        let r = UpsertQuery::new_operation(agent1.account_id().clone(), 50, 100)
            .execute(&mut conn)
            .await
            .expect("failed to start new ban operation");
        assert!(r.is_none());

        // should be ok since we didn't change anything (idempotency)
        let r = UpsertQuery::new_operation(agent1.account_id().clone(), 0, 0)
            .execute(&mut conn)
            .await
            .expect("failed to start new ban operation");
        assert!(r.is_some());

        // should be ok to start new operation afterwards
        let r = UpsertQuery::new_operation(agent1.account_id().clone(), 0, 1)
            .execute(&mut conn)
            .await
            .expect("failed to complete video streaming ban");
        assert!(r.is_some());
    }
}
