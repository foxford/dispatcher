use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::Acquire;
use uuid::Uuid;

use crate::app::AppContext;
use crate::clients::tq::Task as TqTask;
use crate::db::class::Object as Class;
use crate::db::recording::Segments;

use super::{shared_helpers, RtcUploadResult};

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
                rtc.segments.clone(),
                rtc.started_at,
                rtc.uri.clone(),
            )
            .execute(&mut conn)
            .await?;
        }

        self.ctx
            .event_client()
            // TODO FIX OFFSET
            .adjust_room(
                self.webinar.event_room_id(),
                rtc.started_at,
                rtc.segments.clone(),
                4018,
            )
            .await
            .context("Failed to adjust room")
    }

    async fn handle_adjust(
        &self,
        original_room_id: Uuid,
        modified_room_id: Uuid,
        modified_segments: Segments,
    ) -> Result<()> {
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
}
