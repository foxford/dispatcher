use serde_derive::Deserialize;
use svc_agent::{mqtt::AgentConfig, AccountId};
use svc_authn::jose::{Algorithm, ConfigMap as Authn};
use svc_authz::ConfigMap as Authz;
use svc_error::extension::sentry::Config as SentryConfig;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub id: AccountId,
    pub id_token: JwtConfig,
    pub agent_label: String,
    pub broker_id: AccountId,
    pub mqtt: AgentConfig,
    pub default_frontend_base: tide::http::url::Url,
    pub sentry: Option<SentryConfig>,
    pub http: HttpConfig,
    pub conference_client: MqttServiceConfig,
    pub event_client: MqttServiceConfig,
    pub tq_client: TqClientConfig,
    pub tenants: Vec<String>,
    pub authn: Authn,
    pub authz: Authz,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JwtConfig {
    #[serde(deserialize_with = "svc_authn::serde::algorithm")]
    pub algorithm: Algorithm,
    #[serde(deserialize_with = "svc_authn::serde::file")]
    pub key: Vec<u8>,
}

pub fn load() -> Result<Config, config::ConfigError> {
    let mut parser = config::Config::default();
    parser.merge(config::File::with_name("App"))?;
    parser.merge(config::Environment::with_prefix("APP").separator("__"))?;
    parser.try_into::<Config>()
}
#[derive(Clone, Debug, Deserialize)]
pub struct HttpConfig {
    pub listener_address: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TqClientConfig {
    pub base_url: surf::Url,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MqttServiceConfig {
    pub account_id: AccountId,
    pub timeout: u64,
    pub api_version: String,
}
