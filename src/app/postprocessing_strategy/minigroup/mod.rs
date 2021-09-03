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
use uuid::Uuid;

use crate::clients::event::{Event, EventData, RoomAdjustResult};
use crate::clients::tq::{
    Task as TqTask, TranscodeMinigroupToHlsStream, TranscodeMinigroupToHlsSuccess,
};
use crate::db::class::Object as Class;
use crate::db::recording::Segments;
use crate::{app::AppContext, clients::conference::ConfigSnapshot};

use super::{
    shared_helpers, MjrDumpsUploadReadyData, MjrDumpsUploadResult, TranscodeSuccess, UploadedStream,
};

const NS_IN_MS: i64 = 1000000;
const PIN_EVENT_TYPE: &str = "pin";
const HOST_EVENT_TYPE: &str = "host";
// TODO: make configurable for each audience.
const PREROLL_OFFSET: i64 = 4018;

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
        let recordings = {
            let mut conn = self.ctx.get_conn().await?;
            crate::db::recording::StreamUploadUpdateQuery::new(
                self.minigroup.id(),
                stream.id,
                stream.segments,
                stream.uri,
                stream.started_at,
            )
            .execute(&mut conn)
            .await?;
            crate::db::recording::RecordingListQuery::new(self.minigroup.id())
                .execute(&mut conn)
                .await?
        };
        let ready_recordings = recordings
            .iter()
            .filter_map(|recording| ReadyRecording::from_db_object(recording))
            .collect::<Vec<_>>();
        if recordings.len() != ready_recordings.len() {
            return Ok(());
        }

        // Find host stream id.
        let host = match self.find_host(self.minigroup.event_room_id()).await? {
            None => bail!("No host in room"),
            Some(agent_id) => agent_id,
        };

        call_adjust(
            self.ctx.clone(),
            self.minigroup.event_room_id(),
            ready_recordings,
            host,
        )
        .await?;
        Ok(())
    }

    async fn handle_adjust(&self, room_adjust_result: RoomAdjustResult) -> Result<()> {
        match room_adjust_result {
            RoomAdjustResult::Success {
                original_room_id,
                modified_room_id,
                modified_segments,
            } => {
                // Find host stream id.
                let host = match self.find_host(modified_room_id).await? {
                    None => bail!("No host in room"),
                    Some(agent_id) => agent_id,
                };

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

                    let recordings = crate::db::recording::AdjustMinigroupUpdateQuery::new(
                        self.minigroup.id(),
                        modified_segments,
                        host.clone(),
                    )
                    .execute(&mut txn)
                    .await?;

                    txn.commit().await?;
                    recordings
                        .into_iter()
                        .map(|recording| ReadyRecording::from_db_object(&recording))
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| anyhow!("Not all recordings are ready"))?
                };

                self.ctx
                    .event_client()
                    .dump_room(modified_room_id)
                    .await
                    .context("Dump room event failed")?;

                let maybe_host_recording = recordings
                    .iter()
                    .find(|recording| recording.created_by == host);

                let host_stream = match maybe_host_recording {
                    // Host has been set but there's no recording, skip transcoding.
                    None => bail!("No host stream id in room"),
                    Some(recording) => recording,
                };

                // Find the earliest recording.
                let earliest_recording = recordings
                    .iter()
                    .min_by(|a, b| a.started_at.cmp(&b.started_at))
                    .ok_or_else(|| anyhow!("No recordings"))?;

                // Fetch event room opening time for events' offset calculation.
                let modified_event_room = self
                    .ctx
                    .event_client()
                    .read_room(modified_room_id)
                    .await
                    .context("Failed to read modified event room")?;

                match modified_event_room.time {
                    (Bound::Included(_), _) => (),
                    _ => bail!("Wrong event room opening time"),
                };

                // Fetch pin events for building pin segments.
                let pin_events = self
                    .ctx
                    .event_client()
                    .list_events(modified_room_id, PIN_EVENT_TYPE)
                    .await
                    .context("Failed to get pin events for room")?;

                let conference_room_id = self.minigroup.conference_room_id().ok_or_else(|| {
                    anyhow!(
                        "Minigroup {} must have a conference room id",
                        self.minigroup.id()
                    )
                })?;

                // Fetch writer config snapshots for building muted segments.
                let mute_events = self
                    .ctx
                    .conference_client()
                    .read_config_snapshots(conference_room_id)
                    .await
                    .context("Failed to get writer config snapshots for room")?;

                // Build streams for template bindings.
                let streams = recordings
                    .iter()
                    .map(|recording| {
                        let event_room_offset = recording.started_at
                            - (host_stream.started_at - Duration::milliseconds(PREROLL_OFFSET));

                        let recording_offset = recording.started_at - earliest_recording.started_at;

                        build_stream(
                            recording,
                            &pin_events,
                            event_room_offset,
                            recording_offset,
                            &mute_events,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let host_stream_id = host_stream.rtc_id;

                // Create a tq task.
                let task = TqTask::TranscodeMinigroupToHls {
                    streams,
                    host_stream_id,
                };

                self.ctx
                    .tq_client()
                    .create_task(&self.minigroup, task)
                    .await
                    .context("TqClient create task failed")
            }
            RoomAdjustResult::Error { error } => {
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
                    id: self.minigroup.id(),
                    scope: self.minigroup.scope().to_owned(),
                    tags: self.minigroup.tags().map(ToOwned::to_owned),
                    status: "success".to_string(),
                    recording_duration,
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
                )
                .await?
        }
        Ok(())
    }
}

impl MinigroupPostprocessingStrategy {
    async fn find_host(&self, event_room_id: Uuid) -> Result<Option<AgentId>> {
        let host_events = self
            .ctx
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

async fn call_adjust(
    ctx: Arc<dyn AppContext>,
    room_id: Uuid,
    recordings: Vec<ReadyRecording>,
    host: AgentId,
) -> Result<()> {
    let host_recording = recordings
        .into_iter()
        .find(|recording| recording.created_by == host)
        .ok_or_else(|| anyhow!("No host recording"))?;

    ctx.event_client()
        .adjust_room(
            room_id,
            host_recording.started_at,
            host_recording.segments,
            PREROLL_OFFSET,
        )
        .await
        .map_err(|err| anyhow!("Failed to adjust room, id = {}: {}", room_id, err))?;

    Ok(())
}

fn build_stream(
    recording: &ReadyRecording,
    pin_events: &[Event],
    event_room_offset: Duration,
    recording_offset: Duration,
    configs_changes: &[ConfigSnapshot],
) -> anyhow::Result<TranscodeMinigroupToHlsStream> {
    let event_room_offset = event_room_offset.num_milliseconds();
    let mut pin_segments = vec![];
    let mut pin_start = None;

    let recording_end = match recording
        .segments
        .last()
        .map(|range| range.end)
        .ok_or_else(|| anyhow!("Recording segments have no end?"))?
    {
        Bound::Included(t) | Bound::Excluded(t) => t,
        Bound::Unbounded => bail!("Unbounded recording end"),
    };

    for event in pin_events {
        if let EventData::Pin(data) = event.data() {
            // Shift from the event room's dimension to the recording's dimension.
            let occurred_at = event.occurred_at() as i64 / NS_IN_MS - event_room_offset;

            if data
                .agent_id()
                .map(|aid| *aid == recording.created_by)
                .unwrap_or(false)
                && pin_start.is_none()
            {
                // Stream has got pinned.
                pin_start = Some(occurred_at);
            } else if let Some(pinned_at) = pin_start {
                // Stream has got unpinned.
                pin_segments.push((Bound::Included(pinned_at), Bound::Excluded(occurred_at)));
                pin_start = None;
            }
        }
    }

    // If the stream hasn't got unpinned since some moment then add a pin segment to the end
    // of the recording to keep it pinned.
    if let Some(start) = pin_start {
        pin_segments.push((Bound::Included(start), Bound::Excluded(recording_end)));
    }

    // We need only changes for the recording that fall into recording span
    let changes = configs_changes.iter().filter(|snapshot| {
        let m = (snapshot.created_at - recording.started_at).num_milliseconds();
        m > 0 && m < recording_end && snapshot.rtc_id == recording.rtc_id
    });
    let mut video_mute_start = None;
    let mut audio_mute_start = None;
    let mut video_mute_segments = vec![];
    let mut audio_mute_segments = vec![];

    for change in changes {
        if change.send_video == Some(false) && video_mute_start.is_none() {
            video_mute_start = Some(change);
        }

        if change.send_video == Some(true) && video_mute_start.is_some() {
            let start = video_mute_start.take().unwrap();
            let muted_at = (start.created_at - recording.started_at).num_milliseconds();
            let unmuted_at = (change.created_at - recording.started_at).num_milliseconds();
            video_mute_segments.push((Bound::Included(muted_at), Bound::Excluded(unmuted_at)));
        }

        if change.send_audio == Some(false) && audio_mute_start.is_none() {
            audio_mute_start = Some(change);
        }

        if change.send_audio == Some(true) && audio_mute_start.is_some() {
            let start = audio_mute_start.take().unwrap();
            let muted_at = (start.created_at - recording.started_at).num_milliseconds();
            let unmuted_at = (change.created_at - recording.started_at).num_milliseconds();
            audio_mute_segments.push((Bound::Included(muted_at), Bound::Excluded(unmuted_at)));
        }
    }

    // If last mute segment was left open, close it with recording end
    if let Some(start) = video_mute_start {
        let muted_at = (start.created_at - recording.started_at).num_milliseconds();
        video_mute_segments.push((Bound::Included(muted_at), Bound::Excluded(recording_end)));
    }

    if let Some(start) = audio_mute_start {
        let muted_at = (start.created_at - recording.started_at).num_milliseconds();
        audio_mute_segments.push((Bound::Included(muted_at), Bound::Excluded(recording_end)));
    }

    let v = TranscodeMinigroupToHlsStream::new(recording.rtc_id, recording.stream_uri.to_owned())
        .offset(recording_offset.num_milliseconds() as u64)
        .segments(recording.segments.to_owned())
        .pin_segments(pin_segments.into())
        .video_mute_segments(video_mute_segments.into())
        .audio_mute_segments(audio_mute_segments.into());

    Ok(v)
}

#[derive(Debug)]
struct ReadyRecording {
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
    recording_duration: u64,
}

#[cfg(test)]
mod tests;
