use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::postgres::{types::PgRange, PgConnection};
use uuid::Uuid;

use super::{AgentId, ClassType, Object, Time, WrongKind};
#[cfg(test)]
use super::{GenericReadQuery, WebinarType};

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Webinar {
    id: Uuid,
    #[serde(skip)]
    #[allow(dead_code)]
    kind: ClassType,
    scope: String,
    #[serde(with = "super::serde::time")]
    time: Time,
    audience: String,
    #[serde(with = "ts_seconds")]
    created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<JsonValue>,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_event_room_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_event_room_id: Option<Uuid>,
    preserve_history: bool,
    reserve: Option<i32>,
    room_events_uri: Option<String>,
}

impl std::convert::TryFrom<Object> for Webinar {
    type Error = WrongKind;

    fn try_from(value: Object) -> Result<Self, Self::Error> {
        match value.kind() {
            ClassType::Webinar => Ok(Self {
                conference_room_id: value.conference_room_id,
                id: value.id,
                kind: value.kind,
                scope: value.scope,
                time: value.time,
                audience: value.audience,
                event_room_id: value.event_room_id,
                original_event_room_id: value.original_event_room_id,
                modified_event_room_id: value.modified_event_room_id,
                created_at: value.created_at,
                tags: value.tags,
                properties: value.properties,
                preserve_history: value.preserve_history,
                reserve: value.reserve,
                room_events_uri: value.room_events_uri,
            }),
            _ => Err(WrongKind::new(&value, ClassType::Webinar)),
        }
    }
}

#[cfg(test)]
pub type WebinarReadQuery = GenericReadQuery<WebinarType>;

pub struct WebinarInsertQuery {
    scope: String,
    audience: String,
    time: Time,
    tags: Option<JsonValue>,
    properties: Option<JsonValue>,
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
            properties: None,
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

    #[cfg(test)]
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
                original_event_room_id, modified_event_room_id, reserve, room_events_uri,
                properties
            )
            VALUES ($1, $2, $3, $4, $5, $6::class_type, $7, $8, $9, $10, $11, $12, $13)
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                time AS "time!: Time",
                tags,
                properties,
                preserve_history,
                created_at,
                event_room_id AS "event_room_id!: Uuid",
                conference_room_id AS "conference_room_id!: Uuid",
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri,
                host AS "host: AgentId",
                timed_out
            "#,
            self.scope,
            self.audience,
            time,
            self.tags,
            self.preserve_history,
            ClassType::Webinar as ClassType,
            Some(self.conference_room_id),
            self.event_room_id,
            self.original_event_room_id,
            self.modified_event_room_id,
            self.reserve,
            self.room_events_uri,
            self.properties,
        )
        .fetch_one(conn)
        .await
    }
}
