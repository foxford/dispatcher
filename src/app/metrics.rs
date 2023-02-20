use chrono::Duration;
use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_int_counter_vec, Histogram, HistogramTimer, IntCounter,
};
use prometheus_static_metric::make_static_metric;

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
            stats: MqttStats::from(&mqtt_stats),
            connection_error: mqtt_errors.with_label_values(&["connection_error"]),
            disconnect: mqtt_errors.with_label_values(&["disconnect"]),
            reconnection: mqtt_errors.with_label_values(&["reconnect"]),
            authz_time: register_histogram!("auth_time", "Authorization time")
                .expect("Bad authz hist"),
        }
    }
}
