use std::ops::Bound;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_derive::Serialize;
use serde_json::Value as JsonValue;
use sqlx::{postgres::PgConnection, Acquire};
use svc_agent::mqtt::{
    IntoPublishableMessage, OutgoingEvent, OutgoingEventProperties, ShortTermTimingProperties,
};
use uuid::Uuid;

use crate::app::AppContext;
use crate::clients::event::{Event, EventData, RoomAdjustResult};
use crate::clients::tq::{
    Task as TqTask, TaskCompleteResult, TaskCompleteSuccess, TranscodeMinigroupToHlsStream,
    TranscodeMinigroupToHlsSuccess,
};
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
        if rtcs.is_empty() {
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

    async fn handle_adjust(&self, room_adjust_result: RoomAdjustResult) -> Result<()> {
        match room_adjust_result {
            RoomAdjustResult::Success {
                original_room_id,
                modified_room_id,
                ..
            } => {
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
                        let event_room_offset =
                            recording.started_at() - modified_event_room_opened_at;

                        let recording_offset =
                            recording.started_at() - earliest_recording.started_at();

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
            TaskCompleteResult::Success(TaskCompleteSuccess::TranscodeMinigroupToHls(
                TranscodeMinigroupToHlsSuccess { recording_duration },
            )) => {
                let recording_duration = recording_duration.parse::<f64>()?.round() as u64;

                {
                    let mut conn = self.ctx.get_conn().await?;

                    crate::db::recording::TranscodingUpdateQuery::new(self.minigroup.id())
                        .execute(&mut conn)
                        .await?;
                }

                let timing = ShortTermTimingProperties::new(Utc::now());
                let props = OutgoingEventProperties::new("minigroup.ready", timing);
                let path = format!("audiences/{}/events", self.minigroup.audience());

                let payload = MinigroupReady {
                    tags: self.minigroup.tags(),
                    recording_duration,
                    status: "success",
                    scope: self.minigroup.scope(),
                    id: self.minigroup.id(),
                };

                let event = OutgoingEvent::broadcast(payload, props, &path);
                let boxed_event = Box::new(event) as Box<dyn IntoPublishableMessage + Send>;

                self.ctx
                    .publisher()
                    .publish(boxed_event)
                    .context("Failed to publish minigroup.ready event")
            }
            TaskCompleteResult::Success(success_result) => {
                bail!(
                    "Got transcoding success for an unexpected tq template; expected transcode-minigroup-to-hls for a minigroup, id = {}, result = {:?}",
                    self.minigroup.id(),
                    success_result,
                );
            }
            TaskCompleteResult::Failure { error } => {
                bail!("Transcoding failed: {}", error);
            }
        }
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
            rtc.segments.to_owned(),
            rtc.started_at,
            rtc.uri.to_owned(),
            rtc.created_by.to_owned(),
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

#[derive(Serialize)]
struct MinigroupReady {
    tags: Option<JsonValue>,
    status: &'static str,
    recording_duration: u64,
    scope: String,
    id: Uuid,
}

////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    mod handle_upload {
        use std::ops::Bound;
        use std::sync::Arc;

        use chrono::{DateTime, Duration, Utc};
        use svc_agent::AccountId;
        use uuid::Uuid;

        use crate::app::AppContext;
        use crate::db::recording::{RecordingListQuery, Segments};
        use crate::test_helpers::prelude::*;

        use super::super::super::{PostprocessingStrategy, RtcUploadReadyData, RtcUploadResult};
        use super::super::*;

        #[async_std::test]
        async fn handle_upload() {
            let now = Utc::now();
            let conference = TestAgent::new("conference-0", "conference", SVC_AUDIENCE);
            let mut state = TestState::new(conference, TestAuthz::new()).await;
            let conference_room_id = Uuid::new_v4();
            let event_room_id = Uuid::new_v4();

            // Insert a minigroup.
            let minigroup = {
                let mut conn = state.get_conn().await.expect("Failed to get conn");

                let time = (
                    Bound::Included(now - Duration::hours(1)),
                    Bound::Excluded(now - Duration::minutes(10)),
                );

                factory::Minigroup::new(
                    "minigroup123".to_string(),
                    USR_AUDIENCE.to_string(),
                    time.into(),
                    AccountId::new("host", USR_AUDIENCE),
                    conference_room_id,
                    event_room_id,
                )
                .insert(&mut conn)
                .await
            };

            let minigroup_id = minigroup.id();

            // Set up event client mock.
            let expected_started_at = now - Duration::hours(1);
            let expected_segments = vec![(Bound::Included(0), Bound::Excluded(3000000))].into();

            state
                .event_client_mock()
                .expect_adjust_room()
                .withf(
                    move |room_id: &Uuid,
                          started_at: &DateTime<Utc>,
                          segments: &Segments,
                          offset: &i64| {
                        assert_eq!(*room_id, event_room_id);
                        assert_eq!(*started_at, expected_started_at);
                        assert_eq!(segments, &expected_segments);
                        assert_eq!(*offset, 4018);
                        true
                    },
                )
                .returning(|_, _, _, _| Ok(()));

            // Handle uploading two RTCs.
            let rtc1_id = Uuid::new_v4();
            let uri1 = "s3://minigroup.origin.dev.example.com/rtc1.webm";
            let started_at1 = now - Duration::hours(1);
            let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);

            let segments1: Segments = vec![
                (Bound::Included(0), Bound::Excluded(1500000)),
                (Bound::Included(1800000), Bound::Excluded(3000000)),
            ]
            .into();

            let rtc1 = RtcUploadResult::Ready(RtcUploadReadyData {
                id: rtc1_id,
                uri: uri1.to_string(),
                started_at: started_at1,
                segments: segments1.clone(),
                created_by: agent1.agent_id().to_owned(),
            });

            let rtc2_id = Uuid::new_v4();
            let uri2 = "s3://minigroup.origin.dev.example.com/rtc2.webm";
            let started_at2 = now - Duration::minutes(50);
            let segments2: Segments = vec![(Bound::Included(0), Bound::Excluded(2700000))].into();
            let agent2 = TestAgent::new("web", "user2", USR_AUDIENCE);

            let rtc2 = RtcUploadResult::Ready(RtcUploadReadyData {
                id: rtc2_id,
                uri: uri2.to_string(),
                started_at: started_at2,
                segments: segments2.clone(),
                created_by: agent2.agent_id().to_owned(),
            });

            let state = Arc::new(state);

            MinigroupPostprocessingStrategy::new(state.clone(), minigroup)
                .handle_upload(vec![rtc1, rtc2])
                .await
                .expect("Failed to handle upload");

            // Assert recordings in the DB.
            let recordings = {
                let mut conn = state.get_conn().await.expect("Failed to get conn");

                RecordingListQuery::new(minigroup_id)
                    .execute(&mut conn)
                    .await
                    .expect("Failed to list recordings")
            };

            assert_eq!(recordings.len(), 2);

            let recording1 = recordings
                .iter()
                .find(|recording| recording.rtc_id() == rtc1_id)
                .expect("Recording 1 not found");

            assert_eq!(recording1.stream_uri(), uri1);
            assert_eq!(recording1.started_at(), started_at1);
            assert_eq!(recording1.segments(), &segments1);
            assert_eq!(recording1.created_by(), agent1.agent_id());

            let recording2 = recordings
                .iter()
                .find(|recording| recording.rtc_id() == rtc2_id)
                .expect("Recording 2 not found");

            assert_eq!(recording2.stream_uri(), uri2);
            assert_eq!(recording2.started_at(), now - Duration::minutes(50));
            assert_eq!(recording2.segments(), &segments2);
            assert_eq!(recording2.created_by(), agent2.agent_id());
        }
    }
}
