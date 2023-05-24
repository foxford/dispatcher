use sqlx::PgConnection;
use svc_authn::AccountId;
use uuid::Uuid;

pub struct Object {
    pub user_account: AccountId,
    pub last_op_id: Uuid,
    pub video_complete: bool,
    pub event_access_complete: bool,
}

impl Object {
    pub fn complete(&self) -> bool {
        self.video_complete && self.event_access_complete
    }
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
                video_complete,
                event_access_complete
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
    video_complete: Option<bool>,
    event_access_complete: Option<bool>,
    last_op_id: Uuid,
    new_op_id: Uuid,
}

impl UpsertQuery {
    pub fn new_operation(user_account: AccountId, last_op_id: Uuid, new_op_id: Uuid) -> Self {
        Self {
            user_account,
            video_complete: None,
            event_access_complete: None,
            last_op_id,
            new_op_id,
        }
    }

    pub fn new_video_complete(user_account: AccountId, op_id: Uuid) -> Self {
        Self {
            user_account,
            video_complete: Some(true),
            event_access_complete: None,
            last_op_id: op_id,
            new_op_id: op_id,
        }
    }

    pub fn new_event_access_complete(user_account: AccountId, op_id: Uuid) -> Self {
        Self {
            user_account,
            video_complete: None,
            event_access_complete: Some(true),
            last_op_id: op_id,
            new_op_id: op_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO ban_account_op (user_account, last_op_id, video_complete, event_access_complete)
            VALUES ($1, $2, COALESCE($3, false), COALESCE($4, false))
            ON CONFLICT (user_account) DO UPDATE
            SET
                video_complete        = COALESCE(EXCLUDED.video_complete, ban_account_op.video_complete),
                event_access_complete = COALESCE(EXCLUDED.event_access_complete, ban_account_op.event_access_complete),
                last_op_id            = EXCLUDED.last_op_id
            WHERE
                -- allow to 'complete' operation iff there's no change in last_op_id
                -- or allow to do upsert without real changes so we can process
                -- the same message twice
                ban_account_op.last_op_id = EXCLUDED.last_op_id OR
                -- allow change op id iff the previous operation is completed
                (
                    ban_account_op.last_op_id            = $5 AND
                    ban_account_op.video_complete        = true AND
                    ban_account_op.event_access_complete = true
                )
            RETURNING
                user_account AS "user_account: _",
                last_op_id,
                video_complete,
                event_access_complete
            "#,
            self.user_account as AccountId,
            self.new_op_id,
            self.video_complete,
            self.event_access_complete,
            self.last_op_id
        )
        .fetch_optional(conn)
        .await
    }
}
