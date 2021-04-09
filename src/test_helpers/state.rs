use std::result::Result as StdResult;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use sqlx::pool::PoolConnection;
use sqlx::postgres::Postgres;
use svc_agent::error::Error as AgentError;
use svc_agent::{
    mqtt::{Address, IntoPublishableMessage, PublishableMessage},
    AccountId, AgentId,
};
use svc_authn::{token::jws_compact::extract::parse_jws_compact, Error as AuthnError};
use svc_authz::ClientMap as Authz;
use tide::http::url::Url;

use crate::app::{AppContext, Publisher};
use crate::clients::conference::{ConferenceClient, MockConferenceClient};
use crate::clients::event::{EventClient, MockEventClient};
use crate::clients::tq::{MockTqClient, TqClient};
use crate::config::{Config, StorageConfig};

use super::agent::TestAgent;
use super::authz::TestAuthz;
use super::db::TestDb;

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
    pub async fn new(agent: TestAgent, authz: TestAuthz) -> Self {
        let address = agent.address().to_owned();

        Self {
            db_pool: TestDb::new().await,
            config: crate::config::load().expect("Failed to load config"),
            agent,
            publisher: Arc::new(TestPublisher::new(address)),
            conference_client: Arc::new(MockConferenceClient::new()),
            event_client: Arc::new(MockEventClient::new()),
            tq_client: Arc::new(MockTqClient::new()),
            authz: authz.into(),
        }
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
        println!("TOKEN 1: {:?}", token);

        let token = token
            .map(|s| s.replace("Bearer ", ""))
            .unwrap_or_else(|| "".to_string());

        println!("TOKEN 2: {}", token);
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
}

////////////////////////////////////////////////////////////////////////////////

pub struct TestPublisher {
    address: Address,
    messages: Mutex<Vec<PublishableMessage>>,
}

impl TestPublisher {
    fn new(address: Address) -> Self {
        Self {
            address,
            messages: Mutex::new(vec![]),
        }
    }

    pub fn flush(&self) -> Vec<PublishableMessage> {
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

        let mut messages_lock = self
            .messages
            .lock()
            .expect("Failed to obtain messages lock");

        (*messages_lock).push(dump);
        Ok(())
    }
}
