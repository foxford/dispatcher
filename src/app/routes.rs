use std::{sync::Arc, time::Duration};

use axum::routing::Router;
use axum::routing::{get, options, post};
use axum::AddExtensionLayer;
use hyper::{body::HttpBody, Body, Request, Response};
use tower_http::trace::TraceLayer;
use tracing::{error, field::Empty, info, Span};

use super::api::v1::authz::proxy as proxy_authz;
use super::api::v1::chat::{convert as convert_chat, create as create_chat};
use super::api::v1::class::{read, read_by_scope, recreate, update, update_by_scope};
use super::api::v1::minigroup::{create as create_minigroup, download as download_minigroup};
use super::api::v1::p2p::{convert as convert_p2p, create as create_p2p};
use super::api::v1::webinar::{
    convert as convert_webinar, create as create_webinar, download as download_webinar,
    options as read_options,
};
use super::api::{
    redirect_to_frontend, rollback, v1::create_event, v1::healthz,
    v1::redirect_to_frontend as redirect_to_frontend2,
};
use super::info::{list_frontends, list_scopes};

use super::middleware::CorsMiddlewareLayer;

use crate::app::metrics::MeteredRoute;
use crate::app::AppContext;
use crate::db::class::{ChatType, MinigroupType, P2PType, WebinarType};

pub fn router(ctx: Arc<dyn AppContext>) -> Router {
    let router = redirects_router();
    let router = router.merge(webinars_router());
    let router = router.merge(p2p_router());
    let router = router.merge(minigroups_router());
    let router = router.merge(chat_router());
    let router = router.merge(authz_router());

    router.layer(AddExtensionLayer::new(ctx)).layer(
        TraceLayer::new_for_http()
            .make_span_with(|request: &Request<Body>| {
                tracing::error_span!(
                    "http-api-request",
                    status_code = Empty,
                    path = request.uri().path(),
                    query = request.uri().query(),
                    body_size = ?request.body().size_hint().upper()
                )
            })
            .on_response(|response: &Response<_>, latency: Duration, span: &Span| {
                span.record("status_code", &tracing::field::debug(response.status()));
                if response.status().is_success() {
                    info!("response generated in {:?}", latency)
                } else {
                    error!("response generated in {:?}", latency)
                }
            }),
    )
}

fn redirects_router() -> Router {
    Router::new()
        .metered_route("/info/scopes", get(list_scopes))
        .metered_route("/info/frontends", get(list_frontends))
        .metered_route(
            "/redirs/tenants/:tenant/apps/:app",
            get(redirect_to_frontend),
        )
        .metered_route("/api/scopes/:scope/rollback", post(rollback))
        .metered_route("/api/v1/healthz", get(healthz))
        .metered_route("/api/v1/scopes/:scope/rollback", post(rollback))
        .metered_route("/api/v1/redirs", get(redirect_to_frontend2))
}

fn webinars_router() -> Router {
    Router::new()
        .metered_route(
            "/api/v1/webinars/:id",
            options(read_options)
                .get(read::<WebinarType>)
                .put(update::<WebinarType>),
        )
        .metered_route(
            "/api/v1/audiences/:audience/webinars/:scope",
            options(read_options).get(read_by_scope::<WebinarType>),
        )
        .layer(CorsMiddlewareLayer)
        .metered_route("/api/v1/webinars", post(create_webinar))
        .metered_route("/api/v1/webinars/convert", post(convert_webinar))
        .metered_route("/api/v1/webinars/:id/download", get(download_webinar))
        .metered_route(
            "/api/v1/webinars/:id/recreate",
            post(recreate::<WebinarType>),
        )
        .metered_route("/api/v1/webinars/:id/events", post(create_event))
}

fn p2p_router() -> Router {
    Router::new()
        .metered_route(
            "/api/v1/p2p/:id",
            options(read_options).get(read::<P2PType>),
        )
        .metered_route(
            "/api/v1/audiences/:audience/p2p/:scope",
            options(read_options).get(read_by_scope::<P2PType>),
        )
        .layer(CorsMiddlewareLayer)
        .metered_route("/api/v1/p2p", post(create_p2p))
        .metered_route("/api/v1/p2p/convert", post(convert_p2p))
        .metered_route("/api/v1/p2p/:id/events", post(create_event))
}

fn minigroups_router() -> Router {
    Router::new()
        .metered_route(
            "/api/v1/minigroups/:id",
            options(read_options)
                .get(read::<MinigroupType>)
                .put(update::<MinigroupType>),
        )
        .metered_route(
            "/api/v1/audiences/:audience/minigroups/:scope",
            options(read_options)
                .get(read_by_scope::<MinigroupType>)
                .put(update_by_scope::<MinigroupType>),
        )
        .layer(CorsMiddlewareLayer)
        .metered_route(
            "/api/v1/minigroups/:id/recreate",
            post(recreate::<MinigroupType>),
        )
        .metered_route("/api/v1/minigroups", post(create_minigroup))
        .metered_route("/api/v1/minigroups/:id/download", get(download_minigroup))
        .metered_route("/api/v1/minigroups/:id/events", post(create_event))
}

fn chat_router() -> Router {
    Router::new()
        .metered_route(
            "/api/v1/chats/:id",
            options(read_options).get(read::<ChatType>),
        )
        .metered_route(
            "/api/v1/audiences/:audience/chats/:scope",
            options(read_options).get(read_by_scope::<ChatType>),
        )
        .layer(CorsMiddlewareLayer)
        .metered_route("/api/v1/chats", post(create_chat))
        .metered_route("/api/v1/chats/convert", post(convert_chat))
        .metered_route("/api/v1/chats/:id/events", post(create_event))
}

fn authz_router() -> Router {
    Router::new().metered_route("/api/v1/authz/:audience", post(proxy_authz))
}
