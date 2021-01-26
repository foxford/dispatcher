use anyhow::{Context, Result};
use futures::StreamExt;
use jsonwebtoken::{TokenData, Validation};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};
use svc_agent::{
    mqtt::{Agent, AgentBuilder, AgentNotification, ConnectionMode},
    AgentId, Authenticable,
};
use svc_authn::token::jws_compact;
use svc_authn::token::jws_compact::extract::decode_jws_compact;
use svc_authn::{jose::Claims, Error};
use tide::http::url::Url;

use crate::config::Config;

const API_VERSION: &str = "v1";

#[derive(Clone)]
pub struct TideState {
    db_pool: PgPool,
    config: Config,
    agent: Agent,
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

        let (agent, rx) = AgentBuilder::new(agent_id, API_VERSION)
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

        async_std::task::spawn(async move {
            while let Some(message) = mq_rx.next().await {
                async_std::task::spawn(async move {
                    match message {
                        AgentNotification::Message(_, _) => {}
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

        Ok(Self {
            db_pool,
            config,
            agent,
        })
    }

    pub async fn get_conn(&self) -> Result<PoolConnection<Postgres>> {
        self.db_pool
            .acquire()
            .await
            .context("Failed to acquire DB connection")
    }

    pub fn default_frontend_base(&self) -> Url {
        self.config.default_frontend_base.clone()
    }

    pub fn validate_token(&self, token: &str) -> Result<TokenData<Claims<String>>, Error> {
        let verifier = Validation {
            iss: Some(self.config.api_auth.audience.clone()),
            algorithms: vec![self.config.api_auth.algorithm],
            ..Validation::default()
        };

        decode_jws_compact(
            token,
            &verifier,
            &self.config.api_auth.key,
            self.config.api_auth.algorithm,
        )
    }

    pub fn clone_agent(&self) -> Agent {
        self.agent.clone()
    }
}
