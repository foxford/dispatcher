use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};
use svc_agent::error::Error as AgentError;
use svc_agent::mqtt::{Agent, IntoPublishableMessage};
use svc_agent::AgentId;
use svc_authz::ClientMap as Authz;
use svc_nats_client::NatsClient;
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
    fn build_default_frontend_url(&self, tenant: &str, app: &str) -> Url;
    fn build_default_frontend_url_new(&self, tenant: &str, app: &str) -> Url;
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
    fn nats_client(&self) -> Option<&dyn NatsClient>;

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
    nats_client: Option<Arc<dyn NatsClient>>,
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
            nats_client: None,
        }
    }

    pub fn add_nats_client(self, nats_client: impl NatsClient + 'static) -> Self {
        Self {
            nats_client: Some(Arc::new(nats_client)),
            ..self
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

    fn build_default_frontend_url(&self, tenant: &str, app: &str) -> Url {
        build_default_url(self.config.default_frontend_base.clone(), tenant, app)
    }

    fn build_default_frontend_url_new(&self, tenant: &str, app: &str) -> Url {
        if let Some(config) = self.config.frontend.get(tenant) {
            build_tenant_url(config.base_url.clone(), app)
        } else {
            build_default_url_new(
                self.config.default_frontend_base_new.clone(),
                self.config.short_namespace.as_deref(),
                tenant,
                app,
            )
        }
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

    fn nats_client(&self) -> Option<&dyn NatsClient> {
        self.nats_client.as_deref()
    }
}

pub mod message_handler;

fn build_default_url(mut url: Url, tenant: &str, app: &str) -> Url {
    let host = url.host_str().map(|h| format!("{}.{}.{}", tenant, app, h));
    if let Err(e) = url.set_host(host.as_deref()) {
        tracing::error!("Default url set_host failed, reason = {:?}", e);
    }
    url
}

fn build_default_url_new(mut url: Url, ns: Option<&str>, tenant: &str, app: &str) -> Url {
    let path = match ns {
        Some(ns) => format!("/{}/{}-{}/{}/", ns, app, tenant, app),
        None => format!("/{}-{}/{}/", app, tenant, app),
    };
    url.set_path(&path);
    url
}

fn build_tenant_url(mut url: Url, app: &str) -> Url {
    url.path_segments_mut()
        .expect("cannot-be-a-base URL")
        .extend(&[app, ""]);
    url
}

#[test]
fn test_new_default_url() {
    let table = [
        (
            "\"https://dev.netology-group.services/\"",
            (Some("t01"), "foxford", "webinar"),
            "https://dev.netology-group.services/t01/webinar-foxford/webinar/",
        ),
        (
            "\"https://netology-group.services/\"",
            (None, "foxford", "classroom"),
            "https://netology-group.services/classroom-foxford/classroom/",
        ),
    ];
    for (url, (ns, tenant, app), sample) in table {
        let url: url::Url = serde_json::from_str(url).unwrap();
        let u = build_default_url_new(url, ns, tenant, app).to_string();
        assert_eq!(u, sample);
    }
    let url: url::Url = serde_json::from_str("\"https://dev.netology-group.services/\"").unwrap();
    let u = build_default_url_new(url, Some("t01"), "foxford", "webinar").to_string();
    assert_eq!(
        u,
        "https://dev.netology-group.services/t01/webinar-foxford/webinar/"
    );
}
