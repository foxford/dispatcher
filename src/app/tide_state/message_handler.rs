use std::sync::Arc;

use anyhow::{Context, Result};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use svc_agent::mqtt::{
    IncomingEvent, IncomingResponse, IntoPublishableMessage, OutgoingEvent,
    OutgoingEventProperties, ShortTermTimingProperties,
};
use svc_agent::request::Dispatcher;
use tracing::{debug, error, field::display, info, instrument, warn, Span};
use uuid::Uuid;

use super::AppContext;
use crate::{
    app::error::{ErrorExt, ErrorKind as AppErrorKind},
    clients::{event::RoomAdjustResult, tq::TaskCompleteResult},
    db::recording::Segments,
};
use crate::{
    app::metrics::MqttMetrics,
    db::class::{ClassType, Object as Class},
};
use crate::{app::postprocessing_strategy, clients::tq::TaskCompleteSuccess};
use crate::{app::postprocessing_strategy::TranscodeSuccess, clients::event::RoomAdjust};
use crate::{app::postprocessing_strategy::UploadedStream, clients::tq::TaskComplete};

pub struct MessageHandler {
    ctx: Arc<dyn AppContext>,
    dispatcher: Arc<Dispatcher>,
}

impl MessageHandler {
    pub fn new(ctx: Arc<dyn AppContext>, dispatcher: Arc<Dispatcher>) -> Self {
        Self { ctx, dispatcher }
    }

    pub fn ctx(&self) -> &dyn AppContext {
        self.ctx.as_ref()
    }

    pub async fn handle_response(&self, data: IncomingResponse<String>) {
        match IncomingResponse::convert::<JsonValue>(data) {
            Ok(message) => {
                if let Err(e) = self.dispatcher.response(message) {
                    error!("Failed to commit response, reason = {:?}", e);
                }
            }
            Err(e) => error!("Failed to parse response, reason = {:?}", e),
        }
    }

    #[instrument(
        level = "warn",
        skip(self, data),
        fields(
            payload = %data.payload(),
            label = %data.properties().label().map(|s| format!("Some({})", s)).unwrap_or_else(|| "None".into())
        )
    )]
    pub async fn handle_event(&self, data: IncomingEvent<String>, topic: String) {
        warn!("Incoming event",);

        let topic = topic.split('/').collect::<Vec<&str>>();

        let label = data.properties().label().map(|x| x.to_owned());
        let result = match label.as_deref() {
            Some("room.close") => self
                .handle_close(data, topic)
                .await
                .error(AppErrorKind::ClassClosingFailed),
            Some("room.upload") => self
                .handle_stream_upload(data)
                .await
                .error(AppErrorKind::TranscodingFlowFailed),
            Some("room.adjust") => self
                .handle_adjust(data)
                .await
                .error(AppErrorKind::TranscodingFlowFailed),
            Some("task.complete") => self
                .handle_tq_task_completion(data)
                .await
                .error(AppErrorKind::TranscodingFlowFailed),
            Some("room.dump_events") => self
                .handle_dump_events(data)
                .await
                .error(AppErrorKind::TranscodingFlowFailed),
            Some("edition.commit") => self
                .handle_edition_commit(data)
                .await
                .error(AppErrorKind::EditionFailed),
            _label => {
                debug!("Unexpected incoming event",);
                Ok(())
            }
        };
        MqttMetrics::observe_event_result(&result, label.as_deref());

        if let Err(e) = result {
            error!("Event handler failed, err = {:?}", e);
            e.notify_sentry();
        } else {
            info!("Event handler done")
        }
    }

    #[instrument(skip(self, data, topic), fields(payload_id, class_id))]
    async fn handle_close(&self, data: IncomingEvent<String>, topic: Vec<&str>) -> Result<()> {
        let payload = serde_json::from_str::<RoomClose>(&data.extract_payload())?;
        Span::current().record("payload_id", &display(payload.id));
        let mut conn = self.ctx.get_conn().await?;

        warn!("Close event handler started");

        let query = match topic.get(1) {
            Some(app) if app.starts_with("event.") => {
                crate::db::class::ReadQuery::by_event_room(payload.id)
            }
            Some(app) if app.starts_with("conference.") => {
                crate::db::class::ReadQuery::by_conference_room(payload.id)
            }
            _ => return Ok(()),
        };

        let class = query
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Class not found by id from payload = {:?}", payload,))?;

        Span::current().record("class_id", &display(class.id()));
        warn!("Close event found class",);

        let label = match class.kind() {
            ClassType::P2P => "p2p.stop",
            ClassType::Minigroup => "minigroup.stop",
            ClassType::Webinar => "webinar.stop",
        };

        crate::db::class::RoomCloseQuery::new(class.id(), payload.timed_out.unwrap_or(false))
            .execute(&mut conn)
            .await?;

        warn!("Close event, room close query done");

        let timing = ShortTermTimingProperties::new(chrono::Utc::now());
        let props = OutgoingEventProperties::new(label, timing);
        let path = format!("audiences/{}/events", class.audience());

        let payload = ClassStop {
            tags: class.tags().map(ToOwned::to_owned),
            scope: class.scope().to_owned(),
            id: class.id(),
        };

        warn!("Close event, sending notification, payload = {:?}", payload);

        let event = OutgoingEvent::broadcast(payload, props, &path);
        let boxed_event = Box::new(event) as Box<dyn IntoPublishableMessage + Send>;

        self.ctx
            .publisher()
            .publish(boxed_event)
            .with_context(|| format!("Failed to publish {} event", label))
    }

    async fn handle_stream_upload(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let room_upload = serde_json::from_str::<RoomUpload>(&payload)?;

        let class = {
            let mut conn = self.ctx.get_conn().await?;

            match crate::db::class::ReadQuery::by_conference_room(room_upload.id)
                .execute(&mut conn)
                .await?
            {
                Some(class) => class,
                None => {
                    warn!(
                        conference_room_id = %room_upload.id,
                        "Class not found by conference room id, probably recreated class",
                    );
                    return Ok(());
                }
            }
        };

        // Do not run transcoding for replicas
        if class.original_class_id().is_some() {
            return Ok(());
        }

        postprocessing_strategy::get(self.ctx.clone(), class)?
            .handle_mjr_dumps_upload(room_upload.rtcs)
            .await
    }

    async fn handle_edition_commit(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let commit = serde_json::from_str::<EditionCommit>(&payload)?;
        let class = if let EditionCommitResult::Success { source_room_id, .. } = &commit.result {
            self.get_class_original_by_room_id(*source_room_id).await
        } else {
            Err(anyhow!("Commit result unsucessful: {:?}", commit))
        }?;

        postprocessing_strategy::get(self.ctx.clone(), class)?
            .handle_adjust(commit.result.into_adjust_result())
            .await
    }

    async fn handle_adjust(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let room_adjust: RoomAdjust = serde_json::from_str(&payload)?;

        // Do not run adjust for replicas
        let class = self
            .get_original_class_by_room_id(room_adjust.room_id())
            .await?;

        postprocessing_strategy::get(self.ctx.clone(), class)?
            .handle_adjust(room_adjust.into())
            .await
    }

    async fn handle_tq_task_completion(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let task: TaskComplete = serde_json::from_str(&payload)?;
        match task.result {
            TaskCompleteResult::Success(success) => {
                let class = self
                    .get_class_from_tags_by_conference_id(task.tags.as_ref())
                    .await?;
                let class = match class {
                    Some(class) => class,
                    None => return Ok(()),
                };
                match success {
                    TaskCompleteSuccess::TranscodeStreamToHls(result) => {
                        postprocessing_strategy::get(self.ctx.clone(), class)?
                            .handle_transcoding_completion(TranscodeSuccess::TranscodeStreamToHls(
                                result,
                            ))
                            .await
                    }
                    TaskCompleteSuccess::TranscodeMinigroupToHls(result) => {
                        postprocessing_strategy::get(self.ctx.clone(), class)?
                            .handle_transcoding_completion(
                                TranscodeSuccess::TranscodeMinigroupToHls(result),
                            )
                            .await
                    }
                    TaskCompleteSuccess::ConvertMjrDumpsToStream(result) => {
                        let stream = UploadedStream::from_convert_result(&result)?;
                        postprocessing_strategy::get(self.ctx.clone(), class)?
                            .handle_stream_upload(stream)
                            .await
                    }
                }
            }
            TaskCompleteResult::Failure { error } => {
                error!(?error, "Tq task error");
                Ok(())
            }
        }
    }

    async fn handle_dump_events(&self, data: IncomingEvent<String>) -> Result<()> {
        let payload = data.extract_payload();
        let dump_events: DumpEvents = serde_json::from_str(&payload)?;
        match dump_events.result {
            DumpEventsResult::Success { room_id, s3_uri } => {
                let mut conn = self.ctx.get_conn().await?;
                crate::db::class::UpdateDumpEventsQuery::new(room_id, s3_uri)
                    .execute(&mut conn)
                    .await?;
                Ok(())
            }
            DumpEventsResult::Error { error } => {
                bail!("Dump failed, err = {:#?}", error);
            }
        }
    }

    async fn get_class_from_tags_by_conference_id(
        &self,
        tags: Option<&JsonValue>,
    ) -> Result<Option<Class>> {
        let conference_room_id = tags
            .and_then(|tags| tags.get("conference_room_id"))
            .and_then(|s| s.as_str())
            .ok_or_else(|| anyhow!("No conference room id in tags"))
            .and_then(|s| {
                Uuid::parse_str(s)
                    .map_err(|e| anyhow!("Failed to parse conference room id uuid, err = {:?}", e))
            })?;

        let mut conn = self.ctx.get_conn().await?;

        match crate::db::class::ReadQuery::by_conference_room(conference_room_id)
            .execute(&mut conn)
            .await?
        {
            Some(class) => Ok(Some(class)),
            None => {
                warn!(
                    %conference_room_id,
                    "Class not found by conference room id, probably recreated class",
                );
                Ok(None)
            }
        }
    }

    async fn get_original_class_by_room_id(&self, room_id: Uuid) -> Result<Class> {
        let mut conn = self.ctx.get_conn().await?;

        crate::db::class::ReadQuery::by_event_room(room_id)
            .original()
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Class not found by event room id = {}", room_id))
    }

    async fn get_class_original_by_room_id(&self, room_id: Uuid) -> Result<Class> {
        let mut conn = self.ctx.get_conn().await?;

        crate::db::class::ReadQuery::by_original_event_room(room_id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Class not found by original event room id = {}", room_id))
    }
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct EditionCommit {
    tags: Option<JsonValue>,
    #[serde(flatten)]
    result: EditionCommitResult,
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum EditionCommitResult {
    Success {
        source_room_id: Uuid,
        committed_room_id: Uuid,
        #[serde(with = "crate::db::recording::serde::segments")]
        modified_segments: Segments,
    },
    Error {
        error: JsonValue,
    },
}

impl EditionCommitResult {
    fn into_adjust_result(self) -> RoomAdjustResult {
        match self {
            EditionCommitResult::Success {
                source_room_id,
                committed_room_id,
                modified_segments,
            } => RoomAdjustResult::Success {
                original_room_id: source_room_id,
                modified_room_id: committed_room_id,
                modified_segments,
                cut_original_segments: vec![].into(),
            },
            EditionCommitResult::Error { error } => RoomAdjustResult::Error { error },
        }
    }
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct RoomClose {
    id: Uuid,
    audience: String,
    timed_out: Option<bool>,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: crate::db::class::BoundedDateTimeTuple,
}

#[derive(Deserialize, Debug)]
struct RoomUpload {
    id: Uuid,
    rtcs: Vec<postprocessing_strategy::MjrDumpsUploadResult>,
}

#[derive(Serialize, Debug)]
struct ClassStop {
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    scope: String,
    id: Uuid,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "status", content = "result")]
#[serde(rename_all = "snake_case")]
enum DumpEventsResult {
    Success { room_id: Uuid, s3_uri: String },
    Error { error: JsonValue },
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct DumpEvents {
    tags: Option<JsonValue>,
    #[serde(flatten)]
    result: DumpEventsResult,
}
