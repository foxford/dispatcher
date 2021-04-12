use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_derive::Serialize;
use serde_json::Value as JsonValue;
use sqlx::Acquire;
use svc_agent::mqtt::{
    IntoPublishableMessage, OutgoingEvent, OutgoingEventProperties, ShortTermTimingProperties,
};
use uuid::Uuid;

use crate::app::AppContext;
use crate::clients::event::RoomAdjustResult;
use crate::clients::tq::{
    Task as TqTask, TaskCompleteResult, TaskCompleteSuccess, TranscodeStreamToHlsSuccess,
};
use crate::db::class::Object as Class;

use super::{shared_helpers, RtcUploadResult};

// TODO: make configurable for each audience.
const PREROLL_OFFSET: i64 = 4018;

pub(super) struct WebinarPostprocessingStrategy {
    ctx: Arc<dyn AppContext>,
    webinar: Class,
}

impl WebinarPostprocessingStrategy {
    pub(super) fn new(ctx: Arc<dyn AppContext>, webinar: Class) -> Self {
        Self { ctx, webinar }
    }
}

#[async_trait]
impl super::PostprocessingStrategy for WebinarPostprocessingStrategy {
    async fn handle_upload(&self, rtcs: Vec<RtcUploadResult>) -> Result<()> {
        let ready_rtcs = shared_helpers::extract_ready_rtcs(rtcs)?;

        let rtc = ready_rtcs
            .first()
            .ok_or_else(|| anyhow!("Expected exactly 1 RTC"))?;

        {
            let mut conn = self.ctx.get_conn().await?;

            crate::db::recording::RecordingInsertQuery::new(
                self.webinar.id(),
                rtc.id,
                rtc.segments.to_owned(),
                rtc.started_at,
                rtc.uri.to_owned(),
                rtc.created_by.to_owned(),
            )
            .execute(&mut conn)
            .await?;
        }

        self.ctx
            .event_client()
            .adjust_room(
                self.webinar.event_room_id(),
                rtc.started_at,
                rtc.segments.clone(),
                PREROLL_OFFSET,
            )
            .await
            .context("Failed to adjust room")
    }

    async fn handle_adjust(&self, room_adjust_result: RoomAdjustResult) -> Result<()> {
        match room_adjust_result {
            RoomAdjustResult::Success {
                original_room_id,
                modified_room_id,
                modified_segments,
            } => {
                let recording = {
                    let mut conn = self.ctx.get_conn().await?;

                    let mut txn = conn
                        .begin()
                        .await
                        .context("Failed to begin sqlx db transaction")?;

                    let q = crate::db::class::UpdateQuery::new(
                        self.webinar.id(),
                        original_room_id,
                        modified_room_id,
                    );

                    q.execute(&mut txn).await?;

                    let q = crate::db::recording::AdjustWebinarUpdateQuery::new(
                        self.webinar.id(),
                        modified_segments.clone(),
                    );

                    let recording = q.execute(&mut txn).await?;
                    txn.commit().await?;
                    recording
                };

                self.ctx
                    .tq_client()
                    .create_task(
                        &self.webinar,
                        TqTask::TranscodeStreamToHls {
                            stream_id: recording.rtc_id(),
                            stream_uri: recording.stream_uri().to_string(),
                            event_room_id: Some(modified_room_id),
                            segments: Some(modified_segments),
                        },
                    )
                    .await
                    .context("TqClient create task failed")
            }
            RoomAdjustResult::Error { error } => {
                bail!("Adjust failed, err = {:?}", error);
            }
        }
    }

    async fn handle_transcoding_completion(
        &self,
        completion_result: TaskCompleteResult,
    ) -> Result<()> {
        match completion_result {
            TaskCompleteResult::Success(TaskCompleteSuccess::TranscodeStreamToHls(
                TranscodeStreamToHlsSuccess {
                    stream_duration,
                    stream_id,
                    stream_uri,
                    event_room_id,
                },
            )) => {
                let stream_duration = stream_duration.parse::<f64>()?.round() as u64;

                {
                    let mut conn = self.ctx.get_conn().await?;

                    crate::db::recording::TranscodingUpdateQuery::new(self.webinar.id())
                        .execute(&mut conn)
                        .await?;
                }

                let timing = ShortTermTimingProperties::new(Utc::now());
                let props = OutgoingEventProperties::new("webinar.ready", timing);
                let path = format!("audiences/{}/events", self.webinar.audience());

                let payload = WebinarReady {
                    tags: self.webinar.tags().map(ToOwned::to_owned),
                    stream_duration,
                    stream_uri,
                    stream_id,
                    status: "success",
                    scope: self.webinar.scope().to_owned(),
                    id: self.webinar.id(),
                    event_room_id,
                };

                let event = OutgoingEvent::broadcast(payload, props, &path);
                let boxed_event = Box::new(event) as Box<dyn IntoPublishableMessage + Send>;

                self.ctx
                    .publisher()
                    .publish(boxed_event)
                    .context("Failed to publish webinar.ready event")
            }
            TaskCompleteResult::Success(success_result) => {
                bail!(
                    "Got transcoding success for an unexpected tq template; expected transcode-stream-to-hls for a webinar, id = {}, result = {:?}",
                    self.webinar.id(),
                    success_result,
                );
            }
            TaskCompleteResult::Failure { error } => {
                bail!("Transcoding failed: {}", error);
            }
        }
    }
}

#[derive(Serialize)]
struct WebinarReady {
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    status: &'static str,
    stream_duration: u64,
    stream_id: Uuid,
    stream_uri: String,
    scope: String,
    id: Uuid,
    event_room_id: Uuid,
}
