use std::sync::Arc;

use axum::routing::Router;
use axum::routing::{get, options, post};
use axum::AddExtensionLayer;
use svc_utils::middleware::{CorsLayer, LogLayer, MeteredRoute};

use super::api::v1::authz::proxy as proxy_authz;
use super::api::v1::class::{
    commit_edition, create_timestamp, read, read_by_scope, read_property, recreate, update,
    update_by_scope, update_property,
};
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

use crate::app::AppContext;
use crate::db::class::{MinigroupType, P2PType, WebinarType};

pub fn router(ctx: Arc<dyn AppContext>, authn: svc_authn::jose::ConfigMap) -> Router {
    let router = redirects_router()
        .merge(webinars_router())
        .merge(p2p_router())
        .merge(minigroups_router())
        .merge(authz_router())
        .merge(utils_router());

    router
        .layer(AddExtensionLayer::new(Arc::new(authn)))
        .layer(AddExtensionLayer::new(ctx))
        .layer(LogLayer::new())
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
        .metered_route(
            "/api/v1/redirs/tenants/:tenant/apps/:app",
            get(redirect_to_frontend2),
        )
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
        .metered_route(
            "/api/v1/webinars/:id/timestamps",
            options(read_options).post(create_timestamp::<WebinarType>),
        )
        .layer(CorsLayer)
        .metered_route("/api/v1/webinars", post(create_webinar))
        .metered_route("/api/v1/webinars/convert", post(convert_webinar))
        .metered_route("/api/v1/webinars/:id/download", get(download_webinar))
        .metered_route(
            "/api/v1/webinars/:id/recreate",
            post(recreate::<WebinarType>),
        )
        .metered_route("/api/v1/webinars/:id/events", post(create_event))
        .metered_route(
            "/api/v1/webinars/:id/properties/:property_id",
            get(read_property).put(update_property),
        )
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
        .layer(CorsLayer)
        .metered_route("/api/v1/p2p", post(create_p2p))
        .metered_route("/api/v1/p2p/convert", post(convert_p2p))
        .metered_route("/api/v1/p2p/:id/events", post(create_event))
        .metered_route(
            "/api/v1/p2p/:id/properties/:property_id",
            get(read_property).put(update_property),
        )
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
        .metered_route(
            "/api/v1/minigroups/:id/timestamps",
            options(read_options).post(create_timestamp::<MinigroupType>),
        )
        .layer(CorsLayer)
        .metered_route(
            "/api/v1/minigroups/:id/recreate",
            post(recreate::<MinigroupType>),
        )
        .metered_route("/api/v1/minigroups", post(create_minigroup))
        .metered_route("/api/v1/minigroups/:id/download", get(download_minigroup))
        .metered_route("/api/v1/minigroups/:id/events", post(create_event))
        .metered_route(
            "/api/v1/minigroups/:id/properties/:property_id",
            get(read_property).put(update_property),
        )
}

fn authz_router() -> Router {
    Router::new().metered_route("/api/v1/authz/:audience", post(proxy_authz))
}

fn utils_router() -> Router {
    Router::new().metered_route(
        "/api/v1/audiences/:audience/classes/:scope/editions/:id",
        post(commit_edition),
    )
}
