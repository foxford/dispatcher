use std::ops::Bound;

use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::postgres::{types::PgRange, PgConnection};
use uuid::Uuid;

use super::{ClassType, Object, Time};

enum ReadQueryPredicate {
    Id(Uuid),
    Scope { audience: String, scope: String },
}

pub struct P2PReadQuery {
    condition: ReadQueryPredicate,
}

impl P2PReadQuery {
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

        let q = Select::from_table("class");

        let q = match self.condition {
            ReadQueryPredicate::Id(_) => q.and_where("id".equals("_placeholder_")),
            ReadQueryPredicate::Scope { .. } => q
                .and_where("audience".equals("_placeholder_"))
                .and_where("scope".equals("_placeholder_")),
        };

        let q = q.and_where("kind".equals("_placeholder_"));

        let (sql, _bindings) = Postgres::build(q);

        let query = sqlx::query_as(&sql);

        let query = match self.condition {
            ReadQueryPredicate::Id(id) => query.bind(id),
            ReadQueryPredicate::Scope { audience, scope } => query.bind(audience).bind(scope),
        };
        let query = query.bind(ClassType::P2P);

        query.fetch_optional(conn).await
    }
}

pub struct P2PInsertQuery {
    scope: String,
    audience: String,
    tags: Option<JsonValue>,
    conference_room_id: Uuid,
    event_room_id: Uuid,
}

impl P2PInsertQuery {
    pub fn new(
        scope: String,
        audience: String,
        conference_room_id: Uuid,
        event_room_id: Uuid,
    ) -> Self {
        Self {
            scope,
            audience,
            tags: None,
            conference_room_id,
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
        let time: PgRange<DateTime<Utc>> = (Bound::Unbounded, Bound::Unbounded).into();

        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO class (
                scope, audience, time, tags, preserve_history, kind,
                conference_room_id, event_room_id
            )
            VALUES ($1, $2, $3, $4, $5, $6::class_type, $7, $8)
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                time AS "time!: Time",
                tags,
                preserve_history,
                created_at,
                event_room_id,
                conference_room_id,
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri
            "#,
            self.scope,
            self.audience,
            time,
            self.tags,
            false,
            ClassType::P2P as ClassType,
            self.conference_room_id,
            self.event_room_id,
        )
        .fetch_one(conn)
        .await
    }
}
