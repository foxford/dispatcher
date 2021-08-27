use std::time::Duration;
use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use futures::StreamExt;
use prometheus::{Encoder, TextEncoder};
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
use svc_error::{extension::sentry, Error as SvcError};
use tide::http::headers::HeaderValue;
use tide::security::{CorsMiddleware, Origin};

use crate::clients::event::{EventClient, MqttEventClient};
use crate::clients::tq::{HttpTqClient, TqClient};
use crate::config::{self, Config};
use crate::{
    app::metrics::MqttMetrics,
    clients::conference::{ConferenceClient, MqttConferenceClient},
};
use api::v1::authz::proxy as proxy_authz;
use api::v1::chat::{
    convert as convert_chat, create as create_chat, read_by_scope as read_chat_by_scope, read_chat,
};
use api::v1::minigroup::{
    create as create_minigroup, read as read_minigroup, read_by_scope as read_minigroup_by_scope,
    recreate as recreate_minigroup, update as update_minigroup,
    update_by_scope as update_minigroup_by_scope,
};
use api::v1::p2p::{
    convert as convert_p2p, create as create_p2p, read as read_p2p,
    read_by_scope as read_p2p_by_scope,
};
use api::v1::webinar::{
    convert as convert_webinar, create as create_webinar, download as download_webinar,
    options as read_options, read as read_webinar, read_by_scope as read_webinar_by_scope,
    recreate as recreate_webinar, update as update_webinar,
};
use api::{
    redirect_to_frontend, rollback, v1::create_event, v1::healthz,
    v1::redirect_to_frontend as redirect_to_frontend2,
};
pub use authz::AuthzObject;
use info::{list_frontends, list_scopes};
use tide_state::message_handler::MessageHandler;
pub use tide_state::{AppContext, Publisher, TideState};

use self::{api::v1::AppEndpoint, metrics::AddMetrics};

pub const API_VERSION: &str = "v1";

pub async fn run(db: PgPool, authz_cache: Option<Box<dyn AuthzCache>>) -> Result<()> {
    let config = config::load().context("Failed to load config")?;
    info!(crate::LOG, "App config: {:?}", config);

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
                    AgentNotification::ConnectionError => {
                        MqttMetrics::observe_connection_error();
                        error!(crate::LOG, "Connection to broker errored")
                    }
                    AgentNotification::Reconnection => {
                        error!(crate::LOG, "Reconnected to broker");
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
    async_std::task::spawn(start_metrics_collector(
        config.http.metrics_listener_address,
    ));

    let mut app = tide::with_state(state);
    app.with(request_logger::LogMiddleware::new());
    bind_redirects_routes(&mut app);
    bind_webinars_routes(&mut app);
    bind_p2p_routes(&mut app);
    bind_minigroups_routes(&mut app);
    bind_chat_routes(&mut app);
    bind_authz_routes(&mut app);

    let app_future = app.listen(config.http.listener_address);
    pin_utils::pin_mut!(app_future);
    let mut signals_stream = signal_hook_async_std::Signals::new(TERM_SIGNALS)?.fuse();
    let signals = signals_stream.next();

    futures::future::select(app_future, signals).await;

    // sleep for 2 secs to finish requests
    // this is very primitive way of waiting for them but
    // neither tide nor svc-agent (rumqtt) support graceful shutdowns
    async_std::task::sleep(Duration::from_secs(2)).await;
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
        let err = format!("Failed to resubscribe after reconnection: {:?}", err);
        error!(crate::LOG, "{:?}", err);

        let svc_error = SvcError::builder()
            .kind("resubscription_error", "Resubscription error")
            .detail(&err)
            .build();

        sentry::send(svc_error)
            .unwrap_or_else(|err| warn!(crate::LOG, "Error sending error to Sentry: {:?}", err));
    }
}

fn bind_redirects_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/info/scopes").with_metrics().get(list_scopes);

    app.at("/info/frontends").with_metrics().get(list_frontends);

    app.at("/redirs/tenants/:tenant/apps/:app")
        .with_metrics()
        .get(redirect_to_frontend);

    app.at("/api/scopes/:scope/rollback")
        .with_metrics()
        .post(rollback);

    app.at("/api/v1/healthz").with_metrics().get(healthz);

    app.at("/api/v1/scopes/:scope/rollback")
        .with_metrics()
        .post(rollback);

    app.at("/api/v1/redirs")
        .with_metrics()
        .get(redirect_to_frontend2);
}

fn bind_webinars_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/api/v1/webinars/:id")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_webinar));

    app.at("/api/v1/audiences/:audience/webinars/:scope")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_webinar_by_scope));

    app.at("/api/v1/webinars")
        .with_metrics()
        .post(AppEndpoint(create_webinar));

    app.at("/api/v1/webinars/:id")
        .with_metrics()
        .put(AppEndpoint(update_webinar));

    app.at("/api/v1/webinars/convert")
        .with_metrics()
        .post(AppEndpoint(convert_webinar));

    app.at("/api/v1/webinars/:id/download")
        .with_metrics()
        .get(AppEndpoint(download_webinar));

    app.at("/api/v1/webinars/:id/recreate")
        .with_metrics()
        .post(AppEndpoint(recreate_webinar));

    app.at("/api/v1/webinars/:id/events")
        .with_metrics()
        .post(AppEndpoint(create_event));
}

fn bind_p2p_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/api/v1/p2p/:id")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_p2p));

    app.at("/api/v1/audiences/:audience/p2p/:scope")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_p2p_by_scope));

    app.at("/api/v1/p2p")
        .with_metrics()
        .post(AppEndpoint(create_p2p));

    app.at("/api/v1/p2p/convert")
        .with_metrics()
        .post(AppEndpoint(convert_p2p));

    app.at("/api/v1/p2p/:id/events")
        .with_metrics()
        .post(AppEndpoint(create_event));
}

fn bind_minigroups_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/api/v1/minigroups/:id")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_minigroup));

    app.at("/api/v1/audiences/:audience/minigroups/:scope")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_minigroup_by_scope))
        .put(AppEndpoint(update_minigroup_by_scope));

    app.at("/api/v1/minigroups/:id/recreate")
        .with_metrics()
        .post(AppEndpoint(recreate_minigroup));

    app.at("/api/v1/minigroups")
        .with_metrics()
        .post(AppEndpoint(create_minigroup));
    app.at("/api/v1/minigroups/:id")
        .with_metrics()
        .put(AppEndpoint(update_minigroup));

    app.at("/api/v1/minigroups/:id/events")
        .with_metrics()
        .post(AppEndpoint(create_event));
}

fn bind_chat_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/api/v1/chats/:id")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_chat));

    app.at("/api/v1/audiences/:audience/chats/:scope")
        .with_metrics()
        .with(cors())
        .options(read_options)
        .get(AppEndpoint(read_chat_by_scope));

    app.at("/api/v1/chats")
        .with_metrics()
        .post(AppEndpoint(create_chat));

    app.at("/api/v1/chats/convert")
        .with_metrics()
        .post(AppEndpoint(convert_chat));

    app.at("/api/v1/chats/:id/events")
        .with_metrics()
        .post(AppEndpoint(create_event));
}

fn bind_authz_routes(app: &mut tide::Server<Arc<dyn AppContext>>) {
    app.at("/api/v1/authz/:audience")
        .with_metrics()
        .post(AppEndpoint(proxy_authz));
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

fn build_tq_client(config: &Config, token: &str) -> Arc<dyn TqClient> {
    Arc::new(HttpTqClient::new(
        config.tq_client.base_url.clone(),
        token.to_owned(),
        config.tq_client.timeout,
    ))
}

fn cors() -> CorsMiddleware {
    CorsMiddleware::new()
        .allow_methods("GET, OPTIONS, PUT".parse::<HeaderValue>().unwrap())
        .allow_origin(Origin::from("*"))
        .allow_headers("*".parse::<HeaderValue>().unwrap())
}

async fn start_metrics_collector(bind_addr: SocketAddr) -> async_std::io::Result<()> {
    let mut app = tide::with_state(());
    app.at("/metrics")
        .get(|_req: tide::Request<()>| async move {
            let mut buffer = vec![];
            let encoder = TextEncoder::new();
            let metric_families = prometheus::gather();
            match encoder.encode(&metric_families, &mut buffer) {
                Ok(_) => {
                    let mut response = tide::Response::new(200);
                    response.set_body(buffer);
                    Ok(response)
                }
                Err(err) => {
                    warn!(crate::LOG, "Metrics not gathered: {:?}", err);
                    Ok(tide::Response::new(500))
                }
            }
        });
    app.listen(bind_addr).await
}

mod api;
mod authz;
mod error;
mod info;
mod metrics;
mod postprocessing_strategy;
mod request_logger;
mod services;
mod tide_state;
