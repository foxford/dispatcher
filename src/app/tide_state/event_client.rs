use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_derive::Serialize;
use serde_json::Value as JsonValue;
use svc_agent::{
    error::Error as AgentError,
    mqtt::{
        OutgoingMessage, OutgoingRequest, OutgoingRequestProperties, ShortTermTimingProperties,
        SubscriptionTopic,
    },
    request::Dispatcher,
    AccountId, AgentId, Subscription,
};
use uuid::Uuid;

use super::{generate_correlation_data, ClientError};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::recording::Object as Recording;

const EVENT_API_VERSION: &str = "v1";

#[async_trait]
pub trait EventClient: Sync + Send {
    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        preserve_history: Option<bool>,
        tags: Option<JsonValue>,
    ) -> Result<Uuid, ClientError>;

    async fn update_room(&self, id: Uuid, time: BoundedDateTimeTuple) -> Result<(), ClientError>;

    async fn adjust_room(&self, recording: &Recording, offset: i64) -> Result<(), ClientError>;
}

pub struct MqttEventClient {
    me: AgentId,
    event_account_id: AccountId,
    dispatcher: Arc<Dispatcher>,
    timeout: Option<Duration>,
}

impl MqttEventClient {
    pub fn new(
        me: AgentId,
        event_account_id: AccountId,
        dispatcher: Arc<Dispatcher>,
        timeout: Option<Duration>,
    ) -> Self {
        Self {
            me,
            event_account_id,
            dispatcher,
            timeout,
        }
    }
}

#[derive(Serialize)]
struct EventRoomPayload {
    audience: String,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
    preserve_history: Option<bool>,
    tags: Option<JsonValue>,
}

#[derive(Serialize)]
struct EventRoomUpdatePayload {
    id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
}

#[derive(Serialize)]
struct EventAdjustPayload {
    id: Uuid,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    started_at: DateTime<Utc>,
    #[serde(with = "crate::db::recording::serde::segments")]
    segments: crate::db::recording::Segments,
    offset: i64,
}

#[async_trait]
impl EventClient for MqttEventClient {
    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        preserve_history: Option<bool>,
        tags: Option<JsonValue>,
    ) -> Result<Uuid, ClientError> {
        let me = self.me.clone();
        let event = self.event_account_id.clone();
        let dispatcher = self.dispatcher.clone();

        let response_topic =
            match Subscription::unicast_responses_from(&event).subscription_topic(&me, "v2") {
                Err(e) => {
                    let e = AgentError::new(&e.to_string()).into();
                    return Err(e);
                }
                Ok(topic) => topic,
            };

        let reqp = OutgoingRequestProperties::new(
            "room.create",
            &response_topic,
            &generate_correlation_data(),
            ShortTermTimingProperties::new(Utc::now()),
        );

        let payload = EventRoomPayload {
            time,
            audience,
            tags,
            preserve_history,
        };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &event, EVENT_API_VERSION)
        {
            msg
        } else {
            unreachable!()
        };

        let request = dispatcher.request::<_, JsonValue>(msg);
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

    async fn update_room(&self, id: Uuid, time: BoundedDateTimeTuple) -> Result<(), ClientError> {
        let me = self.me.clone();
        let event = self.event_account_id.clone();
        let dispatcher = self.dispatcher.clone();

        let response_topic =
            match Subscription::unicast_responses_from(&event).subscription_topic(&me, "v2") {
                Err(e) => {
                    let e = AgentError::new(&e.to_string()).into();
                    return Err(e);
                }
                Ok(topic) => topic,
            };

        let reqp = OutgoingRequestProperties::new(
            "room.create",
            &response_topic,
            &generate_correlation_data(),
            ShortTermTimingProperties::new(Utc::now()),
        );

        let payload = EventRoomUpdatePayload { id, time };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &event, EVENT_API_VERSION)
        {
            msg
        } else {
            unreachable!()
        };

        let request = dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            async_std::future::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::TimeoutError)?
        } else {
            request.await
        };
        let payload = payload_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;
        match payload.properties().status().as_u16() {
            200 => Ok(()),
            _ => Err(ClientError::PayloadError(
                "Event room update returned non 200 status".into(),
            )),
        }
    }

    async fn adjust_room(&self, recording: &Recording, offset: i64) -> Result<(), ClientError> {
        let me = self.me.clone();
        let event = self.event_account_id.clone();
        let dispatcher = self.dispatcher.clone();

        let response_topic =
            match Subscription::unicast_responses_from(&event).subscription_topic(&me, "v2") {
                Err(e) => {
                    let e = AgentError::new(&e.to_string()).into();
                    return Err(e);
                }
                Ok(topic) => topic,
            };

        let reqp = OutgoingRequestProperties::new(
            "room.adjust",
            &response_topic,
            &generate_correlation_data(),
            ShortTermTimingProperties::new(Utc::now()),
        );

        let payload = EventAdjustPayload {
            id: recording.class_id(),
            started_at: recording.started_at(),
            segments: recording.segments(),
            offset,
        };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &event, EVENT_API_VERSION)
        {
            msg
        } else {
            unreachable!()
        };

        let request = dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            async_std::future::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::TimeoutError)?
        } else {
            request.await
        };

        let payload = payload_result.map_err(|e| ClientError::PayloadError(e.to_string()))?;

        match payload.properties().status().as_u16() {
            202 => Ok(()),
            status => {
                let e = format!("Wrong status, expected 202, got {:?}", status);
                Err(ClientError::PayloadError(e))
            }
        }
    }
}
