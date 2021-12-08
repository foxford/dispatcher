use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;
use serde_json::Value as JsonValue;
use tower::{Layer, Service};

use super::*;

pub struct ToJsonLayer;

impl ToJsonLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for ToJsonLayer {
    type Service = ToJsonMiddleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        ToJsonMiddleware { service }
    }
}

#[derive(Debug, Clone)]
pub struct ToJsonMiddleware<S> {
    service: S,
}

#[allow(clippy::type_complexity)]
impl<S, Request> Service<Request> for ToJsonMiddleware<S>
where
    Request: MqttRequest,
    S: Service<JsonMqttRequest, Response = JsonValue, Error = ClientError>,
    <S as Service<JsonMqttRequest>>::Future: Send + Sync + 'static,
{
    type Response = <Request as MqttRequest>::Response;
    type Error = ClientError;
    type Future = Pin<
        Box<
            dyn futures::Future<Output = Result<Self::Response, Self::Error>>
                + Send
                + Sync
                + 'static,
        >,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let request = request.into();
        let fut = self.service.call(request);
        Box::pin(async move {
            let json_body: JsonValue = fut.await.map_err::<Self::Error, _>(Into::into)?;
            serde_json::from_value::<Self::Response>(json_body)
                .map_err(|e| ClientError::Payload(e.to_string()))
        })
    }
}
