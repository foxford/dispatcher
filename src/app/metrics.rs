use std::collections::HashMap;
use std::convert::{Infallible, TryFrom};
use std::iter::FromIterator;
use std::sync::Arc;

use axum::routing::Router;
use chrono::Duration;
use hyper::{Method, StatusCode};
use once_cell::sync::{Lazy, OnceCell};
use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter_vec, Histogram,
    HistogramTimer, HistogramVec, IntCounter, IntCounterVec,
};
use prometheus_static_metric::make_static_metric;
use tracing::error;

use super::error::Error;

make_static_metric! {
    struct MqttStats: IntCounter {
        "method" => {
            room_close,
            room_upload,
            room_adjust,
            task_complete,
            room_dump_events,
            edition_commit,
        },
        "status" => {
            success,
            failure,
        },
    }
}

pub struct AuthMetrics;

impl AuthMetrics {
    pub fn start_timer() -> HistogramTimer {
        METRICS.authz_time.start_timer()
    }
}

pub struct MqttMetrics;

impl MqttMetrics {
    pub fn observe_disconnect() {
        METRICS.disconnect.inc()
    }

    pub fn observe_reconnect() {
        METRICS.reconnection.inc()
    }

    pub fn observe_connection_error() {
        METRICS.connection_error.inc()
    }

    pub fn observe_event_result(result: &Result<(), Error>, label: Option<&str>) {
        match label {
            Some("room.close") => {
                if result.is_err() {
                    METRICS.stats.room_close.failure.inc();
                } else {
                    METRICS.stats.room_close.success.inc();
                }
            }
            Some("room.upload") => {
                if result.is_err() {
                    METRICS.stats.room_upload.failure.inc();
                } else {
                    METRICS.stats.room_upload.success.inc();
                }
            }
            Some("room.adjust") => {
                if result.is_err() {
                    METRICS.stats.room_adjust.failure.inc();
                } else {
                    METRICS.stats.room_adjust.success.inc();
                }
            }
            Some("task.complete") => {
                if result.is_err() {
                    METRICS.stats.task_complete.failure.inc();
                } else {
                    METRICS.stats.task_complete.success.inc();
                }
            }
            Some("room.dump_events") => {
                if result.is_err() {
                    METRICS.stats.room_dump_events.failure.inc();
                } else {
                    METRICS.stats.room_dump_events.success.inc();
                }
            }
            Some("edition.commit") => {
                if result.is_err() {
                    METRICS.stats.edition_commit.failure.inc();
                } else {
                    METRICS.stats.edition_commit.success.inc();
                }
            }
            _ => {}
        }
    }
}

pub trait AuthorizeMetrics {
    fn measure(self) -> Self;
}

impl AuthorizeMetrics for Result<Duration, svc_authz::error::Error> {
    fn measure(self) -> Self {
        if let Ok(Ok(d)) = self.as_ref().map(|d| d.to_std()) {
            let nanos = f64::from(d.subsec_nanos()) / 1e9;
            METRICS.authz_time.observe(d.as_secs() as f64 + nanos)
        }
        self
    }
}

static METRICS: Lazy<Metrics> = Lazy::new(Metrics::new);

struct Metrics {
    duration_vec: HistogramVec,
    status_vec: IntCounterVec,
    stats: MqttStats,
    connection_error: IntCounter,
    disconnect: IntCounter,
    reconnection: IntCounter,
    authz_time: Histogram,
}

impl Metrics {
    pub fn new() -> Self {
        let mqtt_stats =
            register_int_counter_vec!("mqtt_stats", "Mqtt stats", &["method", "status"])
                .expect("Can't create stats metrics");
        let mqtt_errors =
            register_int_counter_vec!("mqtt_messages", "Mqtt message types", &["status"])
                .expect("Bad mqtt messages metric");
        Metrics {
            duration_vec: register_histogram_vec!(
                "request_duration",
                "Request duration",
                &["path", "method"]
            )
            .expect("Can't create stats metrics"),
            status_vec: register_int_counter_vec!(
                "request_stats",
                "Request stats",
                &["path", "method", "status_code"]
            )
            .expect("Can't create stats metrics"),
            stats: MqttStats::from(&mqtt_stats),
            connection_error: mqtt_errors.with_label_values(&["connection_error"]),
            disconnect: mqtt_errors.with_label_values(&["disconnect"]),
            reconnection: mqtt_errors.with_label_values(&["reconnect"]),
            authz_time: register_histogram!("auth_time", "Authorization time")
                .expect("Bad authz hist"),
        }
    }
}

#[derive(Clone)]
struct MethodStatusCounters(Arc<HashMap<(Method, StatusCode), OnceCell<IntCounter>>>);

impl FromIterator<((Method, StatusCode), OnceCell<IntCounter>)> for MethodStatusCounters {
    fn from_iter<T: IntoIterator<Item = ((Method, StatusCode), OnceCell<IntCounter>)>>(
        iter: T,
    ) -> MethodStatusCounters {
        let mut map: HashMap<(Method, StatusCode), OnceCell<IntCounter>> = HashMap::new();
        map.extend(iter);
        MethodStatusCounters(Arc::new(map))
    }
}

impl MethodStatusCounters {
    fn inc_counter(&self, method: Method, status: StatusCode, path: &str) {
        let counter = self.0.get(&(method.clone(), status)).and_then(|c| {
            c.get_or_try_init(|| {
                METRICS
                    .status_vec
                    .get_metric_with_label_values(&[path, method.as_ref(), &status.to_string()])
                    .map_err(|err| {
                        error!(
                            path,
                            %method,
                            ?status,
                            "Creating counter for metrics errored: {:?}", err
                        );
                    })
            })
            .ok()
        });
        if let Some(counter) = counter {
            counter.inc()
        }
    }
}

#[derive(Clone)]
pub struct MetricsMiddleware<S> {
    durations: HashMap<Method, OnceCell<Histogram>>,
    stats: MethodStatusCounters,
    path: String,
    service: S,
}

impl<S> MetricsMiddleware<S> {
    fn new(service: S, path: &str) -> Self {
        let path = path.trim_start_matches('/').replace('/', "_");
        let methods = [
            Method::PUT,
            Method::POST,
            Method::OPTIONS,
            Method::GET,
            Method::PATCH,
            Method::HEAD,
        ];
        let status_codes = (100..600).filter_map(|x| StatusCode::try_from(x).ok());
        let durations = methods
            .iter()
            .map(|method| (method.to_owned(), OnceCell::new()))
            .collect();
        let stats = status_codes
            .flat_map(|s| {
                methods
                    .iter()
                    .map(move |m| ((m.to_owned(), s), OnceCell::new()))
            })
            .collect();
        Self {
            durations,
            stats,
            path,
            service,
        }
    }

    fn start_timer(&self, method: Method) -> Option<HistogramTimer> {
        self.durations
            .get(&method)
            .and_then(|h| {
                h.get_or_try_init(|| {
                    METRICS
                        .duration_vec
                        .get_metric_with_label_values(&[&self.path, method.as_ref()])
                        .map_err(|err| {
                            error!(
                                path = %self.path,
                                %method,
                                "Creating timer for metrics errored: {:?}", err
                            )
                        })
                })
                .ok()
            })
            .map(|x| x.start_timer())
    }
}

use futures::future::BoxFuture;
use hyper::Request;
use hyper::Response;
use std::task::{Context, Poll};
use tower::{Layer, Service};

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for MetricsMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // best practice is to clone the inner service like this
        // see https://github.com/tower-rs/tower/issues/547 for details
        let clone = self.service.clone();
        let mut inner = std::mem::replace(&mut self.service, clone);
        let method = req.method().to_owned();
        let timer = self.start_timer(method.clone());

        let path = self.path.clone();
        let counters = self.stats.clone();
        Box::pin(async move {
            let res: Response<ResBody> = inner.call(req).await?;
            counters.inc_counter(method, res.status(), &path);
            drop(timer);
            Ok(res)
        })
    }
}

#[derive(Debug, Clone)]
pub struct MetricsMiddlewareLayer {
    path: String,
}

impl MetricsMiddlewareLayer {
    pub fn new(path: String) -> Self {
        Self { path }
    }
}

impl<S> Layer<S> for MetricsMiddlewareLayer {
    type Service = MetricsMiddleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        MetricsMiddleware::new(service, &self.path)
    }
}

use axum::body::Body;

pub trait MeteredRoute<H>
where
    H: Service<Request<Body>, Error = Infallible> + Send,
{
    type Output;

    fn metered_route(self, path: &str, svc: H) -> Self::Output;
}

impl<H> MeteredRoute<H> for Router
where
    H: Service<Request<Body>, Response = Response<axum::body::BoxBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    H::Future: Send + 'static,
{
    type Output = Router;

    fn metered_route(self, path: &str, svc: H) -> Self::Output {
        let handler = MetricsMiddlewareLayer::new(path.to_owned()).layer(svc);
        self.route(path, handler)
    }
}
