use std::collections::HashMap;
use std::convert::TryInto;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
#[cfg(test)]
use mockall::{automock, predicate::*};
use reqwest::{header, Url};
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use tracing::{error, info};
use uuid::Uuid;

use super::ClientError;
use crate::config::TqAudienceSettings;
use crate::db::class::Object as Class;
use crate::db::recording::Segments;

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Normal,
    High,
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Serialize)]
struct TaskWithOptions<'a> {
    #[serde(flatten)]
    task: Task,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preroll: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postroll: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    watermark: Option<&'a str>,
}

impl<'a> TaskWithOptions<'a> {
    fn new(task: Task) -> Self {
        Self {
            task,
            to: None,
            preroll: None,
            postroll: None,
            watermark: None,
        }
    }

    fn set_audience_settings(&mut self, settings: &'a TqAudienceSettings) {
        match self.task {
            Task::TranscodeStreamToHls { .. } => {
                self.to = settings.to.as_deref();
                self.preroll = settings.preroll.as_deref();
                self.postroll = settings.postroll.as_deref();
                self.watermark = settings.watermark.as_deref();
            }
            Task::ConvertMjrDumpsToStream { .. } => {
                self.to = settings.to.as_deref();
            }
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Task {
    TranscodeStreamToHls {
        stream_id: Uuid,
        stream_uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        event_room_id: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(serialize_with = "crate::db::recording::serde::segments_option")]
        segments: Option<Segments>,
    },
    TranscodeMinigroupToHls {
        streams: Vec<TranscodeMinigroupToHlsStream>,
        host_stream_id: Uuid,
    },
    ConvertMjrDumpsToStream {
        mjr_dumps_uris: Vec<String>,
        stream_uri: String,
        stream_id: Uuid,
    },
}

impl Task {
    fn template(&self) -> &'static str {
        match self {
            Self::TranscodeStreamToHls { .. } => "transcode-stream-to-hls",
            Self::TranscodeMinigroupToHls { .. } => "transcode-minigroup-to-hls",
            Self::ConvertMjrDumpsToStream { .. } => "convert-mjr-dumps-to-stream",
        }
    }
    fn stream_id(&self) -> Option<Uuid> {
        if let Task::ConvertMjrDumpsToStream { stream_id, .. } = self {
            Some(*stream_id)
        } else {
            None
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct TranscodeMinigroupToHlsStream {
    id: Uuid,
    uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::db::recording::serde::segments_option")]
    segments: Option<Segments>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::db::recording::serde::segments_option")]
    modified_segments: Option<Segments>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "crate::db::recording::serde::segments_option")]
    pin_segments: Option<Segments>,
    #[serde(with = "crate::db::recording::serde::segments")]
    video_mute_segments: Segments,
    #[serde(with = "crate::db::recording::serde::segments")]
    audio_mute_segments: Segments,
}

impl TranscodeMinigroupToHlsStream {
    pub fn new(id: Uuid, uri: String) -> Self {
        Self {
            id,
            uri,
            offset: None,
            segments: None,
            modified_segments: None,
            pin_segments: None,
            video_mute_segments: Segments::empty(),
            audio_mute_segments: Segments::empty(),
        }
    }

    pub fn offset(self, offset: u64) -> Self {
        Self {
            offset: Some(offset),
            ..self
        }
    }

    pub fn segments(self, segments: Segments) -> Self {
        Self {
            segments: Some(segments),
            ..self
        }
    }

    pub fn modified_segments(self, segments: Segments) -> Self {
        Self {
            modified_segments: Some(segments),
            ..self
        }
    }

    pub fn pin_segments(self, pin_segments: Segments) -> Self {
        Self {
            pin_segments: Some(pin_segments),
            ..self
        }
    }

    pub fn video_mute_segments(self, video_mute_segments: Segments) -> Self {
        Self {
            video_mute_segments,
            ..self
        }
    }

    pub fn audio_mute_segments(self, audio_mute_segments: Segments) -> Self {
        Self {
            audio_mute_segments,
            ..self
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TaskComplete {
    pub tags: Option<JsonValue>,
    #[serde(flatten)]
    pub result: TaskCompleteResult,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status")]
pub enum TaskCompleteResult {
    #[serde(rename = "success")]
    Success(TaskCompleteSuccess),
    #[serde(rename = "failure")]
    Failure { error: Option<JsonValue> },
}

impl From<TaskComplete> for TaskCompleteResult {
    fn from(task_complete: TaskComplete) -> Self {
        task_complete.result
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "template")]
pub enum TaskCompleteSuccess {
    #[serde(rename = "transcode-stream-to-hls")]
    TranscodeStreamToHls(TranscodeStreamToHlsSuccess),
    #[serde(rename = "transcode-minigroup-to-hls")]
    TranscodeMinigroupToHls(TranscodeMinigroupToHlsSuccess),
    #[serde(rename = "convert-mjr-dumps-to-stream")]
    ConvertMjrDumpsToStream(ConvertMjrDumpsToStreamSuccess),
}

#[derive(Debug, Deserialize)]
pub struct ConvertMjrDumpsToStreamSuccess {
    pub stream_id: Uuid,
    pub stream_uri: String,
    pub segments: String,
}

#[derive(Debug, Deserialize)]
pub struct TranscodeStreamToHlsSuccess {
    pub stream_id: Uuid,
    pub stream_uri: String,
    pub stream_duration: String,
    pub event_room_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct TranscodeMinigroupToHlsSuccess {
    pub recording_duration: String,
}

////////////////////////////////////////////////////////////////////////////////

#[cfg_attr(test, automock)]
#[async_trait]
pub trait TqClient: Sync + Send {
    async fn create_task(
        &self,
        class: &Class,
        task: Task,
        priority: Priority,
    ) -> Result<(), ClientError>;
}

pub struct HttpTqClient {
    client: reqwest::Client,
    host: Url,
    audience_settings: HashMap<String, TqAudienceSettings>,
}

impl HttpTqClient {
    pub fn new(
        base_url: String,
        token: String,
        timeout: u64,
        audience_settings: HashMap<String, TqAudienceSettings>,
    ) -> Self {
        let host = base_url
            .parse()
            .expect("Failed to convert HttpTqClient base url");
        let mut headers = header::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {}", token).try_into().unwrap(),
        );
        headers.insert(
            http::header::CONTENT_TYPE,
            "application/json".try_into().unwrap(),
        );
        headers.insert(
            http::header::USER_AGENT,
            format!("dispatcher-{}", crate::APP_VERSION)
                .try_into()
                .unwrap(),
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout))
            .default_headers(headers)
            .build()
            .expect("Failed to build HttpTqClient");

        Self {
            client,
            host,
            audience_settings,
        }
    }

    fn build_url(&self, class: &Class, task: &Task) -> Result<Url, ClientError> {
        let route = format!(
            "api/v1/audiences/{}/tasks/{}/classrooms/{}",
            class.audience(),
            Self::task_id(class, task),
            class.id()
        );

        let url = self.host.join(&route).map_err(|e| {
            ClientError::Http(format!(
                "Failed to join base_url with route, base_url = {}, route = {}, err = {}",
                self.host, route, e
            ))
        })?;

        Ok(url)
    }

    fn build_task<'a>(
        &self,
        class: &'a Class,
        task: Task,
        priority: Priority,
    ) -> TaskPayload<'a, '_> {
        let template = task.template();

        let mut tags = class
            .tags()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| json!({"scope": class.scope().to_owned()}));

        tags.as_object_mut().and_then(|map| {
            map.insert(
                "conference_room_id".to_string(),
                json!(class.conference_room_id()),
            )
        });

        let task_with_options = if let Some(settings) = self.audience_settings.get(class.audience())
        {
            let mut t = TaskWithOptions::new(task);
            t.set_audience_settings(settings);
            t
        } else {
            TaskWithOptions::new(task)
        };

        TaskPayload {
            audience: class.audience(),
            tags,
            priority,
            bindings: task_with_options,
            template,
        }
    }

    fn task_id(class: &Class, task: &Task) -> String {
        let mut task_id = format!("{}-{}", task.template(), class.scope());
        if let Some(id) = task.stream_id() {
            task_id = format!("{}-{}", task_id, id)
        }

        task_id
    }
}

#[derive(Serialize)]
struct TaskPayload<'a, 'b> {
    audience: &'a str,
    tags: JsonValue,
    priority: Priority,
    template: &'a str,
    bindings: TaskWithOptions<'b>,
}

#[async_trait]
impl TqClient for HttpTqClient {
    async fn create_task(
        &self,
        class: &Class,
        task: Task,
        priority: Priority,
    ) -> Result<(), ClientError> {
        let url = self.build_url(class, &task)?;

        let task = self.build_task(class, task, priority);

        let json = serde_json::to_string(&task).map_err(|e| ClientError::Payload(e.to_string()))?;

        let resp = self
            .client
            .post(url)
            .body(json)
            .send()
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?;

        let status = resp.status();
        if status == http::StatusCode::OK {
            info!(class_id = %class.id(), "created tq task");
            Ok(())
        } else {
            let e = match resp.text().await {
                Err(e) => format!(
                    "Failed to create tq task and read response body, status = {:?}, error = {:?}",
                    status, e
                ),
                Ok(body) => {
                    format!(
                        "Failed to create tq task, status = {:?}, response = {:?}",
                        status, body
                    )
                }
            };
            error!(class_id = %class.id(), error = %e, "failed to create tq task");

            Err(ClientError::Payload(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use crate::clients::tq::Priority;

    #[test]
    fn test_priority_serialization() {
        #[derive(Debug, Serialize)]
        struct Test {
            priority: Priority,
        }

        let t = Test {
            priority: Priority::Normal,
        };

        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s.as_str(), "{\"priority\":\"normal\"}");
    }
}
