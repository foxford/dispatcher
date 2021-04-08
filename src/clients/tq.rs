use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use futures::AsyncReadExt;
use isahc::config::Configurable;
#[cfg(test)]
use mockall::{automock, predicate::*};
use serde_derive::Serialize;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::ClientError;
use crate::db::recording::Segments;

const PRIORITY: &str = "normal";

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, Serialize)]
pub enum Task {
    TranscodeStreamToHls {
        stream_id: Uuid,
        stream_uri: String,
        event_room_id: Option<Uuid>,
        #[serde(serialize_with = "crate::db::recording::serde::segments_option")]
        segments: Option<Segments>,
    },
    TranscodeMinigroupToHls {
        streams: Vec<TranscodeMinigroupToHlsStream>,
    },
}

impl Task {
    fn template(&self) -> &'static str {
        match self {
            Self::TranscodeStreamToHls { .. } => "transcode-stream-to-hls",
            Self::TranscodeMinigroupToHls { .. } => "transcode-minigroup-to-hls",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TranscodeMinigroupToHlsStream {
    id: Uuid,
    uri: String,
    offset: Option<u64>,
    #[serde(serialize_with = "crate::db::recording::serde::segments_option")]
    segments: Option<Segments>,
    #[serde(serialize_with = "crate::db::recording::serde::segments_option")]
    pin_segments: Option<Segments>,
}

impl TranscodeMinigroupToHlsStream {
    pub fn new(id: Uuid, uri: String) -> Self {
        Self {
            id,
            uri,
            offset: None,
            segments: None,
            pin_segments: None,
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

    pub fn pin_segments(self, pin_segments: Segments) -> Self {
        Self {
            pin_segments: Some(pin_segments),
            ..self
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

#[cfg_attr(test, automock)]
#[async_trait]
pub trait TqClient: Sync + Send {
    async fn create_task(
        &self,
        class: &crate::db::class::Object,
        task: Task,
    ) -> Result<(), ClientError>;
}

pub struct HttpTqClient {
    client: isahc::HttpClient,
    base_url: url::Url,
}

impl HttpTqClient {
    pub fn new(base_url: String, token: String, timeout: u64) -> Self {
        let base_url = url::Url::parse(&base_url).expect("Failed to convert HttpTqClient base url");

        let client = isahc::HttpClient::builder()
            .timeout(Duration::from_secs(timeout))
            .default_header(
                http::header::AUTHORIZATION.as_str(),
                format!("Bearer {}", token),
            )
            .default_header(http::header::CONTENT_TYPE.as_str(), "application/json")
            .default_header(
                http::header::USER_AGENT.as_str(),
                format!("dispatcher-{}", crate::APP_VERSION),
            )
            .build()
            .expect("Failed to build HttpTqClient");

        Self { client, base_url }
    }
}

#[derive(Serialize)]
struct TaskPayload {
    audience: String,
    tags: Option<JsonValue>,
    priority: String,
    template: String,
    bindings: Task,
}

#[async_trait]
impl TqClient for HttpTqClient {
    async fn create_task(
        &self,
        class: &crate::db::class::Object,
        task: Task,
    ) -> Result<(), ClientError> {
        let task = TaskPayload {
            audience: class.audience(),
            tags: class.tags(),
            priority: PRIORITY.into(),
            template: task.template().into(),
            bindings: task,
        };

        let route = format!(
            "/api/v1/audiences/{}/tasks/{}",
            class.audience(),
            class.scope()
        );

        let url = self.base_url.join(&route).map_err(|e| {
            ClientError::HttpError(format!(
                "Failed to join base_url with route, base_url = {}, route = {}, err = {}",
                self.base_url, route, e
            ))
        })?;

        let json =
            serde_json::to_string(&task).map_err(|e| ClientError::PayloadError(e.to_string()))?;
        let mut resp = self
            .client
            .post_async(url.as_str(), json)
            .await
            .map_err(|e| ClientError::HttpError(e.to_string()))?;
        if resp.status() == http::StatusCode::OK {
            Ok(())
        } else {
            let mut body = String::new();
            let e = if let Err(e) = resp.body_mut().read_to_string(&mut body).await {
                format!(
                    "Failed to create tq task and read response body, status = {:?}, response body read = {:?}, error = {:?}",
                    resp.status(),
                    body,
                    e
                )
            } else {
                format!(
                    "Failed to create tq task, status = {:?}, response = {:?}",
                    resp.status(),
                    body
                )
            };
            Err(ClientError::PayloadError(e))
        }
    }
}
