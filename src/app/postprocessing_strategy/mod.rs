use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_derive::Deserialize;
use svc_agent::AgentId;
use uuid::Uuid;

use crate::app::AppContext;
use crate::clients::event::RoomAdjustResult;
use crate::clients::tq::TaskCompleteResult;
use crate::db::class::{ClassType, Object as Class};
use crate::db::recording::Segments;

use minigroup::MinigroupPostprocessingStrategy;
use webinar::WebinarPostprocessingStrategy;

////////////////////////////////////////////////////////////////////////////////

pub(crate) fn get(
    ctx: Arc<dyn AppContext>,
    class: Class,
) -> Result<Box<dyn PostprocessingStrategy + Send + Sync>> {
    match class.kind() {
        ClassType::Classroom => bail!("Postprocessing for a classroom is not available"),
        ClassType::Minigroup => Ok(Box::new(MinigroupPostprocessingStrategy::new(ctx, class))),
        ClassType::Webinar => Ok(Box::new(WebinarPostprocessingStrategy::new(ctx, class))),
    }
}

#[async_trait]
pub(crate) trait PostprocessingStrategy {
    async fn handle_upload(&self, rtcs: Vec<RtcUploadResult>) -> Result<()>;
    async fn handle_adjust(&self, room_adjust_result: RoomAdjustResult) -> Result<()>;

    async fn handle_transcoding_completion(
        &self,
        completion_result: TaskCompleteResult,
    ) -> Result<()>;
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Deserialize)]
#[serde(tag = "status")]
#[serde(rename_all = "lowercase")]
pub(crate) enum RtcUploadResult {
    Ready(RtcUploadReadyData),
    Missing(RtcUploadMissingData),
}

#[derive(Debug, Deserialize)]
pub(crate) struct RtcUploadReadyData {
    pub(self) id: Uuid,
    pub(self) uri: String,
    #[serde(deserialize_with = "chrono::serde::ts_milliseconds::deserialize")]
    pub(self) started_at: DateTime<Utc>,
    #[serde(deserialize_with = "crate::db::recording::serde::segments::deserialize")]
    pub(self) segments: Segments,
    pub(self) created_by: AgentId,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RtcUploadMissingData {
    pub(self) id: Uuid,
}

////////////////////////////////////////////////////////////////////////////////

mod minigroup;
pub(self) mod shared_helpers;
mod webinar;
