use std::ops::Bound;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Duration;
use sqlx::{postgres::PgConnection, Acquire};
use uuid::Uuid;

use crate::app::AppContext;
use crate::clients::event::{Event, EventData};
use crate::clients::tq::{Task as TqTask, TranscodeMinigroupToHlsStream};
use crate::db::class::Object as Class;
use crate::db::recording::{BoundedOffsetTuples, Object as Recording, Segments};

use super::{shared_helpers, RtcUploadReadyData, RtcUploadResult};

const PIN_EVENT_TYPE: &str = "pin";

pub(super) struct MinigroupPostprocessingStrategy {
    ctx: Arc<dyn AppContext>,
    minigroup: Class,
}

impl MinigroupPostprocessingStrategy {
    pub(super) fn new(ctx: Arc<dyn AppContext>, minigroup: Class) -> Self {
        Self { ctx, minigroup }
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
            insert_recordings(&mut conn, self.minigroup.id(), &ready_rtcs).await?;
        }

        call_adjust(
            self.ctx.clone(),
            self.minigroup.event_room_id(),
            &ready_rtcs,
        )
        .await?;
        Ok(())
    }

    async fn handle_adjust(
        &self,
        original_room_id: Uuid,
        modified_room_id: Uuid,
        _modified_segments: Segments,
    ) -> Result<()> {
        // Save adjust results to the DB and fetch recordings.
        let recordings = {
            let mut conn = self.ctx.get_conn().await?;

            let mut txn = conn
                .begin()
                .await
                .context("Failed to begin sqlx db transaction")?;

            let q = crate::db::class::UpdateQuery::new(
                self.minigroup.id(),
                original_room_id,
                modified_room_id,
            );

            q.execute(&mut txn).await?;

            let recordings =
                crate::db::recording::AdjustMinigroupUpdateQuery::new(self.minigroup.id())
                    .execute(&mut txn)
                    .await?;

            txn.commit().await?;
            recordings
        };

        // Find the earliest recording.
        let earliest_recording = recordings
            .iter()
            .min_by(|a, b| a.started_at().cmp(&b.started_at()))
            .ok_or_else(|| anyhow!("No recordings"))?;

        // Fetch event room opening time for events' offset calculation.
        let modified_event_room = self
            .ctx
            .event_client()
            .read_room(modified_room_id)
            .await
            .context("Failed to read modified event room")?;

        let modified_event_room_opened_at = match modified_event_room.time {
            (Bound::Included(opened_at), _) => opened_at,
            _ => bail!("Wrong event room opening time"),
        };

        // Fetch pin events for building pin segments.
        let pin_events = self
            .ctx
            .event_client()
            .list_events(modified_room_id, PIN_EVENT_TYPE)
            .await
            .context("Failed to get pin events for room")?;

        // Build streams for template bindings.
        let streams = recordings
            .iter()
            .map(|recording| {
                let event_room_offset = recording.started_at() - modified_event_room_opened_at;
                let recording_offset = recording.started_at() - earliest_recording.started_at();
                build_stream(recording, &pin_events, event_room_offset, recording_offset)
            })
            .collect::<Vec<_>>();

        // Create a tq task.
        self.ctx
            .tq_client()
            .create_task(&self.minigroup, TqTask::TranscodeMinigroupToHls { streams })
            .await
            .context("TqClient create task failed")
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

fn build_stream(
    recording: &Recording,
    pin_events: &[Event],
    event_room_offset: Duration,
    recording_offset: Duration,
) -> TranscodeMinigroupToHlsStream {
    let event_room_offset = event_room_offset.num_nanoseconds().unwrap_or(i64::MAX);
    let mut pin_segments = vec![];
    let mut pin_start = None;

    for event in pin_events {
        match event.data() {
            EventData::Pin(data) => {
                // Shift from the event room's dimension to the recording's dimension.
                let occurred_at = event.occurred_at() as i64 - event_room_offset;

                if data.agent_id() == recording.created_by() && pin_start.is_none() {
                    // Stream has got pinned.
                    pin_start = Some(occurred_at);
                } else if let Some(pinned_at) = pin_start {
                    // Stream has got unpinned.
                    pin_segments.push((Bound::Included(pinned_at), Bound::Excluded(occurred_at)));
                    pin_start = None;
                }
            }
        }
    }

    // If the stream hasn't got unpinned since some moment then add a pin segment to the end
    // of the recording to keep it pinned.
    if let Some(start) = pin_start {
        let recording_segments: BoundedOffsetTuples = recording.segments().to_owned().into();

        if let Some((_, Bound::Excluded(recording_end))) = recording_segments.last() {
            pin_segments.push((Bound::Included(start), Bound::Excluded(*recording_end)));
        }
    }

    TranscodeMinigroupToHlsStream::new(recording.rtc_id(), recording.stream_uri().to_owned())
        .offset(recording_offset.num_milliseconds() as u64)
        .segments(recording.segments().to_owned())
        .pin_segments(pin_segments.into())
}
