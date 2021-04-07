use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::app::AppContext;
use crate::db::class::Object as Class;

use super::{shared_helpers, RtcUploadResult};

pub(super) struct WebinarPostprocessingStrategy {
    ctx: Arc<dyn AppContext>,
    class: Class,
}

impl WebinarPostprocessingStrategy {
    pub(super) fn new(ctx: Arc<dyn AppContext>, class: Class) -> Self {
        Self { ctx, class }
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
                self.class.id(),
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
                self.class.event_room_id(),
                rtc.started_at,
                rtc.segments.clone(),
                4018,
            )
            .await
            .map_err(|err| anyhow!("Failed to adjust room, id = {}: {}", self.class.id(), err))?;

        Ok(())
    }
}
