use chrono::{DateTime, Utc};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use svc_agent::AgentId;
use uuid::Uuid;

use crate::db::class::BoundedDateTimeTuple;
use crate::db::recording::Segments;

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    id: Uuid,
    room_id: Uuid,
    set: String,
    label: Option<String>,
    attribute: Option<String>,
    #[serde(flatten)]
    data: EventData,
    occurred_at: u64,
    original_occurred_at: u64,
    created_by: AgentId,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    created_at: DateTime<Utc>,
}

impl Event {
    pub fn data(&self) -> &EventData {
        &self.data
    }

    pub fn occurred_at(&self) -> u64 {
        self.occurred_at
    }

    pub fn room_id(&self) -> Uuid {
        self.room_id
    }

    pub fn id(&self) -> Uuid {
        self.id
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "lowercase")]
pub enum EventData {
    Pin(PinEventData),
    Host(HostEventData),
}

#[derive(Clone, Debug, Deserialize)]
pub struct PinEventData {
    agent_id: Option<AgentId>,
}

impl PinEventData {
    #[cfg(test)]
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id: Some(agent_id),
        }
    }

    #[cfg(test)]
    pub fn null() -> Self {
        Self { agent_id: None }
    }

    pub fn agent_id(&self) -> Option<&AgentId> {
        self.agent_id.as_ref()
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct HostEventData {
    agent_id: AgentId,
}

impl HostEventData {
    #[cfg(test)]
    pub fn new(agent_id: AgentId) -> Self {
        Self { agent_id }
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
}

#[derive(Deserialize)]
pub struct RoomAdjust {
    room_id: Uuid,
    #[serde(flatten)]
    result: RoomAdjustResult,
}

impl RoomAdjust {
    pub fn room_id(&self) -> Uuid {
        self.room_id
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum RoomAdjustResult {
    Success {
        original_room_id: Uuid,
        modified_room_id: Uuid,
        #[serde(with = "crate::db::recording::serde::segments")]
        modified_segments: Segments,
        #[serde(with = "crate::db::recording::serde::segments")]
        cut_original_segments: Segments,
    },
    Error {
        error: JsonValue,
    },
}

impl From<RoomAdjust> for RoomAdjustResult {
    fn from(room_adjust: RoomAdjust) -> Self {
        room_adjust.result
    }
}

pub struct RoomUpdate {
    pub time: Option<BoundedDateTimeTuple>,
    pub classroom_id: Option<Uuid>,
}

impl RoomUpdate {
    pub fn is_empty_update(&self) -> bool {
        matches!(
            self,
            RoomUpdate {
                classroom_id: None,
                time: None
            }
        )
    }
}

////////////////////////////////////////////////////////////////////////////////

pub trait MqttRequest: Clone + serde::Serialize {
    type Response: serde::de::DeserializeOwned;

    fn method(&self) -> &'static str;
    fn payload(&self) -> JsonValue {
        serde_json::to_value(&self).unwrap()
    }
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Debug, Serialize)]
pub struct EventRoomCreatePayload {
    pub audience: String,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    pub time: BoundedDateTimeTuple,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserve_history: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classroom_id: Option<Uuid>,
}

impl MqttRequest for EventRoomCreatePayload {
    type Response = EventRoomResponse;

    fn method(&self) -> &'static str {
        "room.create"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EventRoomUpdatePayload {
    pub id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_option_bound_tuple")]
    pub time: Option<BoundedDateTimeTuple>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classroom_id: Option<Uuid>,
}

impl MqttRequest for EventRoomUpdatePayload {
    type Response = EventRoomResponse;

    fn method(&self) -> &'static str {
        "room.update"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EventAdjustPayload {
    pub id: Uuid,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub started_at: DateTime<Utc>,
    #[serde(with = "crate::db::recording::serde::segments")]
    pub segments: Segments,
    pub offset: i64,
}

#[derive(Deserialize)]
pub struct EmptyResponse {}

impl MqttRequest for EventAdjustPayload {
    type Response = EmptyResponse;

    fn method(&self) -> &'static str {
        "room.adjust"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EventCommitPayload {
    pub id: Uuid,
    pub offset: i64,
}

impl MqttRequest for EventCommitPayload {
    type Response = EmptyResponse;

    fn method(&self) -> &'static str {
        "edition.commit"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EventDumpEventsPayload {
    pub id: Uuid,
}

impl MqttRequest for EventDumpEventsPayload {
    type Response = EmptyResponse;

    fn method(&self) -> &'static str {
        "room.dump_events"
    }
}

#[derive(Debug, Serialize)]
pub struct EventPayload<'a> {
    pub room_id: Uuid,
    #[serde(rename(serialize = "type"))]
    pub kind: &'a str,
    pub set: &'a str,
    pub data: JsonValue,
    pub label: &'a str,
}

#[derive(Clone, Debug, Serialize)]
pub struct EventRoomReadPayload {
    pub id: Uuid,
}

impl MqttRequest for EventRoomReadPayload {
    type Response = EventRoomResponse;

    fn method(&self) -> &'static str {
        "room.read"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EventListPayload {
    pub room_id: Uuid,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_occurred_at: Option<u64>,
    pub limit: u64,
}

impl MqttRequest for EventListPayload {
    type Response = Vec<Event>;

    fn method(&self) -> &'static str {
        "event.list"
    }
}

#[derive(Debug, Deserialize)]
pub struct EventRoomResponse {
    pub id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    pub time: BoundedDateTimeTuple,
    pub tags: Option<JsonValue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LockedTypes {
    pub message: bool,
    pub reaction: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EventRoomLockedTypesPayload {
    pub id: Uuid,
    pub locked_types: LockedTypes,
}

impl MqttRequest for EventRoomLockedTypesPayload {
    type Response = EventRoomResponse;

    fn method(&self) -> &'static str {
        "room.locked_types"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EventCreateEventPayload {
    #[serde(flatten)]
    pub json: JsonValue,
}

impl MqttRequest for EventCreateEventPayload {
    type Response = EmptyResponse;

    fn method(&self) -> &'static str {
        "event.create"
    }
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;
    use crate::test_helpers::prelude::*;

    #[derive(Debug, Default)]
    pub struct EventBuilder {
        room_id: Option<Uuid>,
        set: Option<String>,
        data: Option<EventData>,
        occurred_at: Option<u64>,
    }

    impl EventBuilder {
        pub fn new() -> Self {
            Default::default()
        }

        pub fn room_id(self, room_id: Uuid) -> Self {
            Self {
                room_id: Some(room_id),
                ..self
            }
        }

        pub fn set(self, set: String) -> Self {
            Self {
                set: Some(set),
                ..self
            }
        }

        pub fn data(self, data: EventData) -> Self {
            Self {
                data: Some(data),
                ..self
            }
        }

        pub fn occurred_at(self, occurred_at: u64) -> Self {
            Self {
                occurred_at: Some(occurred_at),
                ..self
            }
        }

        pub fn build(self) -> Event {
            let created_by = TestAgent::new("web", "admin", USR_AUDIENCE);

            Event {
                id: Uuid::new_v4(),
                room_id: self.room_id.unwrap(),
                set: self.set.unwrap(),
                label: None,
                attribute: None,
                data: self.data.unwrap(),
                occurred_at: self.occurred_at.unwrap(),
                original_occurred_at: self.occurred_at.unwrap(),
                created_by: created_by.agent_id().to_owned(),
                created_at: Utc::now(),
            }
        }
    }

    #[test]
    fn parse_pin_data() {
        serde_json::from_str::<PinEventData>(r#"{"agent_id": null}"#)
            .expect("Failed to parse pin data");
    }
}
