use sqlx::PgConnection;
use svc_authn::AccountId;
use uuid::Uuid;

pub struct Object {
    pub user_account: AccountId,
    pub last_op_id: Uuid,
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
    is_video_streaming_banned: Option<bool>,
    is_collaboration_banned: Option<bool>,
    last_op_id: Uuid,
    new_op_id: Uuid,
}

impl UpsertQuery {
    pub fn new_operation(user_account: AccountId, last_op_id: Uuid, new_op_id: Uuid) -> Self {
        Self {
            user_account,
            is_video_streaming_banned: None,
            is_collaboration_banned: None,
            last_op_id,
            new_op_id,
        }
    }

    pub fn new_video_streaming_banned(user_account: AccountId, op_id: Uuid) -> Self {
        Self {
            user_account,
            is_video_streaming_banned: Some(true),
            is_collaboration_banned: None,
            last_op_id: op_id,
            new_op_id: op_id,
        }
    }

    pub fn new_collaboration_banned(user_account: AccountId, op_id: Uuid) -> Self {
        Self {
            user_account,
            is_video_streaming_banned: None,
            is_collaboration_banned: Some(true),
            last_op_id: op_id,
            new_op_id: op_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO ban_account_op (user_account, last_op_id, is_video_streaming_banned, is_collaboration_banned)
            VALUES ($1, $2, COALESCE($3, false), COALESCE($4, false))
            ON CONFLICT (user_account) DO UPDATE
            SET
                is_video_streaming_banned = COALESCE(EXCLUDED.is_video_streaming_banned, ban_account_op.is_video_streaming_banned),
                is_collaboration_banned   = COALESCE(EXCLUDED.is_collaboration_banned, ban_account_op.is_collaboration_banned),
                last_op_id                = EXCLUDED.last_op_id
            WHERE
                -- allow to 'complete' operation if there's no change in last_op_id
                -- or allow to do upsert without real changes so we can process
                -- the same message twice
                ban_account_op.last_op_id = EXCLUDED.last_op_id OR
                -- allow change op id if the previous operation is completed
                (
                    ban_account_op.last_op_id                = $5 AND
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
            self.is_video_streaming_banned,
            self.is_collaboration_banned,
            self.last_op_id
        )
        .fetch_optional(conn)
        .await
    }
}
