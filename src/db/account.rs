use sqlx::PgConnection;
use svc_agent::AccountId;

use super::class::KeyValueProperties;

#[allow(dead_code)]
pub struct Object {
    id: AccountId,
    properties: KeyValueProperties,
}

impl Object {
    pub fn properties(&self) -> &KeyValueProperties {
        &self.properties
    }

    pub fn into_properties(self) -> KeyValueProperties {
        self.properties
    }
}

pub struct ReadQuery<'a> {
    id: &'a AccountId,
}

impl<'a> ReadQuery<'a> {
    pub fn by_id(id: &'a AccountId) -> Self {
        Self { id }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT
                id AS "id: _",
                properties AS "properties: _"
            FROM account
            WHERE
                id = $1
            LIMIT 1;
            "#,
            self.id as &AccountId,
        )
        .fetch_optional(conn)
        .await
    }
}

pub struct UpsertQuery<'a> {
    id: &'a AccountId,
    properties: KeyValueProperties,
}

impl<'a> UpsertQuery<'a> {
    pub fn new(id: &'a AccountId, properties: KeyValueProperties) -> Self {
        Self { id, properties }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO account (id, properties)
            VALUES ($1, $2)
            ON CONFLICT (id)
            DO UPDATE SET
                properties = account.properties || EXCLUDED.properties
            RETURNING
                id AS "id: _",
                properties AS "properties: _"
            "#,
            self.id as &AccountId,
            self.properties as KeyValueProperties
        )
        .fetch_one(conn)
        .await
    }
}
