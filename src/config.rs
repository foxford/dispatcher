use std::collections::HashMap;
use std::{net::SocketAddr, time::Duration};

use serde_derive::Deserialize;
use svc_agent::{mqtt::AgentConfig, AccountId};
use svc_authn::jose::{Algorithm, ConfigMap as Authn};
use svc_authz::ConfigMap as Authz;
use svc_error::extension::sentry::Config as SentryConfig;

use crate::app::turn_host::TurnHost;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub id: AccountId,
    pub id_token: JwtConfig,
    pub agent_label: String,
    pub broker_id: AccountId,
    pub mqtt: AgentConfig,
    pub default_frontend_base: url::Url,
    pub default_frontend_base_new: url::Url,
    pub sentry: Option<SentryConfig>,
    pub http: HttpConfig,
    pub conference_client: MqttServiceConfig,
    pub event_client: MqttServiceConfig,
    pub tq_client: TqClientConfig,
    pub tenants: Vec<String>,
    pub authn: Authn,
    pub authz: Authz,
    pub storage: StorageConfig,
    #[serde(with = "humantime_serde")]
    pub retry_delay: Duration,
    pub turn_hosts: vec1::Vec1<TurnHost>,
    pub short_namespace: Option<String>,
    pub frontend: HashMap<String, FrontendConfig>,
    pub nats: Option<svc_nats_client::Config>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JwtConfig {
    #[serde(deserialize_with = "svc_authn::serde::algorithm")]
    pub algorithm: Algorithm,
    #[serde(deserialize_with = "svc_authn::serde::file")]
    pub key: Vec<u8>,
}

pub fn load() -> Result<Config, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("App"))
        .add_source(config::Environment::with_prefix("APP"))
        .build()
        .and_then(|c| c.try_deserialize::<Config>())
}

#[derive(Clone, Debug, Deserialize)]
pub struct HttpConfig {
    pub listener_address: String,
    pub metrics_listener_address: SocketAddr,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TqClientConfig {
    pub base_url: String,
    pub timeout: u64,
    pub account_id: AccountId,
    pub api_version: String,
    #[serde(default)]
    pub audience_settings: HashMap<String, TqAudienceSettings>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct TqAudienceSettings {
    pub to: Option<String>,
    pub preroll: Option<String>,
    pub postroll: Option<String>,
    pub watermark: Option<String>,
    pub preroll_offset: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MqttServiceConfig {
    pub account_id: AccountId,
    pub timeout: u64,
    pub api_version: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StorageConfig {
    pub base_url: url::Url,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FrontendConfig {
    pub base_url: url::Url,
}
