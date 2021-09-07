use serde_json::Value as JsonValue;
use sqlx::postgres::PgConnection;
use uuid::Uuid;

use super::{AgentId, ClassType, Object, Time};

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
            INSERT INTO class (
                scope, audience, tags, event_room_id
            )
            VALUES ($1, $2, $3, $4)
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
                room_events_uri,
                host AS "host: AgentId",
                timed_out
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
