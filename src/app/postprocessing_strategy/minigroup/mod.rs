use std::sync::Arc;
use std::{collections::HashMap, ops::Bound};

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
use crate::db::class::Object as Class;
use crate::db::recording::Segments;
use crate::{app::AppContext, clients::conference::ConfigSnapshot};
use crate::{
    clients::event::{EventData, RoomAdjustResult},
    db::class::ClassType,
};

use super::{
    shared_helpers, MjrDumpsUploadReadyData, MjrDumpsUploadResult, TranscodeSuccess, UploadedStream,
};

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
            .filter_map(|r| {
                Some(AdjustRecording {
                    id: r.id(),
                    rtc_id: r.rtc_id(),
                    host: (host_recording.created_by == *r.created_by()),
                    segments: r.segments()?.to_owned(),
                    started_at: r.started_at()?,
                    created_by: r.created_by().to_owned(),
                })
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
                recordings: record_segments,
                ..
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

                    let records =
                        crate::db::recording::RecordingListQuery::new(self.minigroup.id())
                            .execute(&mut txn)
                            .await?;

                    let host_record = records.iter().find(|r| *r.created_by() == host);
                    let host_record = match host_record {
                        Some(r) => r,
                        None => {
                            bail!("Host record is not found");
                        }
                    };

                    let host_modified_segments = record_segments
                        .iter()
                        .find(|rs| rs.id == host_record.id())
                        .map(|rs| rs.modified_segments.clone());
                    let host_modified_segments = match host_modified_segments {
                        Some(ss) => ss,
                        None => {
                            bail!("Host segments are not found");
                        }
                    };

                    let records = crate::db::recording::AdjustMinigroupUpdateQuery::new(
                        self.minigroup.id(),
                        host_modified_segments,
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
                    record_segments,
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

    // Fetch event room opening time for events' offset calculation.
    let modified_event_room = ctx
        .event_client()
        .read_room(modified_event_room_id)
        .await
        .context("Failed to read modified event room")?;

    match modified_event_room.time {
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
        .filter_map(|r| {
            let recording = recordings_map.get(&r.id)?;

            let recording_offset = recording.started_at - earliest_recording.started_at;
            Some(build_stream(recording, r, recording_offset))
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
        .pin_segments(recording_segments.pin_segments.clone())
        .video_mute_segments(recording_segments.video_mute_segments.clone())
        .audio_mute_segments(recording_segments.audio_mute_segments.clone());

    Ok(v)
}

#[derive(Debug)]
struct ReadyRecording {
    id: Uuid,
    rtc_id: Uuid,
    stream_uri: String,
    segments: Segments,
    // TODO: remove this field if it's really not used
    #[allow(unused)]
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
