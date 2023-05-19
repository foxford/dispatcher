use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use signal_hook::consts::TERM_SIGNALS;
use sqlx::postgres::PgPool;
use svc_agent::{
    mqtt::{Agent, AgentBuilder, AgentNotification, ConnectionMode, IncomingMessage, QoS},
    request::Dispatcher,
    AgentId, Authenticable, SharedGroup, Subscription,
};
use svc_authn::token::jws_compact;
use svc_authz::cache::AuthzCache;
use svc_authz::ClientMap as Authz;
use svc_error::extension::sentry as svc_sentry;
use tracing::{error, info};

use crate::clients::event::{EventClient, TowerClient};
use crate::clients::tq::{HttpTqClient, TqClient};
use crate::config::{self, Config};
use crate::{
    app::metrics::MqttMetrics,
    clients::conference::{ConferenceClient, MqttConferenceClient},
};
pub use authz::AuthzObject;
use tide_state::message_handler::MessageHandler;
pub use tide_state::{AppContext, Publisher, TideState};

pub const API_VERSION: &str = "v1";

pub async fn run(db: PgPool, authz_cache: Option<Box<dyn AuthzCache>>) -> Result<()> {
    let config = config::load().context("Failed to load config")?;
    info!("App config: {:?}", config);

    let agent_id = AgentId::new(&config.agent_label, config.id.clone());
    info!("Agent id: {:?}", &agent_id);

    let token = jws_compact::TokenBuilder::new()
        .issuer(agent_id.as_account_id().audience())
        .subject(&agent_id)
        .key(config.id_token.algorithm, config.id_token.key.as_slice())
        .build()
        .context("Error creating an id token")?;

    let mut agent_config = config.mqtt.clone();
    agent_config.set_password(&token);

    let (mut agent, mut rx) = AgentBuilder::new(agent_id.clone(), API_VERSION)
        .connection_mode(ConnectionMode::Service)
        .start(&agent_config)
        .context("Failed to create an agent")?;

    let dispatcher = Arc::new(Dispatcher::new(&agent));
    let event_client = build_event_client(&config, dispatcher.clone());
    let conference_client = build_conference_client(&config, dispatcher.clone());
    let tq_client = build_tq_client(&config, &token);
    let authz = Authz::new(&config.id, authz_cache, config.authz.clone(), None)
        .context("Error converting authz config to clients")?;

    let state = TideState::new(
        db,
        config.clone(),
        event_client,
        conference_client,
        tq_client,
        agent.clone(),
        authz,
    );

    let state = match &config.nats {
        Some(cfg) => {
            let nats_client = svc_nats_client::Client::new(cfg.clone())
                .await
                .context("nats client")?;
            info!("Connected to nats");

            state.add_nats_client(nats_client)
        }
        None => state,
    };

    let state = Arc::new(state) as Arc<dyn AppContext>;
    let state_ = state.clone();

    let message_handler = Arc::new(MessageHandler::new(state_, dispatcher));
    tokio::task::spawn(async move {
        while let Some(message) = rx.recv().await {
            let message_handler_ = message_handler.clone();
            tokio::task::spawn(async move {
                match message {
                    AgentNotification::Message(Ok(IncomingMessage::Response(data)), _) => {
                        message_handler_.handle_response(data).await;
                    }
                    AgentNotification::Message(Ok(IncomingMessage::Event(data)), message_data) => {
                        let topic = message_data.topic;
                        message_handler_.handle_event(data, topic).await;
                    }
                    AgentNotification::Message(_, _) => (),
                    AgentNotification::ConnectionError => {
                        MqttMetrics::observe_connection_error();
                        error!("Connection to broker errored")
                    }
                    AgentNotification::Reconnection => {
                        error!("Reconnected to broker");
                        MqttMetrics::observe_reconnect();
                        resubscribe(
                            &mut message_handler_
                                .ctx()
                                .agent()
                                .expect("Cant disconnect without agent")
                                .to_owned(),
                            message_handler_.ctx().agent_id(),
                            message_handler_.ctx().config(),
                        );
                    }
                    AgentNotification::Puback(_) => (),
                    AgentNotification::Pubrec(_) => (),
                    AgentNotification::Pubcomp(_) => (),
                    AgentNotification::Suback(_) => (),
                    AgentNotification::Unsuback(_) => (),
                    AgentNotification::Connect(_) => (),
                    AgentNotification::Connack(_) => (),
                    AgentNotification::Pubrel(_) => (),
                    AgentNotification::Subscribe(_) => (),
                    AgentNotification::Unsubscribe(_) => (),
                    AgentNotification::PingReq => (),
                    AgentNotification::PingResp => (),
                    AgentNotification::Disconnect => {
                        MqttMetrics::observe_disconnect();
                    }
                }
            });
        }
    });

    subscribe(&mut agent, &agent_id, &config).expect("Failed to subscribe to required topics");
    let metrics_server =
        svc_utils::metrics::MetricsServer::new(config.http.metrics_listener_address);

    let router = http::router(state, config.authn.clone());

    let app_future = axum::Server::bind(&config.http.listener_address.parse().unwrap())
        .serve(router.into_make_service());
    pin_utils::pin_mut!(app_future);
    let mut signals_stream = signal_hook_tokio::Signals::new(TERM_SIGNALS)?.fuse();
    let signals = signals_stream.next();

    futures::future::select(app_future, signals).await;

    metrics_server.shutdown().await;

    // sleep for 2 secs to finish requests
    // this is very primitive way of waiting for them but
    // neither tide nor svc-agent (rumqtt) support graceful shutdowns
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}

fn subscribe(agent: &mut Agent, agent_id: &AgentId, config: &Config) -> Result<()> {
    agent
        .subscribe(&Subscription::unicast_requests(), QoS::AtMostOnce, None)
        .context("Error subscribing to unicast requests")?;

    let group = SharedGroup::new("loadbalancer", agent_id.as_account_id().clone());
    // Audience level events for each tenant
    for tenant_audience in &config.tenants {
        agent
            .subscribe(
                &Subscription::broadcast_events(
                    &config.conference_client.account_id,
                    &config.conference_client.api_version,
                    &format!("audiences/{}/events", tenant_audience),
                ),
                QoS::AtLeastOnce,
                Some(&group),
            )
            .context("Error subscribing to app's conference topic")?;

        agent
            .subscribe(
                &Subscription::broadcast_events(
                    &config.event_client.account_id,
                    &config.event_client.api_version,
                    &format!("audiences/{}/events", tenant_audience),
                ),
                QoS::AtLeastOnce,
                Some(&group),
            )
            .context("Error subscribing to app's events topic")?;

        agent
            .subscribe(
                &Subscription::broadcast_events(
                    &config.tq_client.account_id,
                    &config.tq_client.api_version,
                    &format!("audiences/{}/events", tenant_audience),
                ),
                QoS::AtLeastOnce,
                Some(&group),
            )
            .context("Error subscribing to app's tq topic")?;
    }

    Ok(())
}

fn resubscribe(agent: &mut Agent, agent_id: &AgentId, config: &Config) {
    if let Err(err) = subscribe(agent, agent_id, config) {
        let err = err.context("Failed to resubscribe after reconnection");
        error!("{:?}", err);

        svc_sentry::send(Arc::new(err))
            .unwrap_or_else(|err| error!("Error sending error to Sentry: {:?}", err));
    }
}

fn build_event_client(config: &Config, dispatcher: Arc<Dispatcher>) -> Arc<dyn EventClient> {
    let agent_id = AgentId::new(&config.agent_label, config.id.clone());

    Arc::new(TowerClient::new(
        agent_id,
        config.event_client.account_id.clone(),
        dispatcher,
        Some(Duration::from_secs(config.event_client.timeout)),
        &config.event_client.api_version,
    ))
}

fn build_conference_client(
    config: &Config,
    dispatcher: Arc<Dispatcher>,
) -> Arc<dyn ConferenceClient> {
    let agent_id = AgentId::new(&config.agent_label, config.id.clone());

    Arc::new(MqttConferenceClient::new(
        agent_id,
        config.conference_client.account_id.clone(),
        dispatcher,
        Some(Duration::from_secs(config.conference_client.timeout)),
        &config.conference_client.api_version,
    ))
}

fn build_tq_client(config: &Config, token: &str) -> Arc<dyn TqClient> {
    Arc::new(HttpTqClient::new(
        config.tq_client.base_url.clone(),
        token.to_owned(),
        config.tq_client.timeout,
        config.tq_client.audience_settings.clone(),
    ))
}

mod api;
mod authz;
pub mod error;
mod http;
mod info;
mod metrics;
mod postprocessing_strategy;
pub mod services;
mod tide_state;
pub mod turn_host;
mod stage;
