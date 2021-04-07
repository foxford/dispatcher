use std::ops::Bound;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{postgres::PgConnection, Acquire};
use uuid::Uuid;

use crate::app::AppContext;
use crate::db::class::Object as Class;
use crate::db::recording::{BoundedOffsetTuples, Segments};

use super::{shared_helpers, RtcUploadReadyData, RtcUploadResult};

pub(super) struct MinigroupPostprocessingStrategy {
    ctx: Arc<dyn AppContext>,
    class: Class,
}

impl MinigroupPostprocessingStrategy {
    pub(super) fn new(ctx: Arc<dyn AppContext>, class: Class) -> Self {
        Self { ctx, class }
    }
}

#[async_trait]
impl super::PostprocessingStrategy for MinigroupPostprocessingStrategy {
    async fn handle_upload(&self, rtcs: Vec<RtcUploadResult>) -> Result<()> {
        if rtcs.len() < 1 {
            bail!("Expected at least 1 RTC");
        }

        let ready_rtcs = shared_helpers::extract_ready_rtcs(rtcs)?;

        {
            let mut conn = self.ctx.get_conn().await?;
            insert_recordings(&mut conn, self.class.id(), &ready_rtcs).await?;
        }

        call_adjust(self.ctx.clone(), self.class.event_room_id(), &ready_rtcs).await?;
        Ok(())
    }
}

async fn insert_recordings(
    conn: &mut PgConnection,
    class_id: Uuid,
    rtcs: &[RtcUploadReadyData],
) -> Result<()> {
    let mut txn = conn
        .begin()
        .await
        .context("Failed to begin sqlx db transaction")?;

    for rtc in rtcs {
        let q = crate::db::recording::RecordingInsertQuery::new(
            class_id,
            rtc.id,
            rtc.segments.clone(),
            rtc.started_at,
            rtc.uri.clone(),
        );

        q.execute(&mut txn).await?;
    }

    txn.commit().await?;
    Ok(())
}

async fn call_adjust(
    ctx: Arc<dyn AppContext>,
    room_id: Uuid,
    rtcs: &[RtcUploadReadyData],
) -> Result<()> {
    let started_at = rtcs
        .iter()
        .map(|rtc| rtc.started_at)
        .min()
        .ok_or_else(|| anyhow!("Couldn't get min started at"))?;

    let segments = build_adjust_segments(&rtcs)?;

    ctx.event_client()
        // TODO FIX OFFSET
        .adjust_room(room_id, started_at, segments, 4018)
        .await
        .map_err(|err| anyhow!("Failed to adjust room, id = {}: {}", room_id, err))?;

    Ok(())
}

fn build_adjust_segments(rtcs: &[RtcUploadReadyData]) -> Result<Segments> {
    let mut maybe_min_start: Option<i64> = None;
    let mut maybe_max_stop: Option<i64> = None;

    for rtc in rtcs.iter() {
        let segments: BoundedOffsetTuples = rtc.segments.clone().into();

        if let Some((Bound::Included(start), _)) = segments.first() {
            if let Some(min_start) = maybe_min_start {
                if *start < min_start {
                    maybe_min_start = Some(*start);
                }
            } else {
                maybe_min_start = Some(*start);
            }
        }

        if let Some((_, Bound::Excluded(stop))) = segments.last() {
            if let Some(max_stop) = maybe_max_stop {
                if *stop > max_stop {
                    maybe_max_stop = Some(*stop);
                }
            } else {
                maybe_max_stop = Some(*stop);
            }
        }
    }

    if let (Some(start), Some(stop)) = (maybe_min_start, maybe_max_stop) {
        Ok(vec![(Bound::Included(start), Bound::Excluded(stop))].into())
    } else {
        bail!("Couldn't find min start & max stop in segments");
    }
}
