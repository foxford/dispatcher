use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::postgres::PgConnection;
use uuid::Uuid;

use chrono::serde::ts_seconds;

use serde_derive::Serialize;
#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Object {
    id: Uuid,
    scope: String,
    audience: String,
    #[serde(with = "ts_seconds")]
    created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    event_room_id: Uuid,
}

impl Object {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn scope(&self) -> String {
        self.scope.clone()
    }

    pub fn event_room_id(&self) -> Uuid {
        self.event_room_id
    }

    pub fn audience(&self) -> String {
        self.audience.clone()
    }
}
enum ReadQueryPredicate {
    Id(Uuid),
    Scope { audience: String, scope: String },
}

pub struct ChatReadQuery {
    condition: ReadQueryPredicate,
}

impl ChatReadQuery {
    pub fn by_id(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::Id(id),
        }
    }

    pub fn by_scope(audience: String, scope: String) -> Self {
        Self {
            condition: ReadQueryPredicate::Scope { audience, scope },
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        use quaint::ast::{Comparable, Select};
        use quaint::visitor::{Postgres, Visitor};

        let q = Select::from_table("chat");

        let q = match self.condition {
            ReadQueryPredicate::Id(_) => q.and_where("id".equals("_placeholder_")),
            ReadQueryPredicate::Scope { .. } => q
                .and_where("audience".equals("_placeholder_"))
                .and_where("scope".equals("_placeholder_")),
        };

        let (sql, _bindings) = Postgres::build(q);

        let query = sqlx::query_as(&sql);

        let query = match self.condition {
            ReadQueryPredicate::Id(id) => query.bind(id),
            ReadQueryPredicate::Scope { audience, scope } => query.bind(audience).bind(scope),
        };

        query.fetch_optional(conn).await
    }
}

pub struct ChatInsertQuery {
    scope: String,
    audience: String,
    tags: Option<JsonValue>,
    event_room_id: Uuid,
}

impl ChatInsertQuery {
    pub fn new(scope: String, audience: String, event_room_id: Uuid) -> Self {
        Self {
            scope,
            audience,
            tags: None,
            event_room_id,
        }
    }

    pub fn tags(self, tags: JsonValue) -> Self {
        Self {
            tags: Some(tags),
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO chat (
                scope, audience, tags, event_room_id
            )
            VALUES ($1, $2, $3, $4)
            RETURNING
                id,
                scope,
                audience,
                tags,
                created_at,
                event_room_id
            "#,
            self.scope,
            self.audience,
            self.tags,
            self.event_room_id,
        )
        .fetch_one(conn)
        .await
    }
}
