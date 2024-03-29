use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::{rejection::JsonRejection, Extension, FromRequest},
    routing::{get, post, Router},
};
use http::Request;
use svc_utils::middleware::{CorsLayer, LogLayer, MeteredRoute};

use super::api::v1::class::{
    commit_edition, create_timestamp, read, read_by_scope, read_property, recreate, update,
    update_by_scope, update_property,
};
use super::api::v1::minigroup::{
    create as create_minigroup, create_whiteboard, download as download_minigroup,
};
use super::api::v1::p2p::{convert as convert_p2p, create as create_p2p};
use super::api::v1::webinar::{
    convert_webinar, create_webinar, create_webinar_replica, download_webinar,
};
use super::api::v1::{
    account, minigroup::restart_transcoding as restart_transcoding_minigroup,
    webinar::restart_transcoding as restart_transcoding_webinar,
};
use super::api::{rollback, v1::create_event, v1::healthz, v1::redirect_to_frontend};
use super::info::{list_frontends, list_scopes};
use super::{api::v1::authz::proxy as proxy_authz, error::ErrorExt};

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
        .layer(Extension(Arc::new(authn)))
        .layer(Extension(ctx))
        .layer(LogLayer::new())
}

fn redirects_router() -> Router {
    Router::new()
        .metered_route("/info/scopes", get(list_scopes))
        .metered_route("/info/frontends", get(list_frontends))
        .metered_route("/healthz", get(healthz))
        .metered_route("/api/v1/scopes/:scope/rollback", post(rollback))
        .metered_route("/api/v1/redirs", get(redirect_to_frontend))
        .metered_route(
            "/api/v1/redirs/tenants/:tenant/apps/:app",
            get(redirect_to_frontend),
        )
}

fn webinars_router() -> Router {
    Router::new()
        .metered_route(
            "/api/v1/webinars/:id",
            get(read::<WebinarType>).put(update::<WebinarType>),
        )
        .metered_route(
            "/api/v1/audiences/:audience/webinars/:scope",
            get(read_by_scope::<WebinarType>),
        )
        .metered_route(
            "/api/v1/webinars/:id/timestamps",
            post(create_timestamp::<WebinarType>),
        )
        .metered_route("/api/v1/webinars/:id/events", post(create_event))
        .metered_route(
            "/api/v1/webinars/:id/properties/:property_id",
            get(read_property).put(update_property),
        )
        .layer(CorsLayer)
        .metered_route("/api/v1/webinars", post(create_webinar))
        .metered_route(
            "/api/v1/webinars/:id/replicas",
            post(create_webinar_replica),
        )
        .metered_route("/api/v1/webinars/convert", post(convert_webinar))
        .metered_route("/api/v1/webinars/:id/download", get(download_webinar))
        .metered_route(
            "/api/v1/webinars/:id/recreate",
            post(recreate::<WebinarType>),
        )
}

fn p2p_router() -> Router {
    Router::new()
        .metered_route("/api/v1/p2p/:id", get(read::<P2PType>))
        .metered_route(
            "/api/v1/audiences/:audience/p2p/:scope",
            get(read_by_scope::<P2PType>),
        )
        .metered_route(
            "/api/v1/p2p/:id/properties/:property_id",
            get(read_property).put(update_property),
        )
        .layer(CorsLayer)
        .metered_route("/api/v1/p2p", post(create_p2p))
        .metered_route("/api/v1/p2p/convert", post(convert_p2p))
        .metered_route("/api/v1/p2p/:id/events", post(create_event))
}

fn minigroups_router() -> Router {
    Router::new()
        .metered_route(
            "/api/v1/minigroups/:id",
            get(read::<MinigroupType>).put(update::<MinigroupType>),
        )
        .metered_route(
            "/api/v1/audiences/:audience/minigroups/:scope",
            get(read_by_scope::<MinigroupType>).put(update_by_scope::<MinigroupType>),
        )
        .metered_route(
            "/api/v1/minigroups/:id/timestamps",
            post(create_timestamp::<MinigroupType>),
        )
        .metered_route(
            "/api/v1/minigroups/:id/properties/:property_id",
            get(read_property).put(update_property),
        )
        .layer(CorsLayer)
        .metered_route(
            "/api/v1/minigroups/:id/recreate",
            post(recreate::<MinigroupType>),
        )
        .metered_route("/api/v1/minigroups", post(create_minigroup))
        .metered_route("/api/v1/minigroups/:id/download", get(download_minigroup))
        .metered_route("/api/v1/minigroups/:id/events", post(create_event))
        .metered_route("/api/v1/minigroups/:id/whiteboard", post(create_whiteboard))
}

fn authz_router() -> Router {
    Router::new().metered_route("/api/v1/authz/:audience", post(proxy_authz))
}

fn utils_router() -> Router {
    Router::new()
        .metered_route(
            "/api/v1/audiences/:audience/classes/:scope/editions/:id",
            post(commit_edition),
        )
        .metered_route(
            "/api/v1/account/properties/:property_id",
            get(account::read_property).put(account::update_property),
        )
        .metered_route(
            "/api/v1/transcoding/minigroup/:id/restart",
            post(restart_transcoding_minigroup),
        )
        .metered_route(
            "/api/v1/transcoding/webinar/:id/restart",
            post(restart_transcoding_webinar),
        )
        .layer(CorsLayer)
}

pub struct Json<T>(pub T);

#[async_trait]
impl<S, B, T> FromRequest<S, B> for Json<T>
where
    axum::Json<T>: FromRequest<S, B, Rejection = JsonRejection>,
    B: Send + 'static,
    S: Sync,
{
    type Rejection = super::error::Error;

    async fn from_request(req: Request<B>, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(value) => Ok(Self(value.0)),
            Err(rejection) => {
                let kind = match rejection {
                    JsonRejection::JsonDataError(_)
                    | JsonRejection::JsonSyntaxError(_)
                    | JsonRejection::MissingJsonContentType(_) => {
                        super::error::ErrorKind::InvalidPayload
                    }
                    _ => super::error::ErrorKind::InternalFailure,
                };

                Err(rejection).error(kind)
            }
        }
    }
}
