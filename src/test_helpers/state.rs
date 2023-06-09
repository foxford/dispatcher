use parking_lot::Mutex;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use sqlx::pool::PoolConnection;
use sqlx::postgres::Postgres;
use svc_agent::error::Error as AgentError;
use svc_agent::{
    mqtt::{Address, IntoPublishableMessage},
    AgentId,
};
use svc_authz::ClientMap as Authz;
use svc_nats_client::{
    AckPolicy, DeliverPolicy, Event, Message, MessageStream, Messages, NatsClient, PublishError,
    Subject, SubscribeError, TermMessageError,
};
use url::Url;
use vec1::{vec1, Vec1};

use crate::app::turn_host::TurnHostSelector;
use crate::app::{AppContext, Publisher};
use crate::clients::conference::{ConferenceClient, MockConferenceClient};
use crate::clients::event::{EventClient, MockEventClient};
use crate::clients::tq::{MockTqClient, TqClient};
use crate::config::{Config, StorageConfig, TqAudienceSettings};

use super::agent::TestAgent;
use super::authz::TestAuthz;
use super::db::TestDb;
use super::outgoing_envelope::OutgoingEnvelope;
use super::SVC_AUDIENCE;

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
    turn_host_selector: TurnHostSelector,
    nats_client: Option<Arc<dyn NatsClient>>,
}

fn build_config() -> Config {
    let id = format!("dispatcher.{}", SVC_AUDIENCE);
    let broker_id = format!("mqtt-gateway.{}", SVC_AUDIENCE);

    let config = json!({
        "id": id,
        "agent_label": "alpha",
        "broker_id": broker_id,
        "default_frontend_base": "http://testing01.example.org",
        "default_frontend_base_new": "http://dev.example.org",
        "frontend": {
            "example": {
                "base_url": "https://apps.example.com",
            },
        },
        "tenants": ["testing01.example.org"],
        "id_token": {
            "algorithm": "ES256",
            "key": "data/keys/svc.private_key.p8.der.sample",
        },
        "authz": {},
        "authn": {},
        "mqtt": {
            "uri": "mqtt://0.0.0.0:1883",
            "clean_session": false,
        },
        "http": {
            "listener_address": "0.0.0.0:3000",
            "metrics_listener_address": "0.0.0.0:3001"
        },
        "storage": {
            "base_url": "http://localhost:4000/some-postfix"
        },
        "conference_client": {
            "account_id": "conference.dev.svc.example.org",
            "timeout": 5,
            "api_version": "v1"
        },
        "event_client": {
            "account_id": "event.dev.svc.example.org",
            "timeout": 5,
            "api_version": "v1"
        },
        "tq_client": {
            "base_url": "http://localhost:3000/",
            "account_id": "tq.dev.svc.example.org",
            "timeout": 5,
            "api_version": "v1"
        },
        "retry_delay": "1 seconds",
        "turn_hosts": [ "turn.example.org" ]
    });

    serde_json::from_value::<Config>(config).expect("Failed to parse test config")
}

struct TestNatsClient;

#[async_trait]
impl NatsClient for TestNatsClient {
    async fn publish(&self, _event: &Event) -> Result<(), PublishError> {
        Ok(())
    }

    async fn subscribe_durable(&self) -> Result<MessageStream, SubscribeError> {
        unimplemented!()
    }

    async fn subscribe_ephemeral(
        &self,
        _subject: Subject,
        _deliver_policy: DeliverPolicy,
        _ack_policy: AckPolicy,
    ) -> Result<Messages, SubscribeError> {
        unimplemented!()
    }

    async fn terminate(&self, _message: &Message) -> Result<(), TermMessageError> {
        unimplemented!()
    }
}

impl TestState {
    pub async fn new(authz: TestAuthz) -> Self {
        let config = build_config();

        let agent = TestAgent::new(&config.agent_label, config.id.label(), config.id.audience());

        let address = agent.address().to_owned();

        Self {
            db_pool: TestDb::new().await,
            turn_host_selector: TurnHostSelector::new(&config.turn_hosts),
            config,
            agent,
            publisher: Arc::new(TestPublisher::new(address)),
            conference_client: Arc::new(MockConferenceClient::new()),
            event_client: Arc::new(MockEventClient::new()),
            tq_client: Arc::new(MockTqClient::new()),
            authz: authz.into(),
            nats_client: Some(Arc::new(TestNatsClient {}) as Arc<dyn NatsClient>),
        }
    }

    pub fn new_with_pool(db_pool: TestDb, authz: TestAuthz) -> Self {
        let config = build_config();

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
            turn_host_selector: TurnHostSelector::new(&vec1!["turn.example.org".into()]),
            nats_client: Some(Arc::new(TestNatsClient {}) as Arc<dyn NatsClient>),
        }
    }

    pub fn set_audience_preroll_offset(&mut self, audience: &str, value: i64) {
        self.config
            .tq_client
            .audience_settings
            .entry(audience.to_owned())
            .and_modify(|v| v.preroll_offset = Some(value))
            .or_insert_with(|| TqAudienceSettings {
                preroll_offset: Some(value),
                ..Default::default()
            });
    }

    pub fn set_turn_hosts(&mut self, hosts: &[&str]) {
        let hosts = hosts.iter().map(|c| (*c).into()).collect::<Vec<_>>();
        let hosts = Vec1::try_from_vec(hosts).unwrap();
        self.turn_host_selector = TurnHostSelector::new(&hosts);
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

    fn build_default_frontend_url(&self, _tenant: &str, _app: &str) -> Result<Url> {
        todo!()
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

    fn turn_host_selector(&self) -> &TurnHostSelector {
        &self.turn_host_selector
    }

    fn nats_client(&self) -> Option<&dyn NatsClient> {
        self.nats_client.as_deref()
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
        let mut messages_lock = self.messages.lock();

        (*messages_lock).drain(0..).collect::<Vec<_>>()
    }
}

impl Publisher for TestPublisher {
    fn publish(&self, message: Box<dyn IntoPublishableMessage>) -> Result<(), AgentError> {
        let dump = message.into_dump(&self.address)?;

        let mut parsed_message = serde_json::from_str::<OutgoingEnvelope>(dump.payload())
            .expect("Failed to parse dumped message");

        parsed_message.set_topic(dump.topic());
        let mut messages_lock = self.messages.lock();

        (*messages_lock).push(parsed_message);
        Ok(())
    }
}
