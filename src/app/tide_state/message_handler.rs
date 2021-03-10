use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::Acquire;
use svc_agent::mqtt::{
    IncomingEvent, IncomingResponse, IntoPublishableMessage, OutgoingEvent,
    OutgoingEventProperties, ShortTermTimingProperties,
};
use svc_agent::request::Dispatcher;
use uuid::Uuid;

use super::AppContext;
use crate::db::recording::Segments;

pub struct MessageHandler {
    ctx: Arc<dyn AppContext>,
    dispatcher: Arc<Dispatcher>,
}

impl MessageHandler {
    pub fn new(ctx: Arc<dyn AppContext>, dispatcher: Arc<Dispatcher>) -> Self {
        Self { ctx, dispatcher }
    }

    pub async fn handle_response(&self, data: IncomingResponse<String>) {
        match IncomingResponse::convert::<JsonValue>(data) {
            Ok(message) => {
                if let Err(e) = self.dispatcher.response(message).await {
                    error!(crate::LOG, "Failed to commit response, reason = {:?}", e);
                }
            }
            Err(e) => error!(crate::LOG, "Failed to parse response, reason = {:?}", e),
        }
    }

    pub async fn handle_event(&self, data: IncomingEvent<String>) {
        let result = match data.properties().label() {
            Some("room.upload") => self.handle_upload(data).await,
            Some("room.adjust") => self.handle_adjust(data).await,
            Some("task.complete") => self.handle_transcoding(data).await,
            val => {
                debug!(
                    crate::LOG,
                    "Unexpected incoming event label = {:?}, payload = {:?}", val, data
                );
                Ok(())
            }
        };

        if let Err(e) = result {
            error!(crate::LOG, "Event handler failed, reason = {:?}", e);
        }
    }

    async fn handle_upload(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let room_upload: RoomUpload = serde_json::from_str(&payload)?;
        let rtc = room_upload
            .rtcs
            .get(0)
            .ok_or_else(|| anyhow!("Missing rtc in room upload, payload = {:?}", room_upload))?;
        let mut conn = self.ctx.get_conn().await?;
        let q = crate::db::recording::RecordingInsertQuery::new(
            room_upload.id,
            rtc.id,
            rtc.segments.clone(),
            rtc.started_at,
        );
        q.execute(&mut conn).await?;
        Ok(())
    }

    async fn handle_adjust(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let room_adjust: RoomAdjust = serde_json::from_str(&payload)?;
        match room_adjust.result {
            RoomAdjustResult::Success {
                original_room_id,
                modified_room_id,
                modified_segments,
            } => {
                if let Some(scope) = room_adjust.tags.and_then(|v| {
                    v.get("scope")
                        .and_then(|s| s.as_str().map(|s| s.to_owned()))
                }) {
                    let mut conn = self.ctx.get_conn().await?;
                    let webinar = crate::db::class::WebinarReadByScopeQuery::new(scope.clone())
                        .execute(&mut conn)
                        .await?
                        .ok_or_else(|| anyhow!("Room not found by scope = {:?}", scope))?;

                    let mut txn = conn
                        .begin()
                        .await
                        .context("Failed to begin sqlx db transaction")?;
                    let q = crate::db::class::WebinarUpdateQuery::new(
                        webinar.id(),
                        original_room_id,
                        modified_room_id,
                    );
                    q.execute(&mut txn).await?;

                    let q = crate::db::recording::AdjustUpdateQuery::new(
                        webinar.id(),
                        modified_segments,
                    );
                    q.execute(&mut txn).await?;
                    txn.commit().await?;
                } else {
                    bail!("No scope specified in tags, payload = {:?}", payload);
                }
            }
            RoomAdjustResult::Error { error } => {
                bail!("Adjust failed, err = {:?}", error);
            }
        }

        Ok(())
    }

    async fn handle_transcoding(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let task: TaskComplete = serde_json::from_str(&payload)?;
        match task.result {
            TaskCompleteResult::Success {
                stream_duration,
                stream_id,
                stream_uri,
            } => {
                if let Some(scope) = task.tags.and_then(|v| {
                    v.get("scope")
                        .and_then(|s| s.as_str().map(|s| s.to_owned()))
                }) {
                    let mut conn = self.ctx.get_conn().await?;
                    let webinar = crate::db::class::WebinarReadByScopeQuery::new(scope.clone())
                        .execute(&mut conn)
                        .await?
                        .ok_or_else(|| anyhow!("Room not found by scope = {:?}", scope))?;

                    crate::db::recording::TranscodingUpdateQuery::new(webinar.id())
                        .execute(&mut conn)
                        .await?;

                    let mut agent = self.ctx.agent();
                    let timing = ShortTermTimingProperties::new(chrono::Utc::now());
                    let props = OutgoingEventProperties::new("webinar.ready", timing);
                    let path = format!("audiences/{}/events", webinar.audience());
                    let payload = WebinarReady {
                        tags: webinar.tags(),
                        stream_duration,
                        stream_uri,
                        stream_id,
                        status: "success",
                        scope: webinar.scope(),
                        id: webinar.id(),
                    };
                    let event = OutgoingEvent::broadcast(payload, props, &path);

                    let e = Box::new(event) as Box<dyn IntoPublishableMessage + Send>;

                    if let Err(err) = agent.publish_publishable(e) {
                        error!(
                            crate::LOG,
                            "Failed to publish rollback event, reason = {:?}", err
                        );
                    }
                } else {
                    bail!("No scope specified in tags, payload = {:?}", payload);
                }
            }
            TaskCompleteResult::Failure { error } => {
                bail!("Transcoding failed, err = {:?}", error);
            }
        }

        Ok(())
    }
}

#[derive(Deserialize, Debug)]
struct RoomUpload {
    id: Uuid,
    rtcs: Vec<RtcUpload>,
}

#[derive(Deserialize, Debug)]
struct RtcUpload {
    id: Uuid,
    uri: String,
    status: String,
    segments: crate::db::recording::Segments,
    started_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct RoomAdjust {
    tags: Option<JsonValue>,
    #[serde(flatten)]
    result: RoomAdjustResult,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RoomAdjustResult {
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
#[derive(Deserialize)]
struct TaskComplete {
    tags: Option<JsonValue>,
    #[serde(flatten)]
    result: TaskCompleteResult,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TaskCompleteResult {
    Success {
        stream_id: Uuid,
        stream_uri: String,
        stream_duration: u64,
    },
    Failure {
        error: JsonValue,
    },
}

#[derive(Serialize)]
struct WebinarReady {
    tags: Option<JsonValue>,
    status: &'static str,
    stream_duration: u64,
    stream_id: Uuid,
    stream_uri: String,
    scope: String,
    id: Uuid,
}
