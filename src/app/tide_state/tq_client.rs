use std::convert::TryInto;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use futures::AsyncReadExt;
use isahc::config::Configurable;
use serde_derive::Serialize;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::ClientError;
use crate::db::recording::Segments;

#[async_trait]
pub trait TqClient: Sync + Send {
    async fn create_task(
        &self,
        webinar: &crate::db::class::Object,
        stream_id: Uuid,
        stream_uri: String,
        event_room_id: Uuid,
        segments: Segments,
    ) -> Result<(), ClientError>;
}

pub struct HttpTqClient {
    client: isahc::HttpClient,
    base_url: http::uri::Uri,
}

impl HttpTqClient {
    pub fn new(base_url: String, token: String, timeout: u64) -> Self {
        let base_url = base_url
            .try_into()
            .expect("Failed to convert HttpTqClient base url");

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
    bindings: TaskBinding,
}

#[derive(Serialize)]
struct TaskBinding {
    stream_id: Uuid,
    stream_uri: String,
    event_room_id: Uuid,
    #[serde(with = "crate::db::recording::serde::segments")]
    segments: crate::db::recording::Segments,
}

#[async_trait]
impl TqClient for HttpTqClient {
    async fn create_task(
        &self,
        webinar: &crate::db::class::Object,
        stream_id: Uuid,
        stream_uri: String,
        event_room_id: Uuid,
        segments: Segments,
    ) -> Result<(), ClientError> {
        let bindings = TaskBinding {
            stream_id,
            stream_uri,
            event_room_id,
            segments,
        };
        let task = TaskPayload {
            bindings,
            audience: webinar.audience(),
            tags: webinar.tags(),
            priority: "normal".into(),
            template: "transcode-stream-to-hls".into(),
        };

        let url = format!(
            "{}/api/v1/audiences/{}/tasks/{}",
            self.base_url,
            webinar.audience(),
            webinar.scope()
        );
        let json =
            serde_json::to_string(&task).map_err(|e| ClientError::PayloadError(e.to_string()))?;
        let mut resp = self
            .client
            .post_async(url, json)
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
