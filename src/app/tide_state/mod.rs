use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use jsonwebtoken::{TokenData, Validation};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde_json::Value as JsonValue;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};
use svc_agent::{
    error::Error as AgentError,
    mqtt::{Agent, AgentBuilder, AgentNotification, ConnectionMode, IncomingResponse, QoS},
    request::Dispatcher,
    AgentId, Authenticable, Subscription,
};
use svc_authn::token::jws_compact;
use svc_authn::token::jws_compact::extract::decode_jws_compact;
use svc_authn::{jose::Claims, Error};
use tide::http::url::Url;

use crate::config::Config;

const API_VERSION: &str = "v2";

use conference_client::{ConferenceClient, MqttConferenceClient};
use event_client::{EventClient, MqttEventClient};

#[async_trait]
pub trait AppContext: Sync + Send {
    async fn get_conn(&self) -> Result<PoolConnection<Postgres>>;
    fn default_frontend_base(&self) -> Url;
    fn validate_token(&self, token: Option<&str>) -> Result<(), Error>;
    fn agent(&self) -> Option<Agent>;
    fn conference_client(&self) -> &dyn ConferenceClient;
    fn event_client(&self) -> &dyn EventClient;
}

#[derive(Clone)]
pub struct TideState {
    db_pool: PgPool,
    config: Config,
    agent: Agent,
    dispatcher: Arc<Dispatcher>,
    conference_client: Arc<dyn ConferenceClient>,
    event_client: Arc<dyn EventClient>,
}

impl TideState {
    pub fn new(db_pool: PgPool, config: Config) -> Result<Self> {
        let agent_id = AgentId::new(&config.agent_label, config.id.clone());
        info!(crate::LOG, "Agent id: {:?}", &agent_id);

        let token = jws_compact::TokenBuilder::new()
            .issuer(&agent_id.as_account_id().audience().to_string())
            .subject(&agent_id)
            .key(config.id_token.algorithm, config.id_token.key.as_slice())
            .build()
            .context("Error creating an id token")?;

        let mut agent_config = config.mqtt.clone();
        agent_config.set_password(&token);

        let (mut agent, rx) = AgentBuilder::new(agent_id.clone(), API_VERSION)
            .connection_mode(ConnectionMode::Service)
            .start(&agent_config)
            .context("Failed to create an agent")?;

        let (mq_tx, mut mq_rx) = futures_channel::mpsc::unbounded::<AgentNotification>();

        std::thread::Builder::new()
            .name("dispatcher-notifications-loop".to_owned())
            .spawn(move || {
                for message in rx {
                    if mq_tx.unbounded_send(message).is_err() {
                        error!(crate::LOG, "Error sending message to the internal channel");
                    }
                }
            })
            .expect("Failed to start dispatcher notifications loop");

        let dispatcher = Arc::new(Dispatcher::new(&agent));
        let dispatcher_ = dispatcher.clone();

        async_std::task::spawn(async move {
            while let Some(message) = mq_rx.next().await {
                let dispatcher__ = dispatcher_.clone();
                async_std::task::spawn(async move {
                    match message {
                        AgentNotification::Message(
                            Ok(svc_agent::mqtt::IncomingMessage::Response(data)),
                            _,
                        ) => {
                            let message = IncomingResponse::convert::<JsonValue>(data)
                                .expect("Couldnt convert message");

                            if let Err(e) = dispatcher__.response(message).await {
                                error!(crate::LOG, "Failed to commit response, reason = {:?}", e);
                            }
                        }
                        AgentNotification::Message(_, _) => (),
                        AgentNotification::Disconnection => {
                            error!(crate::LOG, "Disconnected from broker")
                        }
                        AgentNotification::Reconnection => {
                            error!(crate::LOG, "Reconnected to broker");
                        }
                        AgentNotification::Puback(_) => (),
                        AgentNotification::Pubrec(_) => (),
                        AgentNotification::Pubcomp(_) => (),
                        AgentNotification::Suback(_) => (),
                        AgentNotification::Unsuback(_) => (),
                        AgentNotification::Abort(err) => {
                            error!(crate::LOG, "MQTT client aborted: {:?}", err);
                        }
                    }
                });
            }
        });

        agent
            .subscribe(&Subscription::unicast_requests(), QoS::AtMostOnce, None)
            .context("Error subscribing to unicast requests")?;

        let conference_client = Arc::new(MqttConferenceClient::new(
            agent_id.clone(),
            config.conference.clone(),
            dispatcher.clone(),
            Some(Duration::from_secs(5)),
        ));

        let event_client = Arc::new(MqttEventClient::new(
            agent_id,
            config.event.clone(),
            dispatcher.clone(),
            Some(Duration::from_secs(5)),
        ));

        Ok(Self {
            db_pool,
            config,
            agent,
            dispatcher,
            conference_client,
            event_client,
        })
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
}

#[derive(Debug)]
pub enum ClientError {
    AgentError(AgentError),
    PayloadError(String),
    TimeoutError,
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
