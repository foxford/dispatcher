use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::postgres::{types::PgRange, PgConnection};
use svc_agent::AccountId;
use uuid::Uuid;

use super::{ClassType, Object, Time};

enum ReadQueryPredicate {
    Id(Uuid),
    Scope { audience: String, scope: String },
}

pub struct WebinarReadQuery {
    condition: ReadQueryPredicate,
}

impl WebinarReadQuery {
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

        let query = query.bind(ClassType::Webinar);

        query.fetch_optional(conn).await
    }
}

pub struct WebinarInsertQuery {
    scope: String,
    audience: String,
    time: Time,
    tags: Option<JsonValue>,
    preserve_history: bool,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
}

impl WebinarInsertQuery {
    pub fn new(
        scope: String,
        audience: String,
        time: Time,
        conference_room_id: Uuid,
        event_room_id: Uuid,
    ) -> Self {
        Self {
            scope,
            audience,
            time,
            tags: None,
            preserve_history: true,
            conference_room_id,
            event_room_id,
            original_event_room_id: None,
            modified_event_room_id: None,
        }
    }

    pub fn tags(self, tags: JsonValue) -> Self {
        Self {
            tags: Some(tags),
            ..self
        }
    }

    pub fn original_event_room_id(self, id: Uuid) -> Self {
        Self {
            original_event_room_id: Some(id),
            ..self
        }
    }

    pub fn modified_event_room_id(self, id: Uuid) -> Self {
        Self {
            modified_event_room_id: Some(id),
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        let time: PgRange<DateTime<Utc>> = self.time.into();

        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO class (
                scope, audience, time, tags, preserve_history, kind,
                conference_room_id, event_room_id,
                original_event_room_id, modified_event_room_id
            )
            VALUES ($1, $2, $3, $4, $5, $6::class_type, $7, $8, $9, $10)
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                host AS "host?: AccountId",
                time AS "time!: Time",
                tags,
                preserve_history,
                created_at,
                event_room_id,
                conference_room_id,
                original_event_room_id,
                modified_event_room_id
            "#,
            self.scope,
            self.audience,
            time,
            self.tags,
            self.preserve_history,
            ClassType::Webinar as ClassType,
            self.conference_room_id,
            self.event_room_id,
            self.original_event_room_id,
            self.modified_event_room_id,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct WebinarTimeUpdateQuery {
    id: Uuid,
    time: Time,
}

impl WebinarTimeUpdateQuery {
    pub fn new(id: Uuid, time: Time) -> Self {
        Self { id, time }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        let time: PgRange<DateTime<Utc>> = self.time.into();

        sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET time = $2
            WHERE id = $1
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                host AS "host?: AccountId",
                time AS "time!: Time",
                tags,
                preserve_history,
                created_at,
                event_room_id,
                conference_room_id,
                original_event_room_id,
                modified_event_room_id
            "#,
            self.id,
            time,
        )
        .fetch_one(conn)
        .await
    }
}
