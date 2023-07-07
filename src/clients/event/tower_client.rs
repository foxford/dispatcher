use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use svc_agent::{request::Dispatcher, AccountId, AgentId};
use tower::Service;
use uuid::Uuid;

use super::mqtt_client::{MqttBaseClient, MqttClient};
use super::types::*;
use super::{ClientError, EventClient};
use super::{EVENT_LIST_LIMIT, MAX_EVENT_LIST_PAGES};
use crate::db::recording::Segments;

struct TowerClientInner {
    read_room: MqttClient<EventRoomReadPayload>,
    create_room: MqttClient<EventRoomCreatePayload>,
    update_room: MqttClient<EventRoomUpdatePayload>,
    update_locked_types: MqttClient<EventRoomLockedTypesPayload>,
    adjust_room: MqttClient<EventAdjustPayload>,
    commit_edition: MqttClient<EventCommitPayload>,
    create_event: MqttClient<EventCreateEventPayload>,
    list_events: MqttClient<EventListPayload>,
    dump_events: MqttClient<EventDumpEventsPayload>,
}

impl TowerClientInner {
    fn new(timeout: Option<Duration>, svc: MqttBaseClient) -> Self {
        Self {
            read_room: MqttClient::new(timeout, svc.clone()),
            create_room: MqttClient::new(timeout, svc.clone()),
            update_room: MqttClient::new(timeout, svc.clone()),
            update_locked_types: MqttClient::new(timeout, svc.clone()),
            adjust_room: MqttClient::new(timeout, svc.clone()),
            commit_edition: MqttClient::new(timeout, svc.clone()),
            create_event: MqttClient::new(timeout, svc.clone()),
            list_events: MqttClient::new(timeout, svc.clone()),
            dump_events: MqttClient::new(timeout, svc),
        }
    }

    async fn read_room(
        &self,
        payload: EventRoomReadPayload,
    ) -> Result<EventRoomResponse, ClientError> {
        self.read_room.clone().call(payload).await
    }

    async fn create_room(&self, payload: EventRoomCreatePayload) -> Result<Uuid, ClientError> {
        self.create_room.clone().call(payload).await.map(|v| v.id)
    }

    async fn update_room(&self, payload: EventRoomUpdatePayload) -> Result<(), ClientError> {
        self.update_room.clone().call(payload).await.map(|_v| ())
    }

    async fn update_locked_types(
        &self,
        payload: EventRoomLockedTypesPayload,
    ) -> Result<(), ClientError> {
        self.update_locked_types
            .clone()
            .call(payload)
            .await
            .map(|_v| ())
    }

    async fn adjust_room(&self, payload: EventAdjustPayload) -> Result<(), ClientError> {
        self.adjust_room.clone().call(payload).await.map(|_v| ())
    }

    async fn commit_edition(&self, payload: EventCommitPayload) -> Result<(), ClientError> {
        self.commit_edition.clone().call(payload).await.map(|_v| ())
    }

    async fn create_event(&self, payload: EventCreateEventPayload) -> Result<(), ClientError> {
        self.create_event.clone().call(payload).await.map(|_v| ())
    }

    async fn list_events(&self, payload: EventListPayload) -> Result<Vec<Event>, ClientError> {
        self.list_events.clone().call(payload).await
    }

    async fn dump_events(&self, payload: EventDumpEventsPayload) -> Result<(), ClientError> {
        self.dump_events.clone().call(payload).await.map(|_v| ())
    }
}

#[derive(Clone)]
pub struct TowerClient {
    inner: Arc<TowerClientInner>,
}

impl TowerClient {
    #[allow(dead_code)]
    pub fn new(
        me: AgentId,
        event_account_id: AccountId,
        dispatcher: Arc<Dispatcher>,
        timeout: Option<Duration>,
        api_version: &str,
    ) -> Self {
        let svc = MqttBaseClient::new(me, event_account_id, dispatcher, api_version);
        let inner = TowerClientInner::new(timeout, svc);

        Self {
            inner: Arc::new(inner),
        }
    }
}

#[async_trait]
impl EventClient for TowerClient {
    async fn read_room(&self, id: Uuid) -> Result<EventRoomResponse, ClientError> {
        self.inner.read_room(EventRoomReadPayload { id }).await
    }

    async fn create_room(&self, payload: EventRoomCreatePayload) -> Result<Uuid, ClientError> {
        self.inner.create_room(payload).await
    }

    async fn update_room(&self, id: Uuid, update: RoomUpdate) -> Result<(), ClientError> {
        self.inner
            .update_room(EventRoomUpdatePayload {
                id,
                time: update.time,
                classroom_id: update.classroom_id,
            })
            .await
    }

    async fn update_locked_types(
        &self,
        id: Uuid,
        locked_types: LockedTypes,
    ) -> Result<(), ClientError> {
        self.inner
            .update_locked_types(EventRoomLockedTypesPayload { id, locked_types })
            .await
    }

    async fn adjust_room(
        &self,
        event_room_id: Uuid,
        started_at: DateTime<Utc>,
        segments: Segments,
        offset: i64,
    ) -> Result<(), ClientError> {
        self.inner
            .adjust_room(EventAdjustPayload {
                id: event_room_id,
                started_at,
                segments,
                offset,
            })
            .await
    }

    async fn commit_edition(&self, edition_id: Uuid, offset: i64) -> Result<(), ClientError> {
        self.inner
            .commit_edition(EventCommitPayload {
                id: edition_id,
                offset,
            })
            .await
    }

    async fn create_event(&self, payload: JsonValue) -> Result<(), ClientError> {
        self.inner
            .create_event(EventCreateEventPayload { json: payload })
            .await
            .map(|_v| ())
    }
    async fn list_events(&self, room_id: Uuid, kind: &str) -> Result<Vec<Event>, ClientError> {
        let mut events = vec![];
        let mut last_occurred_at = None;

        for _ in 0..MAX_EVENT_LIST_PAGES {
            let response = self
                .inner
                .list_events(EventListPayload {
                    room_id,
                    kind: kind.to_owned(),
                    last_occurred_at,
                    limit: EVENT_LIST_LIMIT,
                })
                .await?;

            if let Some(last_event) = response.last() {
                last_occurred_at = Some(last_event.occurred_at());

                let mut events_page = response;
                events.append(&mut events_page);
            } else {
                break;
            }
        }

        Ok(events)
    }
    async fn dump_room(&self, event_room_id: Uuid) -> Result<(), ClientError> {
        self.inner
            .dump_events(EventDumpEventsPayload { id: event_room_id })
            .await
    }
}
