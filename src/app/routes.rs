use std::sync::Arc;

use tide::http::headers::HeaderValue;
use tide::security::{CorsMiddleware, Origin};
use tide::Server;

use super::AppContext;
use super::api::v1::authz::proxy as proxy_authz;
use super::api::v1::chat::{
    convert as convert_chat, create as create_chat, read_by_scope as read_chat_by_scope, read_chat,
};
use super::api::v1::minigroup::{
    create as create_minigroup, read as read_minigroup, read_by_scope as read_minigroup_by_scope,
    recreate as recreate_minigroup, update as update_minigroup,
    update_by_scope as update_minigroup_by_scope,
};
use super::api::v1::p2p::{
    convert as convert_p2p, create as create_p2p, read as read_p2p,
    read_by_scope as read_p2p_by_scope,
};
use super::api::v1::webinar::{
    convert as convert_webinar, create as create_webinar, download as download_webinar,
    options as read_options, read as read_webinar, read_by_scope as read_webinar_by_scope,
    recreate as recreate_webinar, update as update_webinar,
};
use super::api::{
    redirect_to_frontend, rollback, v1::create_event, v1::healthz,
    v1::redirect_to_frontend as redirect_to_frontend2,
};

use super::info::{list_frontends, list_scopes};

use super::{api::v1::AppEndpoint, metrics::AddMetrics};


pub fn bind_routes(app: &mut Server<Arc<dyn AppContext>>) {
    bind_redirects_routes(app);
    bind_webinars_routes(app);
    bind_p2p_routes(app);
    bind_minigroups_routes(app);
    bind_chat_routes(app);
    bind_authz_routes(app);
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

fn cors() -> CorsMiddleware {
    CorsMiddleware::new()
        .allow_methods("GET, OPTIONS, PUT".parse::<HeaderValue>().unwrap())
        .allow_origin(Origin::from("*"))
        .allow_headers("*".parse::<HeaderValue>().unwrap())
}
