use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use sqlx::postgres::PgPool;
use svc_agent::{
    mqtt::{AgentBuilder, AgentNotification, ConnectionMode, IncomingMessage, QoS},
    request::Dispatcher,
    AgentId, Authenticable, SharedGroup, Subscription,
};
use svc_authn::token::jws_compact;
use svc_authz::cache::AuthzCache;
use svc_authz::ClientMap as Authz;
use tide::http::headers::HeaderValue;
use tide::security::{CorsMiddleware, Origin};

use crate::app::authz::db_ban_callback;
use crate::config::{self, Config};
use api::v1::classroom::{
    convert as convert_classroom, create as create_classroom,
    read_by_scope as read_classroom_by_scope,
};
use api::v1::webinar::{
    convert as convert_webinar, create as create_webinar, options as read_options,
    read as read_webinar, read_by_scope as read_webinar_by_scope, update as update_webinar,
};
use api::{
    redirect_to_frontend, rollback, v1::healthz, v1::redirect_to_frontend as redirect_to_frontend2,
};
use info::{list_frontends, list_scopes};
use tide_state::conference_client::{ConferenceClient, MqttConferenceClient};
use tide_state::event_client::{EventClient, MqttEventClient};
use tide_state::message_handler::MessageHandler;
use tide_state::tq_client::{HttpTqClient, TqClient};
pub use tide_state::{AppContext, TideState};

const API_VERSION: &str = "v1";

pub async fn run(db: PgPool, authz_cache: Option<Box<dyn AuthzCache>>) -> Result<()> {
    let config = config::load().context("Failed to load config")?;
    info!(crate::LOG, "App config: {:?}", config);
    tide::log::start();

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

    let dispatcher = Arc::new(Dispatcher::new(&agent));
    let event_client = build_event_client(&config, dispatcher.clone());
    let conference_client = build_conference_client(&config, dispatcher.clone());
    let tq_client = build_tq_client(&config);
    let authz = Authz::new(
        &config.id,
        authz_cache,
        config.authz.clone(),
        db_ban_callback(),
    )
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
    let state = Arc::new(state) as Arc<dyn AppContext>;
    let state_ = state.clone();

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

    let message_handler = Arc::new(MessageHandler::new(state_, dispatcher));
    async_std::task::spawn(async move {
        while let Some(message) = mq_rx.next().await {
            let message_handler_ = message_handler.clone();
            async_std::task::spawn(async move {
                match message {
                    AgentNotification::Message(Ok(IncomingMessage::Response(data)), _) => {
                        message_handler_.handle_response(data).await;
                    }
                    AgentNotification::Message(Ok(IncomingMessage::Event(data)), message_data) => {
                        let topic = message_data.topic;
                        message_handler_.handle_event(data, topic).await;
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
            .context("Error subscribing to app's events topic")?;

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
    }

    let mut app = tide::with_state(state);
    bind_redirects_routes(&mut app);
    bind_webinars_routes(&mut app);
    bind_classroom_routes(&mut app);

    app.listen(config.http.listener_address).await?;
    Ok(())
}

fn bind_redirects_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/info/scopes").get(list_scopes);
    app.at("/info/frontends").get(list_frontends);
    app.at("/redirs/tenants/:tenant/apps/:app")
        .get(redirect_to_frontend);
    app.at("/api/scopes/:scope/rollback").post(rollback);

    app.at("/api/v1/healthz").get(healthz);
    app.at("/api/v1/scopes/:scope/rollback").post(rollback);
    app.at("/api/v1/redirs").get(redirect_to_frontend2);
}

fn bind_webinars_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/api/v1/webinars/:id")
        .with(cors())
        .options(read_options);
    app.at("/api/v1/audiences/:audience/webinars/:scope")
        .with(cors())
        .options(read_options);
    app.at("/api/v1/webinars/:id")
        .with(cors())
        .get(read_webinar);
    app.at("/api/v1/audiences/:audience/webinars/:scope")
        .with(cors())
        .get(read_webinar_by_scope);

    app.at("/api/v1/webinars").post(create_webinar);
    app.at("/api/v1/webinars/:id").put(update_webinar);

    app.at("/api/v1/webinars/convert").post(convert_webinar);
}

fn bind_classroom_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/api/v1/audiences/:audience/webinars/:scope")
        .with(cors())
        .options(read_options);
    app.at("/api/v1/audiences/:audience/webinars/:scope")
        .with(cors())
        .get(read_classroom_by_scope);

    app.at("/api/v1/classroom").post(create_classroom);

    app.at("/api/v1/classroom/convert").post(convert_classroom);
}

fn build_event_client(config: &Config, dispatcher: Arc<Dispatcher>) -> Arc<dyn EventClient> {
    let agent_id = AgentId::new(&config.agent_label, config.id.clone());

    Arc::new(MqttEventClient::new(
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

fn build_tq_client(config: &Config) -> Arc<dyn TqClient> {
    Arc::new(HttpTqClient::new(config.tq_client.base_url.clone()))
}

fn cors() -> CorsMiddleware {
    CorsMiddleware::new()
        .allow_methods("GET, OPTIONS".parse::<HeaderValue>().unwrap())
        .allow_origin(Origin::from("*"))
        .allow_headers("*".parse::<HeaderValue>().unwrap())
}

mod api;
mod authz;
mod error;
mod info;
mod tide_state;
