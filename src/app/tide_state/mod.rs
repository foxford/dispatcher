use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};
use svc_agent::error::Error as AgentError;
use svc_agent::mqtt::{Agent, IntoPublishableMessage};
use svc_agent::AgentId;
use svc_authz::ClientMap as Authz;
use url::Url;

use crate::clients::conference::ConferenceClient;
use crate::clients::event::EventClient;
use crate::clients::tq::TqClient;
use crate::config::Config;
use crate::config::StorageConfig;

use super::turn_host::TurnHostSelector;

#[async_trait]
pub trait AppContext: Sync + Send {
    async fn get_conn(&self) -> Result<PoolConnection<Postgres>>;
    fn default_frontend_base(&self) -> Url;
    fn agent_id(&self) -> &AgentId;
    fn publisher(&self) -> &dyn Publisher;
    fn conference_client(&self) -> &dyn ConferenceClient;
    fn event_client(&self) -> &dyn EventClient;
    fn tq_client(&self) -> &dyn TqClient;
    fn authz(&self) -> &Authz;
    fn storage_config(&self) -> &StorageConfig;
    fn config(&self) -> &Config;
    fn agent(&self) -> Option<&Agent>;
    fn turn_host_selector(&self) -> &TurnHostSelector;

    fn get_preroll_offset(&self, audience: &str) -> i64 {
        self.config()
            .tq_client
            .audience_settings
            .get(audience)
            .and_then(|c| c.preroll_offset)
            .unwrap_or(0)
    }
}

pub trait Publisher {
    fn publish(&self, message: Box<dyn IntoPublishableMessage>) -> Result<(), AgentError>;
}

impl Publisher for Agent {
    fn publish(&self, message: Box<dyn IntoPublishableMessage>) -> Result<(), AgentError> {
        self.clone().publish_publishable(message)
    }
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
    turn_host_selector: TurnHostSelector,
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
        let turn_host_selector = TurnHostSelector::new(&config.turn_hosts);

        Self {
            db_pool,
            config,
            agent,
            conference_client,
            event_client,
            tq_client,
            authz,
            turn_host_selector,
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

    fn agent_id(&self) -> &AgentId {
        self.agent.id()
    }

    fn publisher(&self) -> &dyn Publisher {
        &self.agent
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

    fn storage_config(&self) -> &StorageConfig {
        &self.config.storage
    }

    fn config(&self) -> &Config {
        &self.config
    }

    fn agent(&self) -> Option<&Agent> {
        Some(&self.agent)
    }

    fn turn_host_selector(&self) -> &TurnHostSelector {
        &self.turn_host_selector
    }
}

pub mod message_handler;
