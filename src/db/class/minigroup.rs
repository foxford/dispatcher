use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::postgres::{types::PgRange, PgConnection};
use uuid::Uuid;

use super::{ClassType, Object, Time};

enum ReadQueryPredicate {
    Id(Uuid),
    Scope { audience: String, scope: String },
}

pub struct MinigroupReadQuery {
    condition: ReadQueryPredicate,
}

impl MinigroupReadQuery {
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

        let query = query.bind(ClassType::Minigroup);

        query.fetch_optional(conn).await
    }
}

pub struct MinigroupInsertQuery {
    scope: String,
    audience: String,
    time: Time,
    tags: Option<JsonValue>,
    preserve_history: bool,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
    reserve: Option<i32>,
}

impl MinigroupInsertQuery {
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
            reserve: None,
        }
    }

    pub fn tags(self, tags: JsonValue) -> Self {
        Self {
            tags: Some(tags),
            ..self
        }
    }

    #[cfg(test)]
    pub fn original_event_room_id(self, id: Uuid) -> Self {
        Self {
            original_event_room_id: Some(id),
            ..self
        }
    }

    #[cfg(test)]
    pub fn modified_event_room_id(self, id: Uuid) -> Self {
        Self {
            modified_event_room_id: Some(id),
            ..self
        }
    }

    pub fn reserve(self, reserve: i32) -> Self {
        Self {
            reserve: Some(reserve),
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        let time: PgRange<DateTime<Utc>> = self.time.into();

        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO class (
                scope, audience, time, tags, preserve_history, kind, conference_room_id,
                event_room_id, original_event_room_id, modified_event_room_id, reserve
            )
            VALUES ($1, $2, $3, $4, $5, $6::class_type, $7, $8, $9, $10, $11)
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
                reserve
            "#,
            self.scope,
            self.audience,
            time,
            self.tags,
            self.preserve_history,
            ClassType::Minigroup as ClassType,
            self.conference_room_id,
            self.event_room_id,
            self.original_event_room_id,
            self.modified_event_room_id,
            self.reserve,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct MinigroupTimeUpdateQuery {
    id: Uuid,
    time: Option<Time>,
}

impl MinigroupTimeUpdateQuery {
    pub fn new(id: Uuid) -> Self {
        Self { id, time: None }
    }

    pub fn time(&mut self, time: Time) -> &mut Self {
        self.time = Some(time);
        self
    }

    pub async fn execute(&self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        use quaint::ast::{Comparable, Update};
        use quaint::visitor::{Postgres, Visitor};

        let q = Update::table("class");
        let q = match self.time {
            Some(_) => q.set("time", "__placeholder__"),
            None => q,
        };

        let q = q
            .so_that("id".equals(self.id))
            .so_that("kind".equals("__placeholder__"));

        let (sql, _bindings) = Postgres::build(q);

        let query = sqlx::query_as(&sql);

        let query = match &self.time {
            Some(t) => {
                let t: PgRange<DateTime<Utc>> = t.into();
                query.bind(t)
            }
            None => query,
        };

        let query = query.bind(self.id);

        query.fetch_one(conn).await
    }
}
