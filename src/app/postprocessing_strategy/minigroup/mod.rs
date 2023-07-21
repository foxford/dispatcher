use std::collections::HashMap;
use std::ops::Bound;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::{postgres::PgConnection, Acquire};
use svc_agent::{
    mqtt::{
        IntoPublishableMessage, OutgoingEvent, OutgoingEventProperties, ShortTermTimingProperties,
    },
    AgentId,
};
use tracing::{error, warn};
use uuid::Uuid;

use crate::clients::event::{AdjustRecording, RecordingSegments, RoomAdjustResultV2};
use crate::clients::tq::{
    Priority, Task as TqTask, TranscodeMinigroupToHlsStream, TranscodeMinigroupToHlsSuccess,
};
use crate::db::class::{BoundedDateTimeTuple, Object as Class};
use crate::db::recording::Segments;
use crate::{app::AppContext, clients::conference::ConfigSnapshot};
use crate::{
    clients::event::{Event, EventData, RoomAdjustResult},
    db::class::ClassType,
};

use super::{
    shared_helpers, MjrDumpsUploadReadyData, MjrDumpsUploadResult, TranscodeSuccess, UploadedStream,
};

const NS_IN_MS: i64 = 1_000_000;
const PIN_EVENT_TYPE: &str = "pin";
const HOST_EVENT_TYPE: &str = "host";

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
    async fn handle_stream_upload(&self, stream: UploadedStream) -> Result<()> {
        match stream.parsed_data {
            Ok(stream_data) => {
                let mut conn = self.ctx.get_conn().await?;
                crate::db::recording::StreamUploadUpdateQuery::new(
                    self.minigroup.id(),
                    stream.id,
                    stream_data.segments,
                    stream_data.uri,
                    stream_data.started_at,
                )
                .execute(&mut conn)
                .await?;
            }
            Err(err) => {
                warn!(
                    stream_id = ?stream.id,
                    "Failed to transcode recording with stream_id: {}, err: {:?}", stream.id, err
                );
                let mut conn = self.ctx.get_conn().await?;
                crate::db::recording::remove_recording(self.minigroup.id(), stream.id, &mut conn)
                    .await?;
            }
        }
        let recordings = {
            let mut conn = self.ctx.get_conn().await?;
            crate::db::recording::RecordingListQuery::new(self.minigroup.id())
                .execute(&mut conn)
                .await?
        };

        let ready_recordings = recordings
            .iter()
            .filter_map(ReadyRecording::from_db_object)
            .collect::<Vec<_>>();
        if recordings.len() != ready_recordings.len() {
            return Ok(());
        }

        let host_recording = {
            // Find host stream id.
            let host = match self.find_host(self.minigroup.event_room_id()).await? {
                None => {
                    error!(
                        class_id = ?self.minigroup.id(),
                        event_room_id = ?self.minigroup.event_room_id(),
                        "No host in room",
                    );
                    return Ok(());
                }
                Some(agent_id) => agent_id,
            };

            let maybe_recording = ready_recordings
                .into_iter()
                .find(|recording| recording.created_by == host);

            match maybe_recording {
                None => {
                    error!(
                        class_id = ?self.minigroup.id(),
                        "No host recording in room",
                    );
                    return Ok(());
                }
                Some(recording) => recording,
            }
        };

        // Fetch writer config snapshots for building muted segments.
        let mute_events = self
            .ctx
            .conference_client()
            .read_config_snapshots(self.minigroup.conference_room_id())
            .await
            .context("Failed to get writer config snapshots for room")?;

        let recordings = recordings
            .iter()
            .map(|r| AdjustRecording {
                id: r.id(),
                rtc_id: r.rtc_id(),
                host: (host_recording.created_by == *r.created_by()),
                // todo get rid of  unwrap()
                segments: r.segments().unwrap().to_owned(),
                // todo get rid of  unwrap()
                started_at: r.started_at().unwrap(),
                created_by: r.created_by().to_owned(),
            })
            .collect::<Vec<_>>();

        call_adjust_v2(
            self.ctx.clone(),
            self.minigroup.event_room_id(),
            recordings,
            mute_events,
            self.ctx.get_preroll_offset(self.minigroup.audience()),
        )
        .await?;
        Ok(())
    }

    async fn handle_adjust(&self, room_adjust_result: RoomAdjustResult) -> Result<()> {
        let result = match room_adjust_result {
            RoomAdjustResult::V1(_) => {
                bail!("unsupported adjust version")
            }
            RoomAdjustResult::V2(result) => result,
        };

        match result {
            RoomAdjustResultV2::Success {
                original_room_id,
                modified_room_id,
                recordings,
                modified_room_time,
            } => {
                // Find host stream id.
                let host = match self.find_host(modified_room_id).await? {
                    None => {
                        error!(
                            class_id = ?self.minigroup.id(),
                            "No host in room",
                        );
                        return Ok(());
                    }
                    Some(agent_id) => agent_id,
                };

                // Save adjust results to the DB and fetch recordings.
                let records = {
                    let mut conn = self.ctx.get_conn().await?;

                    let mut txn = conn
                        .begin()
                        .await
                        .context("Failed to begin sqlx db transaction")?;

                    let q = crate::db::class::UpdateAdjustedRoomsQuery::new(
                        self.minigroup.id(),
                        original_room_id,
                        modified_room_id,
                    );

                    q.execute(&mut txn).await?;

                    // todo get host modified_segments and save them
                    let records = crate::db::recording::AdjustMinigroupUpdateQuery::new(
                        self.minigroup.id(),
                        // fixme
                        Segments::empty(),
                        host.clone(),
                    )
                    .execute(&mut txn)
                    .await?;

                    txn.commit().await?;

                    records
                };

                send_transcoding_task(
                    &self.ctx,
                    &self.minigroup,
                    records,
                    recordings,
                    Some(modified_room_time),
                    modified_room_id,
                    Priority::Normal,
                )
                .await
            }
            RoomAdjustResultV2::Error { error } => {
                bail!("Adjust failed, err = {:#?}", error);
            }
        }
    }

    async fn handle_transcoding_completion(
        &self,
        completion_result: TranscodeSuccess,
    ) -> Result<()> {
        match completion_result {
            TranscodeSuccess::TranscodeMinigroupToHls(TranscodeMinigroupToHlsSuccess {
                recording_duration,
                ..
            }) => {
                let stream_duration = recording_duration.parse::<f64>()?.round() as u64;

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
                    id: self.minigroup.id(),
                    scope: self.minigroup.scope().to_owned(),
                    tags: self.minigroup.tags().map(ToOwned::to_owned),
                    status: "success".to_string(),
                    stream_duration,
                };

                let event = OutgoingEvent::broadcast(payload, props, &path);
                let boxed_event = Box::new(event) as Box<dyn IntoPublishableMessage + Send>;

                self.ctx
                    .publisher()
                    .publish(boxed_event)
                    .context("Failed to publish minigroup.ready event")
            }
            TranscodeSuccess::TranscodeStreamToHls(success_result) => {
                bail!(
                    "Got transcoding success for an unexpected tq template; expected transcode-minigroup-to-hls for a minigroup, id = {}, result = {:#?}",
                    self.minigroup.id(),
                    success_result,
                );
            }
        }
    }

    async fn handle_mjr_dumps_upload(&self, dumps: Vec<MjrDumpsUploadResult>) -> Result<()> {
        if dumps.is_empty() {
            bail!("Expected at least 1 RTC");
        }

        let ready_dumps = shared_helpers::extract_ready_dumps(dumps)?;
        {
            let mut conn = self.ctx.get_conn().await?;
            insert_recordings(&mut conn, self.minigroup.id(), &ready_dumps).await?;
        }
        let tq_client = self.ctx.tq_client();
        for dump in ready_dumps {
            tq_client
                .create_task(
                    &self.minigroup,
                    TqTask::ConvertMjrDumpsToStream {
                        mjr_dumps_uris: dump.mjr_dumps_uris,
                        stream_uri: dump.uri,
                        stream_id: dump.id,
                    },
                    Priority::Normal,
                )
                .await?
        }
        Ok(())
    }
}

impl MinigroupPostprocessingStrategy {
    async fn find_host(&self, event_room_id: Uuid) -> Result<Option<AgentId>> {
        find_host(self.ctx.clone(), event_room_id).await
    }
}

async fn find_host(ctx: Arc<dyn AppContext>, event_room_id: Uuid) -> Result<Option<AgentId>> {
    let host_events = ctx
        .event_client()
        .list_events(event_room_id, HOST_EVENT_TYPE)
        .await
        .context("Failed to get host events for room")?;

    match host_events.first().map(|event| event.data()) {
        None => Ok(None),
        Some(EventData::Host(data)) => Ok(Some(data.agent_id().to_owned())),
        Some(other) => bail!("Got unexpected host event data: {:?}", other),
    }
}

async fn insert_recordings(
    conn: &mut PgConnection,
    class_id: Uuid,
    dumps: &[MjrDumpsUploadReadyData],
) -> Result<()> {
    let mut txn = conn
        .begin()
        .await
        .context("Failed to begin sqlx db transaction")?;

    for dump in dumps {
        let q = crate::db::recording::RecordingInsertQuery::new(
            class_id,
            dump.id,
            dump.created_by.to_owned(),
        );

        q.execute(&mut txn).await?;
    }

    txn.commit().await?;
    Ok(())
}

async fn call_adjust_v2(
    ctx: Arc<dyn AppContext>,
    room_id: Uuid,
    recordings: Vec<AdjustRecording>,
    mute_events: Vec<ConfigSnapshot>,
    offset: i64,
) -> Result<()> {
    ctx.event_client()
        .adjust_room_v2(room_id, recordings, mute_events, offset)
        .await
        .map_err(|err| anyhow!("Failed to adjust room, id = {}: {}", room_id, err))?;

    Ok(())
}

pub async fn restart_transcoding(
    ctx: Arc<dyn AppContext>,
    minigroup: Class,
    priority: Priority,
) -> Result<()> {
    if minigroup.kind() != ClassType::Minigroup {
        bail!("Invalid class type");
    }

    let modified_event_room_id = match minigroup.modified_event_room_id() {
        Some(id) => id,
        None => bail!("Not adjusted yet"),
    };

    let mut conn = ctx.get_conn().await?;
    let recordings = crate::db::recording::RecordingListQuery::new(minigroup.id())
        .execute(&mut conn)
        .await?;

    send_transcoding_task(
        &ctx,
        &minigroup,
        recordings,
        // fixme
        vec![],
        // fixme
        None,
        modified_event_room_id,
        priority,
    )
    .await
}

async fn send_transcoding_task(
    ctx: &Arc<dyn AppContext>,
    minigroup: &Class,
    recordings: Vec<crate::db::recording::Object>,
    recording_segments: Vec<RecordingSegments>,
    modified_room_time: Option<BoundedDateTimeTuple>,
    modified_event_room_id: Uuid,
    priority: Priority,
) -> Result<()> {
    // Find host stream id.
    let host = match find_host(ctx.clone(), modified_event_room_id).await? {
        None => {
            error!(class_id = ?minigroup.id(), "No host in room");
            return Ok(());
        }
        Some(agent_id) => agent_id,
    };

    let recordings = recordings
        .into_iter()
        .map(|recording| ReadyRecording::from_db_object(&recording))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| anyhow!("Not all recordings are ready"))?;

    let recordings_map = recordings
        .iter()
        .map(|r| (r.id.to_owned(), r.to_owned()))
        .collect::<HashMap<_, _>>();

    let maybe_host_recording = recordings
        .iter()
        .find(|recording| recording.created_by == host);

    let host_stream = match maybe_host_recording {
        // Host has been set but there's no recording, skip transcoding.
        None => bail!("No host stream id in room"),
        Some(recording) => recording,
    };

    // todo get rid of unwrap()
    match modified_room_time.unwrap() {
        (Bound::Included(_), _) => (),
        _ => bail!("Wrong event room opening time"),
    };

    ctx.event_client()
        .dump_room(modified_event_room_id)
        .await
        .context("Dump room event failed")?;

    // Find the earliest recording.
    let earliest_recording = recordings
        .iter()
        .min_by(|a, b| a.started_at.cmp(&b.started_at))
        .ok_or_else(|| anyhow!("No recordings"))?;

    // Build streams for template bindings.

    let streams = recording_segments
        .iter()
        .map(|r| {
            // todo get rid of unwrap()
            let recording = recordings_map.get(&r.id).unwrap();

            let recording_offset = recording.started_at - earliest_recording.started_at;
            build_stream(recording, r, recording_offset)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let host_stream_id = host_stream.rtc_id;

    // Create a tq task.
    let task = TqTask::TranscodeMinigroupToHls {
        streams,
        host_stream_id,
    };

    ctx.tq_client()
        .create_task(minigroup, task, priority)
        .await
        .context("TqClient create task failed")
}

fn build_stream(
    recording: &ReadyRecording,
    recording_segments: &RecordingSegments,
    recording_offset: Duration,
) -> Result<TranscodeMinigroupToHlsStream> {
    let v = TranscodeMinigroupToHlsStream::new(recording.rtc_id, recording.stream_uri.to_owned())
        .offset(recording_offset.num_milliseconds() as u64)
        // We pass not modified but original segments here, this is done because:
        // First of all remember that:
        // - for non-hosts' streams modified segments == og segments
        // - for host's stream modified segments DO differ
        // All non-hosts' streams should be cutted out when there is a gap in host stream, for example:
        //     host's str: begin------------pause----pauseend------------end (notice pause in the middle)
        // non-host's str: begin-----------------------------------------end (no pause at all)
        // For a non-host's stream we must apply the pause in the host's stream
        // Tq does that but it needs pauses, and these pauses are only present in og segments, not in modified segments
        .modified_segments(recording_segments.modified_segments.to_owned())
        .segments(recording.segments.to_owned())
        .pin_segments(recording_segments.pin_segments.into())
        .video_mute_segments(recording_segments.video_mute_segments.into())
        .audio_mute_segments(recording_segments.audio_mute_segments.into());

    Ok(v)
}

fn collect_pin_segments(
    pin_events: &[Event],
    event_room_offset: Duration,
    recording_created_by: &AgentId,
    recording_end: i64,
) -> Vec<(Bound<i64>, Bound<i64>)> {
    let mut pin_segments = vec![];
    let mut pin_start = None;

    let mut add_segment = |start, end| {
        if start <= end && start >= 0 && end <= recording_end {
            pin_segments.push((Bound::Included(start), Bound::Excluded(end)));
        }
    };

    for event in pin_events {
        if let EventData::Pin(data) = event.data() {
            // Shift from the event room's dimension to the recording's dimension.
            let occurred_at =
                event.occurred_at() as i64 / NS_IN_MS - event_room_offset.num_milliseconds();

            if data
                .agent_id()
                .map(|aid| *aid == *recording_created_by)
                .unwrap_or(false)
            {
                // Stream was pinned.
                // Its possible that teacher pins someone twice in a row
                // Do nothing in that case
                if pin_start.is_none() {
                    pin_start = Some(occurred_at);
                }
            } else if let Some(pinned_at) = pin_start {
                // stream was unpinned
                // its possible that pinned_at equals unpin's occurred_at after adjust
                // we skip segments like that
                if occurred_at > pinned_at {
                    add_segment(pinned_at, occurred_at);
                }
                pin_start = None;
            }
        }
    }

    // If the stream hasn't got unpinned since some moment then add a pin segment to the end
    // of the recording to keep it pinned.
    if let Some(start) = pin_start {
        add_segment(start, recording_end);
    }

    pin_segments
}

#[derive(Debug)]
struct ReadyRecording {
    id: Uuid,
    rtc_id: Uuid,
    stream_uri: String,
    segments: Segments,
    modified_segments: Segments,
    started_at: DateTime<Utc>,
    created_by: AgentId,
}

impl ReadyRecording {
    fn from_db_object(recording: &crate::db::recording::Object) -> Option<Self> {
        Some(Self {
            id: recording.id(),
            rtc_id: recording.rtc_id(),
            stream_uri: recording.stream_uri().cloned()?,
            segments: recording.segments().cloned()?,
            modified_segments: recording.modified_or_segments().cloned()?,
            started_at: recording.started_at()?,
            created_by: recording.created_by().clone(),
        })
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct MinigroupReady {
    id: Uuid,
    scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    status: String,
    stream_duration: u64,
}

#[cfg(test)]
mod tests;
