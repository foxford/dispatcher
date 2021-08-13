use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::serde::ts_milliseconds;
use chrono::{DateTime, Utc};
#[cfg(test)]
use mockall::{automock, predicate::*};
use serde_derive::{Deserialize, Serialize};
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

pub struct RoomUpdate {
    pub time: Option<BoundedDateTimeTuple>,
    pub reserve: Option<i32>,
    pub classroom_id: Option<Uuid>,
    pub host: Option<AgentId>,
}

impl RoomUpdate {
    pub fn is_empty_update(&self) -> bool {
        matches!(
            self,
            RoomUpdate {
                classroom_id: None,
                reserve: None,
                host: None,
                time: None
            }
        )
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ConfigSnapshot {
    pub send_video: Option<bool>,
    pub send_audio: Option<bool>,
    pub rtc_id: Uuid,
    #[serde(with = "ts_milliseconds")]
    pub created_at: DateTime<Utc>,
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait ConferenceClient: Sync + Send {
    async fn read_room(&self, id: Uuid) -> Result<ConferenceRoomResponse, ClientError>;

    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        rtc_sharing_policy: Option<String>,
        reserve: Option<i32>,
        tags: Option<JsonValue>,
    ) -> Result<Uuid, ClientError>;

    async fn update_room(&self, id: Uuid, update: RoomUpdate) -> Result<(), ClientError>;

    async fn read_config_snapshots(&self, id: Uuid) -> Result<Vec<ConfigSnapshot>, ClientError>;
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

    fn response_topic(&self) -> Result<String, ClientError> {
        let me = self.me.clone();

        Subscription::unicast_responses_from(&self.conference_account_id)
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

#[derive(Serialize)]
struct ConferenceRoomPayload {
    audience: String,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: BoundedDateTimeTuple,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtc_sharing_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserve: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
}

#[derive(Serialize)]
struct ConferenceRoomUpdatePayload {
    id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classroom_id: Option<Uuid>,
}

#[derive(Serialize)]
struct ConferenceRoomReadPayload {
    id: Uuid,
}

#[derive(Serialize)]
struct ConferenceWriterConfigSnapshotReadPayload {
    room_id: Uuid,
}

#[derive(Deserialize)]
pub struct ConferenceRoomResponse {
    pub id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    pub time: BoundedDateTimeTuple,
}

#[async_trait]
impl ConferenceClient for MqttConferenceClient {
    async fn read_room(&self, id: Uuid) -> Result<ConferenceRoomResponse, ClientError> {
        let reqp = self.build_reqp("room.read")?;

        let payload = ConferenceRoomReadPayload { id };
        let msg = if let OutgoingMessage::Request(msg) = OutgoingRequest::multicast(
            payload,
            reqp,
            &self.conference_account_id,
            &self.api_version,
        ) {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, ConferenceRoomResponse>(msg);
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
        rtc_sharing_policy: Option<String>,
        reserve: Option<i32>,
        tags: Option<JsonValue>,
    ) -> Result<Uuid, ClientError> {
        let reqp = self.build_reqp("room.create")?;

        let payload = ConferenceRoomPayload {
            audience,
            time,
            rtc_sharing_policy,
            reserve,
            tags,
        };
        let msg = if let OutgoingMessage::Request(msg) = OutgoingRequest::multicast(
            payload,
            reqp,
            &self.conference_account_id,
            &self.api_version,
        ) {
            msg
        } else {
            return Err(ClientError::AgentError(AgentError::new(
                "this is actually unreachable",
            )));
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

        let payload = ConferenceRoomUpdatePayload {
            id,
            time: update.time,
            classroom_id: update.classroom_id,
        };
        let msg = if let OutgoingMessage::Request(msg) = OutgoingRequest::multicast(
            payload,
            reqp,
            &self.conference_account_id,
            &self.api_version,
        ) {
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
        match payload.properties().status().as_u16() {
            200 => Ok(()),
            _ => Err(ClientError::PayloadError(
                "Conference room update returned non 200 status".into(),
            )),
        }
    }

    async fn read_config_snapshots(
        &self,
        room_id: Uuid,
    ) -> Result<Vec<ConfigSnapshot>, ClientError> {
        let reqp = self.build_reqp("writer_config_snapshot.read")?;

        let payload = ConferenceWriterConfigSnapshotReadPayload { room_id };
        let msg = if let OutgoingMessage::Request(msg) = OutgoingRequest::multicast(
            payload,
            reqp,
            &self.conference_account_id,
            &self.api_version,
        ) {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, Vec<ConfigSnapshot>>(msg);
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
}
