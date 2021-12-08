use std::collections::HashMap;
use std::pin::Pin;
use std::sync::RwLock;
use std::task::{Context, Poll};

use anyhow::Result;
use once_cell::sync::{Lazy, OnceCell};
use prometheus::{register_int_counter, register_int_counter_vec, IntCounter, IntCounterVec};
use tower::{Layer, Service};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ServiceLabel(&'static str);

static LAYER_METRICS: Lazy<RwLock<HashMap<ServiceLabel, OnceCell<Metrics>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Clone)]
struct Metrics {
    requests_metric: IntCounter,
    response_metrics: IntCounterVec,
}

pub struct MetricsLayer {
    service_prefix: &'static str,
}

impl MetricsLayer {
    pub fn new(service_prefix: &'static str) -> Self {
        Self { service_prefix }
    }

    fn init_metrics(&self) -> Metrics {
        let label = ServiceLabel(self.service_prefix);
        LAYER_METRICS
            .try_write()
            .unwrap()
            .entry(label)
            .or_insert_with(|| {
                let cell = OnceCell::new();
                let requests_metric = register_int_counter!(
                    format!("{}_client_requests_total", self.service_prefix),
                    format!("{} client requests made counter", self.service_prefix),
                )
                .expect("Failed to create counter");
                let response_metrics = register_int_counter_vec!(
                    format!("{}_client_responses", self.service_prefix),
                    format!("{} requests counter", self.service_prefix),
                    &["success"]
                )
                .expect("Failed to create counter");
                let metrics = Metrics {
                    requests_metric,
                    response_metrics,
                };
                cell.set(metrics).unwrap_or_else(|_| {
                    panic!("Failed to init metrics cell for {}", self.service_prefix)
                });
                cell
            });

        let metrics = LAYER_METRICS
            .try_read()
            .unwrap()
            .get(&ServiceLabel(self.service_prefix))
            .unwrap()
            .get()
            .unwrap()
            .clone();

        metrics
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsMiddleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        let metrics = self.init_metrics();
        MetricsMiddleware { metrics, service }
    }
}

#[derive(Debug, Clone)]
pub struct MetricsMiddleware<S> {
    service: S,
    metrics: Metrics,
}

#[allow(clippy::type_complexity)]
impl<S, Req> Service<Req> for MetricsMiddleware<S>
where
    Req: MqttRequest,
    S: Service<Req>,
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

    fn call(&mut self, request: Req) -> Self::Future {
        self.metrics.requests_metric.inc();
        let response_metrics = self.metrics.response_metrics.clone();
        let fut = self.service.call(request);
        Box::pin(async move {
            let response = fut.await;

            if response.is_err() {
                response_metrics
                    .get_metric_with_label_values(&["error"])
                    .expect("Failed to get metric")
                    .inc();
            } else {
                response_metrics
                    .get_metric_with_label_values(&["ok"])
                    .expect("Failed to get metric")
                    .inc();
            }
            response
        })
    }
}
