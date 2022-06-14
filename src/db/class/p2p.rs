use std::ops::Bound;

use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::postgres::{types::PgRange, PgConnection};
use uuid::Uuid;

use super::{AgentId, ClassType, Object, ClassProperties, Time};

pub struct P2PInsertQuery {
    scope: String,
    audience: String,
    tags: Option<JsonValue>,
    properties: Option<JsonValue>,
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
            properties: None,
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

    pub fn properties(self, properties: ClassProperties) -> Self {
        Self {
            properties: Some(JsonValue::Object(properties)),
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
                conference_room_id, event_room_id, properties
            )
            VALUES ($1, $2, $3, $4, $5, $6::class_type, $7, $8, $9)
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
            false,
            ClassType::P2P as ClassType,
            self.conference_room_id,
            self.event_room_id,
            self.properties,
        )
        .fetch_one(conn)
        .await
    }
}

#[cfg(test)]
pub type P2PReadQuery = super::GenericReadQuery<super::P2PType>;
