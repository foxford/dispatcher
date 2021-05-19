use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(test)]
use mockall::{automock, predicate::*};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use svc_agent::{
    error::Error as AgentError,
    mqtt::{
        OutgoingMessage, OutgoingRequest, OutgoingRequestProperties, ResponseStatus,
        ShortTermTimingProperties, SubscriptionTopic,
    },
    request::Dispatcher,
    AccountId, AgentId, Subscription,
};
use uuid::Uuid;

use super::{generate_correlation_data, ClientError};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::recording::Segments;

const MAX_EVENT_LIST_PAGES: u64 = 10;
const EVENT_LIST_LIMIT: u64 = 100;

////////////////////////////////////////////////////////////////////////////////

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
    agent_id: AgentId,
}

impl PinEventData {
    #[cfg(test)]
    pub fn new(agent_id: AgentId) -> Self {
        Self { agent_id }
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
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
    tags: Option<JsonValue>,
    #[serde(flatten)]
    result: RoomAdjustResult,
}

impl RoomAdjust {
    pub fn tags(&self) -> Option<&JsonValue> {
        self.tags.as_ref()
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
    ) -> Result<Uuid, ClientError>;

    async fn update_room(&self, id: Uuid, update: RoomUpdate) -> Result<(), ClientError>;

    async fn adjust_room(
        &self,
        event_room_id: Uuid,
        started_at: DateTime<Utc>,
        segments: Segments,
        offset: i64,
    ) -> Result<(), ClientError>;

    async fn lock_chat(&self, room_id: Uuid) -> Result<(), ClientError>;
    async fn list_events(&self, room_id: Uuid, kind: &str) -> Result<Vec<Event>, ClientError>;
}

pub struct MqttEventClient {
    me: AgentId,
    event_account_id: AccountId,
    dispatcher: Arc<Dispatcher>,
    timeout: Option<Duration>,
    api_version: String,
}

impl MqttEventClient {
    pub fn new(
        me: AgentId,
        event_account_id: AccountId,
        dispatcher: Arc<Dispatcher>,
        timeout: Option<Duration>,
        api_version: &str,
    ) -> Self {
        Self {
            me,
            event_account_id,
            dispatcher,
            timeout,
            api_version: api_version.to_string(),
        }
    }

    fn response_topic(&self) -> Result<String, ClientError> {
        let me = self.me.clone();

        Subscription::unicast_responses_from(&self.event_account_id)
            .subscription_topic(&me, &self.api_version)
            .map_err(|e| AgentError::new(&e.to_string()).into())
    }

    fn build_reqp(&self, method: &str) -> Result<OutgoingRequestProperties, ClientError> {
        let reqp = OutgoingRequestProperties::new(
            method,
            &self.response_topic()?,
            &generate_correlation_data(),
            ShortTermTimingProperties::new(Utc::now()),
        );

        Ok(reqp)
    }
}

#[derive(Debug, Serialize)]
struct EventRoomPayload {
    audience: String,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
    #[serde(skip_serializing_if = "Option::is_none")]
    preserve_history: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
struct EventRoomUpdatePayload {
    id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classroom_id: Option<Uuid>,
}

#[derive(Serialize)]
struct EventAdjustPayload {
    id: Uuid,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    started_at: DateTime<Utc>,
    #[serde(with = "crate::db::recording::serde::segments")]
    segments: Segments,
    offset: i64,
}

#[derive(Debug, Serialize)]
struct ChatLockPayload {
    room_id: Uuid,
    #[serde(rename(serialize = "type"))]
    kind: &'static str,
    set: &'static str,
    data: JsonValue,
}

#[derive(Debug, Serialize)]
struct EventRoomReadPayload {
    id: Uuid,
}

#[derive(Debug, Serialize)]
struct EventListPayload {
    room_id: Uuid,
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_occurred_at: Option<u64>,
    limit: u64,
}

#[derive(Debug, Deserialize)]
pub struct EventRoomResponse {
    pub id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    pub time: BoundedDateTimeTuple,
    pub tags: Option<JsonValue>,
}

#[async_trait]
impl EventClient for MqttEventClient {
    async fn read_room(&self, id: Uuid) -> Result<EventRoomResponse, ClientError> {
        let reqp = self.build_reqp("room.read")?;

        let payload = EventRoomReadPayload { id };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, EventRoomResponse>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            async_std::future::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::TimeoutError)?
        } else {
            request.await
        };
        let payload = payload_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;

        Ok(payload.extract_payload())
    }

    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        preserve_history: Option<bool>,
        tags: Option<JsonValue>,
    ) -> Result<Uuid, ClientError> {
        let reqp = self.build_reqp("room.create")?;

        let payload = EventRoomPayload {
            audience,
            time,
            preserve_history,
            tags,
        };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            async_std::future::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::TimeoutError)?
        } else {
            request.await
        };
        let payload = payload_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;

        let data = payload.extract_payload();

        let uuid_result = match data.get("id").and_then(|v| v.as_str()) {
            Some(id) => Uuid::from_str(id).map_err(|e| ClientError::PayloadError(e.to_string())),
            None => Err(ClientError::PayloadError(
                "Missing id field in room.create response".into(),
            )),
        };

        uuid_result
    }

    async fn update_room(&self, id: Uuid, update: RoomUpdate) -> Result<(), ClientError> {
        let reqp = self.build_reqp("room.update")?;
        let payload = EventRoomUpdatePayload {
            id,
            time: update.time,
            classroom_id: update.classroom_id,
        };

        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            async_std::future::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::TimeoutError)?
        } else {
            request.await
        };
        let payload = payload_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;
        match payload.properties().status() {
            ResponseStatus::OK => Ok(()),
            _ => Err(ClientError::PayloadError(
                "Event room update returned non 200 status".into(),
            )),
        }
    }

    async fn adjust_room(
        &self,
        event_room_id: Uuid,
        started_at: DateTime<Utc>,
        segments: Segments,
        offset: i64,
    ) -> Result<(), ClientError> {
        let reqp = self.build_reqp("room.adjust")?;

        let payload = EventAdjustPayload {
            id: event_room_id,
            started_at,
            segments,
            offset,
        };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            async_std::future::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::TimeoutError)?
        } else {
            request.await
        };

        let payload = payload_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;

        match payload.properties().status() {
            ResponseStatus::ACCEPTED => Ok(()),
            status => {
                let e = format!("Wrong status, expected 202, got {:?}", status);
                Err(ClientError::PayloadError(e))
            }
        }
    }

    async fn lock_chat(&self, room_id: Uuid) -> Result<(), ClientError> {
        let reqp = self.build_reqp("event.create")?;

        let payload = ChatLockPayload {
            room_id,
            kind: "chat_disabled",
            set: "chat_disabled",
            data: serde_json::json!({"value": "true"}),
        };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            async_std::future::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::TimeoutError)?
        } else {
            request.await
        };

        let payload = payload_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;

        match payload.properties().status() {
            ResponseStatus::CREATED => Ok(()),
            status => {
                let e = format!("Wrong status, expected 201, got {:?}", status);
                Err(ClientError::PayloadError(e))
            }
        }
    }

    async fn list_events(&self, room_id: Uuid, kind: &str) -> Result<Vec<Event>, ClientError> {
        let mut events = vec![];
        let mut last_occurred_at = None;

        for _ in 0..MAX_EVENT_LIST_PAGES {
            let reqp = self.build_reqp("event.list")?;

            let payload = EventListPayload {
                room_id,
                kind: kind.to_owned(),
                last_occurred_at,
                limit: EVENT_LIST_LIMIT,
            };

            let msg = if let OutgoingMessage::Request(msg) =
                OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
            {
                msg
            } else {
                unreachable!()
            };

            let request = self.dispatcher.request::<_, Vec<Event>>(msg);

            let response_result = if let Some(dur) = self.timeout {
                async_std::future::timeout(dur, request)
                    .await
                    .map_err(|_e| ClientError::TimeoutError)?
            } else {
                request.await
            };

            let response = response_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;
            let status = response.properties().status();

            if status != ResponseStatus::OK {
                let e = format!("Wrong status, expected 200, got {:?}", status);
                return Err(ClientError::PayloadError(e));
            }

            if let Some(last_event) = response.payload().last() {
                last_occurred_at = Some(last_event.occurred_at());

                let mut events_page = response.payload().to_vec();
                events.append(&mut events_page);
            } else {
                break;
            }
        }

        Ok(events)
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
}
