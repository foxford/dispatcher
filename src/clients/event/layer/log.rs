use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Result;
use tower::{Layer, Service};
use tracing::instrument;

use super::*;

// Layer for Log
pub struct LogLayer {
    remote_service_label: &'static str,
}

impl LogLayer {
    pub fn new(remote_service_label: &'static str) -> Self {
        Self {
            remote_service_label,
        }
    }
}

impl<S> Layer<S> for LogLayer {
    type Service = LogMiddleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        LogMiddleware {
            service,
            remote_service_label: self.remote_service_label,
        }
    }
}

// Log middleware, wraps inner service and logs request and response to the service
// requests and response successes are logged at info, response errors - at error
#[derive(Debug, Clone)]
pub struct LogMiddleware<S> {
    remote_service_label: &'static str,
    service: S,
}

impl<S, Req> Service<Req> for LogMiddleware<S>
where
    Req: MqttRequest,
    S: Service<Req>,
    <S as Service<Req>>::Error: std::fmt::Debug,
    <S as Service<Req>>::Future: Send + Sync + 'static,
{
    type Response = <S as Service<Req>>::Response;
    type Error = <S as Service<Req>>::Error;
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

    #[instrument(skip_all, fields(method, payload, remote_service, response_error))]
    fn call(&mut self, request: Req) -> Self::Future {
        tracing::Span::current().record(
            "remote_service",
            &tracing::field::display(self.remote_service_label),
        );

        tracing::Span::current().record("method", &tracing::field::display(request.method()));
        tracing::Span::current().record("payload", &tracing::field::display(request.payload()));
        let fut = self.service.call(request);
        Box::pin(async move {
            tracing::info!("mqtt request");
            let response = fut.await;
            if let Err(e) = &response {
                tracing::Span::current().record("response_error", &tracing::field::debug(e));

                tracing::error!("mqtt request failed");
            } else {
                tracing::info!("mqtt request done");
            }
            response
        })
    }
}
