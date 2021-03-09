use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
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

#[async_trait]
pub trait ConferenceClient: Sync + Send {
    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        backend: Option<String>,
        reserve: Option<i32>,
        tags: Option<JsonValue>,
    ) -> Result<Uuid, ClientError>;

    async fn update_room(&self, id: Uuid, time: BoundedDateTimeTuple) -> Result<(), ClientError>;
}

pub struct MqttConferenceClient {
    me: AgentId,
    conference_account_id: AccountId,
    dispatcher: Arc<Dispatcher>,
    timeout: Option<Duration>,
    api_version: String,
}

impl MqttConferenceClient {
    pub fn new(
        me: AgentId,
        conference_account_id: AccountId,
        dispatcher: Arc<Dispatcher>,
        timeout: Option<Duration>,
        api_version: &str,
    ) -> Self {
        Self {
            me,
            conference_account_id,
            dispatcher,
            timeout,
            api_version: api_version.to_string(),
        }
    }
}

#[derive(Serialize)]
struct ConferenceRoomPayload {
    audience: String,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
    backend: Option<String>,
    reserve: Option<i32>,
    tags: Option<JsonValue>,
}

#[derive(Serialize)]
struct ConferenceRoomUpdatePayload {
    id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
}

#[async_trait]
impl ConferenceClient for MqttConferenceClient {
    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        backend: Option<String>,
        reserve: Option<i32>,
        tags: Option<JsonValue>,
    ) -> Result<Uuid, ClientError> {
        let me = self.me.clone();
        let conference = self.conference_account_id.clone();
        let dispatcher = self.dispatcher.clone();

        let response_topic =
            match Subscription::unicast_responses_from(&conference).subscription_topic(&me, "v2") {
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

        let payload = ConferenceRoomPayload {
            time,
            audience,
            backend,
            reserve,
            tags,
        };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &conference, &self.api_version)
        {
            msg
        } else {
            return Err(ClientError::AgentError(AgentError::new(
                "this is actually unreachable",
            )));
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
        let conference = self.conference_account_id.clone();
        let dispatcher = self.dispatcher.clone();

        let response_topic =
            match Subscription::unicast_responses_from(&conference).subscription_topic(&me, "v2") {
                Err(e) => {
                    let e = AgentError::new(&e.to_string()).into();
                    return Err(e);
                }
                Ok(topic) => topic,
            };

        let reqp = OutgoingRequestProperties::new(
            "room.update",
            &response_topic,
            &generate_correlation_data(),
            ShortTermTimingProperties::new(Utc::now()),
        );

        let payload = ConferenceRoomUpdatePayload { id, time };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &conference, "v2")
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
                "Conference room update returned non 200 status".into(),
            )),
        }
    }
}
