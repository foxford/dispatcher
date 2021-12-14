use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(test)]
use mockall::{automock, predicate::*};
use serde_json::Value as JsonValue;
use uuid::Uuid;

pub use self::types::*;
use super::{generate_correlation_data, ClientError};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::recording::Segments;

const MAX_EVENT_LIST_PAGES: u64 = 10;
const EVENT_LIST_LIMIT: u64 = 100;

////////////////////////////////////////////////////////////////////////////////

#[cfg_attr(test, automock)]
#[async_trait]
pub trait EventClient: Sync + Send {
    async fn read_room(&self, id: Uuid) -> Result<EventRoomResponse, ClientError>;

    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        preserve_history: Option<bool>,
        tags: Option<JsonValue>,
        classroom_id: Option<Uuid>,
    ) -> Result<Uuid, ClientError>;

    async fn update_room(&self, id: Uuid, update: RoomUpdate) -> Result<(), ClientError>;

    async fn update_locked_types(
        &self,
        id: Uuid,
        locked_types: LockedTypes,
    ) -> Result<(), ClientError>;

    async fn adjust_room(
        &self,
        event_room_id: Uuid,
        started_at: DateTime<Utc>,
        segments: Segments,
        offset: i64,
    ) -> Result<(), ClientError>;

    async fn commit_edition(&self, edition_id: Uuid, offset: i64) -> Result<(), ClientError>;

    async fn create_event(&self, payload: JsonValue) -> Result<(), ClientError>;
    async fn list_events(&self, room_id: Uuid, kind: &str) -> Result<Vec<Event>, ClientError>;
    async fn dump_room(&self, event_room_id: Uuid) -> Result<(), ClientError>;

    async fn lock_chat(&self, room_id: Uuid) -> Result<(), ClientError> {
        let payload = EventPayload {
            room_id,
            kind: "chat_disabled",
            set: "chat_disabled",
            data: serde_json::json!({"value": true}),
            label: None,
        };

        let payload = serde_json::to_value(&payload).unwrap();

        let f1 = self.create_event(payload);
        let f2 = self.update_locked_types(room_id, LockedTypes { message: true });
        tokio::try_join!(f1, f2).map(|_| ())
    }

    async fn create_whiteboard(&self, room_id: Uuid) -> Result<(), ClientError> {
        let payload = EventPayload {
            room_id,
            kind: "document",
            set: "document",
            data: serde_json::json!({"title":"whiteboard","page":1,"published":true,"url":"about:whiteboard"}),
            label: Some(Uuid::new_v4()),
        };

        let payload = serde_json::to_value(&payload).unwrap();

        self.create_event(payload).await?;
        Ok(())
    }
}

pub use client::MqttEventClient;
pub use tower_client::TowerClient;

mod client;
mod layer;
mod mqtt_client;
mod tower_client;
mod types;
