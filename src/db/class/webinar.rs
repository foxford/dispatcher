use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::postgres::{types::PgRange, PgConnection};
use sqlx::Done;
use uuid::Uuid;

use super::{ClassType, Object, Time};
#[cfg(test)]
use super::{GenericReadQuery, WebinarType};

#[cfg(test)]
pub type WebinarReadQuery = GenericReadQuery<WebinarType>;

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
    reserve: Option<i32>,
    room_events_uri: Option<String>,
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
            reserve: None,
            room_events_uri: None,
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
                scope, audience, time, tags, preserve_history, kind,
                conference_room_id, event_room_id,
                original_event_room_id, modified_event_room_id, reserve, room_events_uri
            )
            VALUES ($1, $2, $3, $4, $5, $6::class_type, $7, $8, $9, $10, $11, $12)
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
            self.preserve_history,
            ClassType::Webinar as ClassType,
            self.conference_room_id,
            self.event_room_id,
            self.original_event_room_id,
            self.modified_event_room_id,
            self.reserve,
            self.room_events_uri,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct WebinarUpdateQuery {
    id: Uuid,
    time: Option<Time>,
    reserve: Option<i32>,
}

impl WebinarUpdateQuery {
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            time: None,
            reserve: None,
        }
    }

    pub fn time(mut self, time: Time) -> Self {
        self.time = Some(time);
        self
    }

    pub fn reserve(mut self, reserve: i32) -> Self {
        self.reserve = Some(reserve);
        self
    }

    pub async fn execute(&self, conn: &mut PgConnection) -> sqlx::Result<u64> {
        use quaint::ast::{Comparable, Conjuctive, Update};
        use quaint::visitor::{Postgres, Visitor};

        let q = Update::table("class");
        let q = match (&self.time, &self.reserve) {
            (Some(_), Some(_)) => q
                .set("time", "__placeholder_time__")
                .set("reserve", "__placeholder__"),
            (Some(_), None) => q.set("time", "__placeholder__"),
            (None, Some(_)) => q.set("reserve", "__placeholder__"),
            (None, None) => q,
        };

        let q = q.so_that(
            "id".equals("__placeholder__")
                .and("kind".equals("__placeholder__")),
        );

        let (sql, _bindings) = Postgres::build(q);

        let query = sqlx::query(&sql);

        let query = match &self.time {
            Some(t) => {
                let t: PgRange<DateTime<Utc>> = t.into();
                query.bind(t)
            }
            None => query,
        };

        let query = match &self.reserve {
            Some(r) => query.bind(r),
            None => query,
        };

        let query = query.bind(self.id).bind(ClassType::Webinar);

        query.execute(conn).await.map(|done| done.rows_affected())
    }
}

pub struct WebinarRecreateQuery {
    id: Uuid,
    time: Time,
    event_room_id: Uuid,
    conference_room_id: Uuid,
}

impl WebinarRecreateQuery {
    pub fn new(id: Uuid, time: Time, event_room_id: Uuid, conference_room_id: Uuid) -> Self {
        Self {
            id,
            time,
            event_room_id,
            conference_room_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        let time: PgRange<DateTime<Utc>> = self.time.into();

        sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET time = $2, event_room_id = $3, conference_room_id = $4, original_event_room_id = NULL, modified_event_room_id = NULL
            WHERE id = $1
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
            self.id,
            time,
            self.event_room_id,
            self.conference_room_id,
        )
        .fetch_one(conn)
        .await
    }
}
