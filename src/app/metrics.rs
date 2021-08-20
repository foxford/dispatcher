use std::{collections::HashMap, convert::TryFrom};

use once_cell::sync::{Lazy, OnceCell};
use prometheus::{
    register_histogram_vec, register_int_counter_vec, Histogram, HistogramTimer, HistogramVec,
    IntCounter, IntCounterVec,
};
use prometheus_static_metric::make_static_metric;
use tide::{http::Method, Middleware, Next, Request, Route, StatusCode};

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

pub trait AddMetrics<'a, S> {
    fn with_metrics(&mut self) -> &mut Self;
}

impl<'a, S: Clone + Send + Sync + 'static> AddMetrics<'a, S> for Route<'a, S> {
    fn with_metrics(&mut self) -> &mut Self {
        self.with(MetricsMiddleware::new(self.path()))
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
        }
    }
}

struct MetricsMiddleware {
    durations: HashMap<Method, OnceCell<Histogram>>,
    stats: HashMap<(Method, StatusCode), OnceCell<IntCounter>>,
    path: String,
}

impl MetricsMiddleware {
    fn new(path: &str) -> Self {
        let path = path.trim_start_matches('/').replace('/', "_");
        let methods = [
            Method::Put,
            Method::Post,
            Method::Options,
            Method::Get,
            Method::Patch,
            Method::Head,
        ];
        let status_codes = (100..600).filter_map(|x| StatusCode::try_from(x).ok());
        let durations = methods
            .iter()
            .map(|method| (*method, OnceCell::new()))
            .collect();
        let stats = status_codes
            .flat_map(|s| methods.iter().map(move |m| ((*m, s), OnceCell::new())))
            .collect();
        Self {
            durations,
            stats,
            path,
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
                            error!(crate::LOG, "Crating timer for metrics errored: {:?}", err;
                            "path" => &self.path,
                            "method" => method.as_ref())
                        })
                })
                .ok()
            })
            .map(|x| x.start_timer())
    }

    fn inc_counter(&self, method: Method, status: StatusCode) {
        let counter = self.stats.get(&(method, status)).and_then(|c| {
            c.get_or_try_init(|| {
                METRICS
                    .status_vec
                    .get_metric_with_label_values(&[
                        &self.path,
                        method.as_ref(),
                        &status.to_string(),
                    ])
                    .map_err(|err| {
                        error!(crate::LOG, "Crating counter for metrics errored: {:?}", err;
                            "path" => &self.path,
                            "method" => method.as_ref(),
                            "status" => &status.to_string())
                    })
            })
            .ok()
        });
        if let Some(counter) = counter {
            counter.inc()
        }
    }
}

#[async_trait::async_trait]
impl<State: Clone + Send + Sync + 'static> Middleware<State> for MetricsMiddleware {
    async fn handle(&self, req: Request<State>, next: Next<'_, State>) -> tide::Result {
        let method = req.method();
        let _timer = self.start_timer(method);
        let response = next.run(req).await;
        self.inc_counter(method, response.status());
        Ok(response)
    }
}
