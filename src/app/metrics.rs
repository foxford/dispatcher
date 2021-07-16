use std::{collections::HashMap, sync::RwLock};

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, Histogram, HistogramVec, IntCounter,
    IntCounterVec,
};
use prometheus_static_metric::make_static_metric;
use tide::{http::Method, Endpoint, Middleware, Next, Request, Route, StatusCode};

use super::error::Error;

make_static_metric! {
    struct MqttStats: IntCounter {
        "method" => {
            room_close,
            room_upload,
            room_adjust,
            task_complete,
            room_dumps_events,
        },
        "status" => {
            success,
            failure,
        },
    }
}

static MQTT_METRICS: Lazy<MqttMetrics> = Lazy::new(MqttMetrics::new);

pub struct MqttMetrics {
    stats: MqttStats,
    connection_error: IntCounter,
    disconnect: IntCounter,
    reconnection: IntCounter,
}

impl MqttMetrics {
    pub fn new() -> Self {
        let mqtt_stats =
            register_int_counter_vec!("mqtt_stats", "Mqtt stats", &["method", "status"])
                .expect("Can't create stats metrics");
        let mqtt_errors =
            register_int_counter_vec!("mqtt_messages", "Mqtt message types", &["status"])
                .expect("Bad mqtt messages metric");
        Self {
            stats: MqttStats::from(&mqtt_stats),
            connection_error: mqtt_errors.with_label_values(&["connection_error"]),
            disconnect: mqtt_errors.with_label_values(&["disconnect"]),
            reconnection: mqtt_errors.with_label_values(&["reconnect"]),
        }
    }

    pub fn observe_disconnect() {
        MQTT_METRICS.disconnect.inc()
    }

    pub fn observe_reconnect() {
        MQTT_METRICS.reconnection.inc()
    }

    pub fn observe_connection_error() {
        MQTT_METRICS.connection_error.inc()
    }

    pub fn observe_event_result(result: &Result<(), Error>, label: Option<&str>) {
        match label {
            Some("room.close") => {
                if result.is_err() {
                    MQTT_METRICS.stats.room_close.failure.inc();
                } else {
                    MQTT_METRICS.stats.room_close.success.inc();
                }
            }
            Some("room.upload") => {
                if result.is_err() {
                    MQTT_METRICS.stats.room_upload.failure.inc();
                } else {
                    MQTT_METRICS.stats.room_upload.success.inc();
                }
            }
            Some("room.adjust") => {
                if result.is_err() {
                    MQTT_METRICS.stats.room_adjust.failure.inc();
                } else {
                    MQTT_METRICS.stats.room_adjust.success.inc();
                }
            }
            Some("task.complete") => {
                if result.is_err() {
                    MQTT_METRICS.stats.task_complete.failure.inc();
                } else {
                    MQTT_METRICS.stats.task_complete.success.inc();
                }
            }
            Some("room.dump_events") => {
                if result.is_err() {
                    MQTT_METRICS.stats.room_dumps_events.failure.inc();
                } else {
                    MQTT_METRICS.stats.room_dumps_events.success.inc();
                }
            }
            _ => {}
        }
    }
}

pub trait AddMetrics<'a, S: Clone + Send + Sync + 'static> {
    fn metrics(self) -> MetricsRouter<'a, S>;
}

impl<'a, S: Clone + Send + Sync + 'static> AddMetrics<'a, S> for Route<'a, S> {
    fn metrics(self) -> MetricsRouter<'a, S> {
        MetricsRouter {
            route: self,
            methods: vec![],
        }
    }
}

pub struct MetricsRouter<'a, S: Clone + Send + Sync + 'static> {
    route: Route<'a, S>,
    methods: Vec<Method>,
}

impl<'a, S: Clone + Send + Sync + 'static> Drop for MetricsRouter<'a, S> {
    fn drop(&mut self) {
        self.route
            .with(MetricsMiddleware::new(self.route.path(), &self.methods));
    }
}

impl<'a, S: Clone + Send + Sync + 'static> MetricsRouter<'a, S> {
    pub fn get(&mut self, ep: impl Endpoint<S>) -> &mut Self {
        self.method(Method::Get, ep)
    }

    pub fn post(&mut self, ep: impl Endpoint<S>) -> &mut Self {
        self.method(Method::Post, ep)
    }

    pub fn put(&mut self, ep: impl Endpoint<S>) -> &mut Self {
        self.method(Method::Put, ep)
    }

    pub fn options(&mut self, ep: impl Endpoint<S>) -> &mut Self {
        self.method(Method::Options, ep)
    }

    pub fn with<M>(&mut self, middleware: M) -> &mut Self
    where
        M: Middleware<S>,
    {
        self.route.with(middleware);
        self
    }

    fn method(&mut self, method: Method, ep: impl Endpoint<S>) -> &mut Self {
        self.methods.push(method);
        self.route.method(method, ep);
        self
    }
}

static METRICS: Lazy<HttpMetrics> = Lazy::new(HttpMetrics::new);

#[derive(Debug)]
struct HttpMetrics {
    duration_vec: HistogramVec,
    status_vec: IntCounterVec,
}

impl HttpMetrics {
    pub fn new() -> Self {
        HttpMetrics {
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
        }
    }
}

struct MetricsMiddleware {
    durations: HashMap<Method, Histogram>,
    stats: RwLock<HashMap<StatusCode, Result<IntCounter, prometheus::Error>>>,
    path: String,
}

impl MetricsMiddleware {
    fn new(path: &str, methods: &[Method]) -> Self {
        let path = path.trim_start_matches('/').replace('/', "_");
        let durations = methods
            .iter()
            .map(|method| {
                (
                    *method,
                    METRICS
                        .duration_vec
                        .with_label_values(&[&path, method.as_ref()]),
                )
            })
            .collect();
        let stats = RwLock::new(HashMap::<_, Result<IntCounter, _>>::new());
        Self {
            durations,
            stats,
            path,
        }
    }

    fn increment_stats(&self, status: StatusCode, method: Method) {
        {
            let stats = self.stats.read();
            match stats {
                Ok(stats) => {
                    if let Some(stats) = stats.get(&status) {
                        match stats {
                            Ok(stats) => stats.inc(),
                            Err(err) => error!(crate::LOG, "Got bad metrics: {:?}", err),
                        }
                        return;
                    }
                }
                Err(err) => {
                    error!(crate::LOG, "Metrics log poisoned: {:?}", err)
                }
            }
        }
        {
            let mut stats = self.stats.write();
            match &mut stats {
                Ok(stats) => {
                    let _ = stats
                        .entry(status)
                        .or_insert_with(|| {
                            METRICS.status_vec.get_metric_with_label_values(&[
                                &self.path,
                                method.as_ref(),
                                &status.to_string(),
                            ])
                        })
                        .as_ref()
                        .map(|x| x.inc());
                }
                Err(error) => {
                    error!(crate::LOG, "Metrics log poisoned: {:?}", error)
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl<State: Clone + Send + Sync + 'static> Middleware<State> for MetricsMiddleware {
    async fn handle(&self, req: Request<State>, next: Next<'_, State>) -> tide::Result {
        let method = req.method();
        let _timer = self.durations.get(&method).map(|h| h.start_timer());
        let response = next.run(req).await;
        self.increment_stats(response.status(), method);
        Ok(response)
    }
}
