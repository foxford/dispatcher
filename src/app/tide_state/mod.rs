use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};
use svc_agent::{error::Error as AgentError, mqtt::Agent};
use svc_authn::token::jws_compact::extract::decode_jws_compact_with_config;
use svc_authn::{AccountId, Error};
use svc_authz::ClientMap as Authz;
use tide::http::url::Url;

use crate::config::Config;

use conference_client::ConferenceClient;
use event_client::EventClient;
use tq_client::TqClient;

#[async_trait]
pub trait AppContext: Sync + Send {
    async fn get_conn(&self) -> Result<PoolConnection<Postgres>>;
    fn default_frontend_base(&self) -> Url;
    fn validate_token(&self, token: Option<&str>) -> Result<AccountId, Error>;
    fn agent(&self) -> Agent;
    fn conference_client(&self) -> &dyn ConferenceClient;
    fn event_client(&self) -> &dyn EventClient;
    fn tq_client(&self) -> &dyn TqClient;
    fn authz(&self) -> &Authz;
}

#[derive(Clone)]
pub struct TideState {
    db_pool: PgPool,
    config: Config,
    agent: Agent,
    conference_client: Arc<dyn ConferenceClient>,
    event_client: Arc<dyn EventClient>,
    tq_client: Arc<dyn TqClient>,
    authz: Authz,
}

impl TideState {
    pub fn new(
        db_pool: PgPool,
        config: Config,
        event_client: Arc<dyn EventClient>,
        conference_client: Arc<dyn ConferenceClient>,
        tq_client: Arc<dyn TqClient>,
        agent: Agent,
        authz: Authz,
    ) -> Self {
        Self {
            db_pool,
            config,
            conference_client,
            event_client,
            tq_client,
            agent,
            authz,
        }
    }
}

#[async_trait]
impl AppContext for TideState {
    async fn get_conn(&self) -> Result<PoolConnection<Postgres>> {
        self.db_pool
            .acquire()
            .await
            .context("Failed to acquire DB connection")
    }

    fn default_frontend_base(&self) -> Url {
        self.config.default_frontend_base.clone()
    }

    fn validate_token(&self, token: Option<&str>) -> Result<AccountId, Error> {
        let token = token
            .map(|s| s.replace("Bearer ", ""))
            .unwrap_or_else(|| "".to_string());

        let claims = decode_jws_compact_with_config::<String>(&token, &self.config.authn)?.claims;
        let account = AccountId::new(claims.subject(), claims.audience());

        Ok(account)
    }

    fn agent(&self) -> Agent {
        self.agent.clone()
    }

    fn conference_client(&self) -> &dyn ConferenceClient {
        self.conference_client.as_ref()
    }

    fn event_client(&self) -> &dyn EventClient {
        self.event_client.as_ref()
    }

    fn tq_client(&self) -> &dyn TqClient {
        self.tq_client.as_ref()
    }

    fn authz(&self) -> &Authz {
        &self.authz
    }
}

#[derive(Debug)]
pub enum ClientError {
    AgentError(AgentError),
    PayloadError(String),
    TimeoutError,
    HttpError(String),
}

impl From<AgentError> for ClientError {
    fn from(e: AgentError) -> Self {
        Self::AgentError(e)
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::AgentError(ae) => write!(f, "Agent inner error: {}", ae),
            ClientError::PayloadError(s) => write!(f, "Payload error: {}", s),
            ClientError::TimeoutError => write!(f, "Timeout"),
            ClientError::HttpError(s) => write!(f, "Http error: {}", s),
        }
    }
}

impl StdError for ClientError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
}

const CORRELATION_DATA_LENGTH: usize = 16;

fn generate_correlation_data() -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(CORRELATION_DATA_LENGTH)
        .collect()
}

pub mod conference_client;
pub mod event_client;
pub mod message_handler;
pub mod tq_client;
