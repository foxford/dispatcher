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
