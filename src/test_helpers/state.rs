use std::result::Result as StdResult;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use sqlx::pool::PoolConnection;
use sqlx::postgres::Postgres;
use svc_agent::error::Error as AgentError;
use svc_agent::{
    mqtt::{Address, IntoPublishableMessage},
    AccountId, AgentId,
};
use svc_authn::{token::jws_compact::extract::parse_jws_compact, Error as AuthnError};
use svc_authz::ClientMap as Authz;
use url::Url;

use crate::app::{AppContext, Publisher};
use crate::clients::conference::{ConferenceClient, MockConferenceClient};
use crate::clients::event::{EventClient, MockEventClient};
use crate::clients::tq::{MockTqClient, TqClient};
use crate::config::{Config, StorageConfig};

use super::db::TestDb;
use super::outgoing_envelope::OutgoingEnvelope;
use super::{agent::TestAgent, prelude::TestAuthz};

////////////////////////////////////////////////////////////////////////////////

#[derive(Clone)]
pub struct TestState {
    db_pool: TestDb,
    config: Config,
    agent: TestAgent,
    publisher: Arc<TestPublisher>,
    conference_client: Arc<MockConferenceClient>,
    event_client: Arc<MockEventClient>,
    tq_client: Arc<MockTqClient>,
    authz: Authz,
}

impl TestState {
    pub async fn new(authz: TestAuthz) -> Self {
        let config = crate::config::load("App.sample.toml").expect("Failed to load config");

        let agent = TestAgent::new(&config.agent_label, config.id.label(), config.id.audience());

        let address = agent.address().to_owned();

        Self {
            db_pool: TestDb::new().await,
            config,
            agent,
            publisher: Arc::new(TestPublisher::new(address)),
            conference_client: Arc::new(MockConferenceClient::new()),
            event_client: Arc::new(MockEventClient::new()),
            tq_client: Arc::new(MockTqClient::new()),
            authz: authz.into(),
        }
    }

    pub fn new_with_pool(db_pool: TestDb, authz: TestAuthz) -> Self {
        let config = crate::config::load("App.sample.toml").expect("Failed to load config");

        let agent = TestAgent::new(&config.agent_label, config.id.label(), config.id.audience());

        let address = agent.address().to_owned();

        Self {
            db_pool,
            config,
            agent,
            publisher: Arc::new(TestPublisher::new(address)),
            conference_client: Arc::new(MockConferenceClient::new()),
            event_client: Arc::new(MockEventClient::new()),
            tq_client: Arc::new(MockTqClient::new()),
            authz: authz.into(),
        }
    }
}

impl TestState {
    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn test_publisher(&self) -> &TestPublisher {
        self.publisher.as_ref()
    }

    pub fn conference_client_mock(&mut self) -> &mut MockConferenceClient {
        Arc::get_mut(&mut self.conference_client).expect("Failed to get conference client mock")
    }

    pub fn event_client_mock(&mut self) -> &mut MockEventClient {
        Arc::get_mut(&mut self.event_client).expect("Failed to get event client mock")
    }

    pub fn tq_client_mock(&mut self) -> &mut MockTqClient {
        Arc::get_mut(&mut self.tq_client).expect("Failed to get tq client mock")
    }
}

#[async_trait]
impl AppContext for TestState {
    async fn get_conn(&self) -> Result<PoolConnection<Postgres>> {
        let conn = self.db_pool.get_conn().await;
        Ok(conn)
    }

    fn default_frontend_base(&self) -> Url {
        self.config.default_frontend_base.clone()
    }

    fn validate_token(&self, token: Option<&str>) -> StdResult<AccountId, AuthnError> {
        let token = token
            .map(|s| s.replace("Bearer ", ""))
            .unwrap_or_else(|| "".to_string());

        // Parse but skip key verification.
        let claims = parse_jws_compact::<String>(&token)?.claims;
        Ok(AccountId::new(claims.subject(), claims.audience()))
    }

    fn agent_id(&self) -> &AgentId {
        self.agent.agent_id()
    }

    fn publisher(&self) -> &dyn Publisher {
        self.publisher.as_ref()
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

    fn agent(&self) -> Option<&svc_agent::mqtt::Agent> {
        None
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct TestPublisher {
    address: Address,
    messages: Mutex<Vec<OutgoingEnvelope>>,
}

impl TestPublisher {
    fn new(address: Address) -> Self {
        Self {
            address,
            messages: Mutex::new(vec![]),
        }
    }

    pub fn flush(&self) -> Vec<OutgoingEnvelope> {
        let mut messages_lock = self
            .messages
            .lock()
            .expect("Failed to obtain messages lock");

        (*messages_lock).drain(0..).collect::<Vec<_>>()
    }
}

impl Publisher for TestPublisher {
    fn publish(&self, message: Box<dyn IntoPublishableMessage>) -> Result<(), AgentError> {
        let dump = message.into_dump(&self.address)?;

        let mut parsed_message = serde_json::from_str::<OutgoingEnvelope>(dump.payload())
            .expect("Failed to parse dumped message");

        parsed_message.set_topic(dump.topic());

        let mut messages_lock = self
            .messages
            .lock()
            .expect("Failed to obtain messages lock");

        (*messages_lock).push(parsed_message);
        Ok(())
    }
}
