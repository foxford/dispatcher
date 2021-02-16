use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use jsonwebtoken::{TokenData, Validation};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};
use svc_agent::{error::Error as AgentError, mqtt::Agent};
use svc_authn::token::jws_compact::extract::decode_jws_compact;
use svc_authn::{jose::Claims, Error};
use tide::http::url::Url;

use crate::config::Config;

use conference_client::ConferenceClient;
use event_client::EventClient;
use tq_client::TqClient;

#[async_trait]
pub trait AppContext: Sync + Send {
    async fn get_conn(&self) -> Result<PoolConnection<Postgres>>;
    fn default_frontend_base(&self) -> Url;
    fn validate_token(&self, token: Option<&str>) -> Result<(), Error>;
    fn agent(&self) -> Option<Agent>;
    fn conference_client(&self) -> &dyn ConferenceClient;
    fn event_client(&self) -> &dyn EventClient;
    fn tq_client(&self) -> &dyn TqClient;
}

#[derive(Clone)]
pub struct TideState {
    db_pool: PgPool,
    config: Config,
    agent: Agent,
    conference_client: Arc<dyn ConferenceClient>,
    event_client: Arc<dyn EventClient>,
    tq_client: Arc<dyn TqClient>,
}

impl TideState {
    pub fn new(
        db_pool: PgPool,
        config: Config,
        event_client: Arc<dyn EventClient>,
        conference_client: Arc<dyn ConferenceClient>,
        tq_client: Arc<dyn TqClient>,
        agent: Agent,
    ) -> Self {
        Self {
            db_pool,
            config,
            conference_client,
            event_client,
            tq_client,
            agent,
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

    fn validate_token(&self, token: Option<&str>) -> Result<(), Error> {
        let verifier = Validation {
            iss: Some(self.config.api_auth.audience.clone()),
            algorithms: vec![self.config.api_auth.algorithm],
            ..Validation::default()
        };

        let claims = match token {
            Some(token) if token.starts_with("Bearer ") => {
                let token = token.replace("Bearer ", "");
                decode_jws_compact(
                    &token,
                    &verifier,
                    &self.config.api_auth.key,
                    self.config.api_auth.algorithm,
                )
            }
            _ => decode_jws_compact(
                "",
                &verifier,
                &self.config.api_auth.key,
                self.config.api_auth.algorithm,
            ),
        };

        claims.map(|_: TokenData<Claims<String>>| ())
    }

    fn agent(&self) -> Option<Agent> {
        Some(self.agent.clone())
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
