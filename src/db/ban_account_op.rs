use sqlx::PgConnection;
use svc_authn::AccountId;
use uuid::Uuid;

pub struct Object {
    pub user_account: AccountId,
    pub last_op_id: Uuid,
    pub last_op_done: bool,
}

pub struct ReadQuery<'a> {
    user_account: &'a AccountId,
}

impl<'a> ReadQuery<'a> {
    pub fn by_id(user_account: &'a AccountId) -> Self {
        Self { user_account }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT
                user_account AS "user_account: _",
                last_op_id AS "last_op_id: _",
                last_op_done
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
/// in database. Returns `None` if `last_op_id` in database differs.
pub struct UpsertQuery {
    user_account: AccountId,
    op_done: bool,
    last_op_id: Uuid,
    new_op_id: Uuid,
}

impl UpsertQuery {
    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO ban_account_op (user_account, last_op_id, last_op_done)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_account) DO UPDATE
            SET
                last_op_done = EXCLUDED.last_op_done,
                last_op_id   = EXCLUDED.last_op_id
            WHERE
                -- allow change op id iff the previous operation is completed
                (
                    ban_account_op.last_op_id = $4 AND
                    ban_account_op.last_op_done = true
                ) OR
                -- allow to 'complete' operation iff there's no change in last_op_id
                ban_account_op.last_op_id = EXCLUDED.last_op_id
            RETURNING
                user_account AS "user_account: _",
                last_op_id,
                last_op_done
            "#,
            self.user_account as AccountId,
            self.new_op_id,
            self.op_done,
            self.last_op_id
        )
        .fetch_optional(conn)
        .await
    }
}
