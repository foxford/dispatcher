use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value as JsonValue;
#[cfg(test)]
use sqlx::postgres::{types::PgRange, PgConnection};
use svc_agent::AgentId;
use uuid::Uuid;

use super::{ClassType, Object, Time, WrongKind};

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Minigroup {
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
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_event_room_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_event_room_id: Option<Uuid>,
    preserve_history: bool,
    reserve: Option<i32>,
    room_events_uri: Option<String>,
    host: Option<AgentId>,
}

impl std::convert::TryFrom<Object> for Minigroup {
    type Error = WrongKind;

    fn try_from(value: Object) -> Result<Self, Self::Error> {
        match value.kind() {
            ClassType::Minigroup => Ok(Self {
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
                preserve_history: value.preserve_history,
                reserve: value.reserve,
                room_events_uri: value.room_events_uri,
                host: value.host,
            }),
            _ => Err(WrongKind::new(&value, ClassType::Minigroup)),
        }
    }
}

#[cfg(test)]
use super::{ClassProperties, GenericReadQuery, MinigroupType};

#[cfg(test)]
pub type MinigroupReadQuery = GenericReadQuery<MinigroupType>;

#[cfg(test)]
pub struct MinigroupInsertQuery {
    scope: String,
    audience: String,
    time: Time,
    tags: Option<JsonValue>,
    properties: Option<ClassProperties>,
    preserve_history: bool,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
    reserve: Option<i32>,
}

#[cfg(test)]
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
            properties: None,
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

    pub fn properties(self, properties: ClassProperties) -> Self {
        Self {
            properties: Some(properties),
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
                scope, audience, time, tags, preserve_history, kind, conference_room_id,
                event_room_id, original_event_room_id, modified_event_room_id, reserve,
                properties
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
                event_room_id AS "event_room_id!: Uuid",
                conference_room_id AS "conference_room_id!: Uuid",
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri,
                host AS "host: AgentId",
                timed_out,
                properties AS "properties: _",
                original_class_id
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
            self.properties.unwrap_or_default() as ClassProperties,
        )
        .fetch_one(conn)
        .await
    }
}
