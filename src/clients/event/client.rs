use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

use super::types::*;
use super::{generate_correlation_data, ClientError, EventClient};
use super::{EVENT_LIST_LIMIT, MAX_EVENT_LIST_PAGES};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::recording::Segments;

pub struct MqttEventClient {
    me: AgentId,
    event_account_id: AccountId,
    dispatcher: Arc<Dispatcher>,
    timeout: Option<Duration>,
    api_version: String,
}

impl MqttEventClient {
    #[allow(dead_code)]
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
        let payload_result = request.await;
        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;

        Ok(payload.extract_payload())
    }

    async fn create_room(
        &self,
        time: BoundedDateTimeTuple,
        audience: String,
        preserve_history: Option<bool>,
        tags: Option<JsonValue>,
        classroom_id: Option<Uuid>,
    ) -> Result<Uuid, ClientError> {
        let reqp = self.build_reqp("room.create")?;

        let payload = EventRoomCreatePayload {
            audience,
            time,
            preserve_history,
            tags,
            classroom_id,
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
            tokio::time::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::Timeout)?
        } else {
            request.await
        };
        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;

        let data = payload.extract_payload();

        let uuid_result = match data.get("id").and_then(|v| v.as_str()) {
            Some(id) => Uuid::from_str(id).map_err(|e| ClientError::Payload(e.to_string())),
            None => Err(ClientError::Payload(
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
            tokio::time::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::Timeout)?
        } else {
            request.await
        };
        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;
        match payload.properties().status() {
            ResponseStatus::OK => Ok(()),
            _ => Err(ClientError::Payload(
                "Event room update returned non 200 status".into(),
            )),
        }
    }

    async fn update_locked_types(
        &self,
        id: Uuid,
        locked_types: LockedTypes,
    ) -> Result<(), ClientError> {
        let reqp = self.build_reqp("room.locked_types")?;
        let payload = EventRoomLockedTypesPayload { id, locked_types };

        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            tokio::time::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::Timeout)?
        } else {
            request.await
        };
        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;
        match payload.properties().status() {
            ResponseStatus::OK => Ok(()),
            _ => Err(ClientError::Payload(
                "Event update_locked_types returned non 200 status".into(),
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
            tokio::time::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::Timeout)?
        } else {
            request.await
        };

        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;

        match payload.properties().status() {
            ResponseStatus::ACCEPTED => Ok(()),
            status => {
                let e = format!("Wrong status, expected 202, got {:?}", status);
                Err(ClientError::Payload(e))
            }
        }
    }

    async fn commit_edition(&self, edition_id: Uuid, offset: i64) -> Result<(), ClientError> {
        let reqp = self.build_reqp("edition.commit")?;

        let payload = EventCommitPayload {
            id: edition_id,
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
            tokio::time::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::Timeout)?
        } else {
            request.await
        };

        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;

        match payload.properties().status() {
            ResponseStatus::ACCEPTED => Ok(()),
            status => {
                let e = format!("Wrong status, expected 202, got {:?}", status);
                Err(ClientError::Payload(e))
            }
        }
    }

    async fn create_event(&self, payload: JsonValue) -> Result<(), ClientError> {
        let reqp = self.build_reqp("event.create")?;

        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            tokio::time::timeout(dur, request)
                .await
                .map_err(|_e| ClientError::Timeout)?
        } else {
            request.await
        };

        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;

        match payload.properties().status() {
            ResponseStatus::CREATED => Ok(()),
            status => {
                let e = format!(
                    "Wrong status, expected 201, got {:?}, payload = {:?}",
                    status,
                    payload.payload()
                );
                Err(ClientError::Payload(e))
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
                tokio::time::timeout(dur, request)
                    .await
                    .map_err(|_e| ClientError::Timeout)?
            } else {
                request.await
            };

            let response = response_result.map_err(|e| ClientError::Payload(e.to_string()))?;
            let status = response.properties().status();

            if status != ResponseStatus::OK {
                let e = format!("Wrong status, expected 200, got {:?}", status);
                return Err(ClientError::Payload(e));
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

    async fn dump_room(&self, room_id: Uuid) -> Result<(), ClientError> {
        let reqp = self.build_reqp("room.dump_events")?;
        let payload = EventDumpEventsPayload { id: room_id };
        let msg = if let OutgoingMessage::Request(msg) =
            OutgoingRequest::multicast(payload, reqp, &self.event_account_id, &self.api_version)
        {
            msg
        } else {
            unreachable!()
        };

        let request = self.dispatcher.request::<_, JsonValue>(msg);
        let payload_result = if let Some(dur) = self.timeout {
            tokio::time::timeout(dur, request)
                .await
                .map_err(|_| ClientError::Timeout)?
        } else {
            request.await
        };

        let payload = payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;

        match payload.properties().status() {
            ResponseStatus::ACCEPTED => Ok(()),
            status => {
                let e = format!("Wrong status, expected 202, got {:?}", status);
                Err(ClientError::Payload(e))
            }
        }
    }
}
