use std::ops::Bound;

use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use sqlx::postgres::{types::PgRange, PgConnection};
use uuid::Uuid;

use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub type BoundedDateTimeTuple = (Bound<DateTime<Utc>>, Bound<DateTime<Utc>>);

#[derive(Clone, Debug, sqlx::Type)]
#[sqlx(rename = "class_type", rename_all = "lowercase")]
pub enum ClassType {
    Webinar,
}

#[derive(Clone, Debug, Serialize)]
pub struct Object {
    id: Uuid,
    title: String,
    #[serde(skip)]
    kind: ClassType,
    scope: String,
    #[serde(with = "serde::time")]
    time: Time,
    audience: String,
    #[serde(with = "ts_seconds")]
    created_at: DateTime<Utc>,
    tags: Option<JsonValue>,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
    preserve_history: bool,
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

    pub fn conference_room_id(&self) -> Uuid {
        self.conference_room_id
    }

    pub fn audience(&self) -> String {
        self.audience.clone()
    }

    pub fn tags(&self) -> Option<JsonValue> {
        self.tags.clone()
    }

    pub fn original_event_room_id(&self) -> Option<Uuid> {
        self.original_event_room_id
    }

    pub fn modified_event_room_id(&self) -> Option<Uuid> {
        self.modified_event_room_id
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, sqlx::Type)]
#[sqlx(transparent)]
#[serde(from = "BoundedDateTimeTuple")]
#[serde(into = "BoundedDateTimeTuple")]
pub struct Time(PgRange<DateTime<Utc>>);

impl From<Time> for BoundedDateTimeTuple {
    fn from(time: Time) -> BoundedDateTimeTuple {
        (time.0.start, time.0.end)
    }
}

impl From<BoundedDateTimeTuple> for Time {
    fn from(time: BoundedDateTimeTuple) -> Time {
        Self(PgRange::from(time))
    }
}

impl Into<PgRange<DateTime<Utc>>> for Time {
    fn into(self) -> PgRange<DateTime<Utc>> {
        self.0
    }
}

pub struct WebinarReadQuery {
    id: Uuid,
}

impl WebinarReadQuery {
    pub fn new(id: Uuid) -> Self {
        Self { id }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT
                id,
                title,
                kind AS "kind!: ClassType",
                audience,
                scope,
                time AS "time!: Time",
                tags,
                conference_room_id,
                event_room_id,
                original_event_room_id,
                modified_event_room_id,
                created_at,
                preserve_history
            FROM class
            WHERE kind = 'webinar' AND id = $1
            "#,
            self.id
        )
        .fetch_optional(conn)
        .await
    }
}

pub struct WebinarReadByScopeQuery {
    scope: String,
}

impl WebinarReadByScopeQuery {
    pub fn new(scope: String) -> Self {
        Self { scope }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT
                id,
                title,
                kind AS "kind!: ClassType",
                audience,
                scope,
                time AS "time!: Time",
                tags,
                conference_room_id,
                event_room_id,
                original_event_room_id,
                modified_event_room_id,
                created_at,
                preserve_history
            FROM class
            WHERE kind = 'webinar' AND scope = $1
            "#,
            self.scope
        )
        .fetch_optional(conn)
        .await
    }
}

pub struct WebinarInsertQuery {
    title: String,
    scope: String,
    audience: String,
    time: Time,
    tags: Option<JsonValue>,
    preserve_history: bool,
    conference_room_id: Uuid,
    event_room_id: Uuid,
}

impl WebinarInsertQuery {
    pub fn new(
        title: String,
        scope: String,
        audience: String,
        time: Time,
        conference_room_id: Uuid,
        event_room_id: Uuid,
    ) -> Self {
        Self {
            title,
            scope,
            audience,
            time,
            tags: None,
            preserve_history: true,
            conference_room_id,
            event_room_id,
        }
    }

    pub(crate) fn tags(self, tags: JsonValue) -> Self {
        Self {
            tags: Some(tags),
            ..self
        }
    }

    pub(crate) fn preserve_history(self, preserve_history: bool) -> Self {
        Self {
            preserve_history,
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        let time: PgRange<DateTime<Utc>> = self.time.into();

        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO class (title, scope, audience, time, tags, preserve_history, kind, conference_room_id, event_room_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7::class_type, $8, $9)
            RETURNING
                id,
                title,
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
                modified_event_room_id
            "#,
            self.title,
            self.scope,
            self.audience,
            time,
            self.tags,
            self.preserve_history,
            ClassType::Webinar as ClassType,
            self.conference_room_id,
            self.event_room_id
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
                title,
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
                modified_event_room_id
            "#,
            self.id,
            time,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct WebinarUpdateQuery {
    id: Uuid,
    original_event_room_id: Uuid,
    modified_event_room_id: Uuid,
}

impl WebinarUpdateQuery {
    pub fn new(id: Uuid, original_event_room_id: Uuid, modified_event_room_id: Uuid) -> Self {
        Self {
            id,
            original_event_room_id,
            modified_event_room_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET original_event_room_id = $2,
                modified_event_room_id = $3
            WHERE id = $1
            RETURNING
                id,
                title,
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
                modified_event_room_id
            "#,
            self.id,
            self.original_event_room_id,
            self.modified_event_room_id,
        )
        .fetch_one(conn)
        .await
    }
}

pub(crate) mod serde {
    pub(crate) mod time {
        use super::super::Time;
        use crate::serde::ts_seconds_bound_tuple;
        use serde::{de, ser};

        pub(crate) fn serialize<S>(value: &Time, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: ser::Serializer,
        {
            ts_seconds_bound_tuple::serialize(&value.to_owned().into(), serializer)
        }

        pub(crate) fn deserialize<'de, D>(d: D) -> Result<Time, D::Error>
        where
            D: de::Deserializer<'de>,
        {
            let time = ts_seconds_bound_tuple::deserialize(d)?;
            Ok(Time::from(time))
        }
    }
}
