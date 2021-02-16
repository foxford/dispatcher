use anyhow::Result;
use async_trait::async_trait;
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
    client: surf::Client,
}

impl HttpTqClient {
    pub fn new(base_url: surf::Url) -> Self {
        let mut client = surf::Client::new();
        client.set_base_url(base_url);

        Self { client }
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
        /*
                {
            "template"=>"transcode-stream-to-hls",
            "priority"=>"normal",
            "bindings"=>{
                "stream_id"=>"14fc6419-0b3e-4c34-b8de-66c6bb424c50",
                "stream_uri"=>"s3://origin.webinar.foxford.ru/14fc6419-0b3e-4c34-b8de-66c6bb424c50.source.webm",
                "event_room_id"=>"379dc6eb-e6ac-4e61-8e01-4041be6fc8df",
                "segments"=>[[0, 9242256]]
            },
            "tags"=>{
                "webinar_id"=>121796,
                "scope"=>"Z2lkOi8vc3RvZWdlL1VsbXM6OlJvb21zOjpXZWJpbmFyLzEzMTY4MA"
            },
            "audience"=>"foxford.ru",
            "id"=>"Z2lkOi8vc3RvZWdlL1VsbXM6OlJvb21zOjpXZWJpbmFyLzEzMTY4MA",
        }*/
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
            "/api/v1/audiences/{}/tasks/{}",
            webinar.audience(),
            webinar.scope()
        );
        let json =
            serde_json::to_string(&task).map_err(|e| ClientError::PayloadError(e.to_string()))?;
        let mut resp = self
            .client
            .post(url)
            .body(json)
            .send()
            .await
            .map_err(|e| ClientError::HttpError(e.to_string()))?;
        if resp.status() == surf::StatusCode::Ok {
            Ok(())
        } else {
            let e = format!(
                "Failed to create tq task, status = {:?}, response = {:?}",
                resp.status(),
                resp.body_string().await
            );
            Err(ClientError::PayloadError(e))
        }
    }
}
