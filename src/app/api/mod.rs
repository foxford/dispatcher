use std::sync::Arc;

use axum::extract::{Extension, Path, Query, TypedHeader};
use headers::{authorization::Bearer, Authorization};
use http::Uri;
use hyper::{Body, Request, Response};
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde_derive::Deserialize;
use svc_agent::{
    mqtt::{
        IntoPublishableMessage, OutgoingEvent, OutgoingEventProperties, ShortTermTimingProperties,
    },
    Authenticable,
};
use tracing::error;
use url::Url;

use crate::app::api::v1::AppResult;
use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::{Error as AppError, ErrorKind as AppErrorKind};
use crate::app::AppContext;

use super::metrics::AuthorizeMetrics;

const FEATURE_POLICY: &str = "autoplay *; camera *; microphone *; display-capture *; fullscreen *";

#[derive(Deserialize)]
pub struct RedirQuery {
    pub scope: String,
}

pub async fn redirect_to_frontend(
    ctx: Extension<Arc<dyn AppContext>>,
    Path((tenant, app)): Path<(String, String)>,
    Query(query): Query<RedirQuery>,
    request: Request<Body>,
) -> AppResult {
    let base_url = {
        let conn = ctx.get_conn().await;
        match conn {
            Err(e) => {
                error!("Failed to acquire conn: {}", e);
                None
            }
            Ok(mut conn) => {
                let fe =
                    crate::db::frontend::FrontendByScopeQuery::new(query.scope, app.to_owned())
                        .execute(&mut conn)
                        .await;
                match fe {
                    Err(e) => {
                        error!("Failed to find frontend: {}", e);
                        None
                    }
                    Ok(Some(frontend)) => {
                        let u = Url::parse(&frontend.url);
                        u.ok()
                    }
                    Ok(None) => None,
                }
            }
        }
    };

    let mut url =
        base_url.unwrap_or_else(|| build_default_url(ctx.default_frontend_base(), &tenant, &app));

    url.set_query(request.uri().query());

    // Add dispatcher base URL as `backurl` get parameter.
    let back_url = build_back_url(&request)?.to_string();

    // Percent-encode it since it's being passed as a get parameter.
    let urlencoded_back_url =
        percent_encode(back_url.as_str().as_bytes(), NON_ALPHANUMERIC).to_string();

    url.query_pairs_mut()
        .append_pair("backurl", &urlencoded_back_url);

    let url = url.to_string();

    let response = Response::builder()
        .status(307)
        .header("Location", &url)
        .header("Feature-Policy", FEATURE_POLICY)
        .body(hyper::Body::empty())
        .unwrap();

    Ok(response)
}

pub async fn rollback(
    ctx: Extension<Arc<dyn AppContext>>,
    Path(scope): Path<String>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
) -> Response<Body> {
    match ctx.validate_token(token.token()) {
        Ok(account_id) => {
            let object = AuthzObject::new(&["scopes"]).into();

            if let Err(err) = ctx
                .authz()
                .authorize(
                    ctx.agent_id().as_account_id().audience().to_string(),
                    account_id.clone(),
                    object,
                    "rollback".into(),
                )
                .await
                .measure()
            {
                error!("Failed to authorize action, reason = {:?}", err);
                return Response::builder()
                    .status(403)
                    .body(Body::from("Access denied"))
                    .unwrap();
            }

            match ctx.get_conn().await {
                Err(err) => {
                    error!("Failed to get db conn, reason = {:?}", err);

                    return Response::builder()
                        .status(500)
                        .body(Body::from(format!("Failed to acquire conn: {}", err)))
                        .unwrap();
                }
                Ok(mut conn) => {
                    let r = crate::db::scope::DeleteQuery::new(scope.clone())
                        .execute(&mut conn)
                        .await;

                    if let Err(err) = r {
                        error!("Failed to delete scope from db, reason = {:?}", err);

                        return Response::builder()
                            .status(500)
                            .body(Body::from(format!("Failed to delete scope: {}", err)))
                            .unwrap();
                    }

                    let timing = ShortTermTimingProperties::new(chrono::Utc::now());
                    let props = OutgoingEventProperties::new("scope.frontend.rollback", timing);
                    let path = format!("scopes/{}/events", scope);
                    let event = OutgoingEvent::broadcast("", props, &path);
                    let e = Box::new(event) as Box<dyn IntoPublishableMessage + Send>;

                    if let Err(err) = ctx.publisher().publish(e) {
                        error!(
                            "Failed to publish scope.frontend.rollback event, reason = {:?}",
                            err
                        );
                    }
                }
            }
        }
        Err(e) => {
            error!(
                header = ?token,
                "Failed to process Authorization header, err = {:?}", e
            );
            return Response::builder()
                .status(403)
                .body(Body::from("Access denied"))
                .unwrap();
        }
    }

    Response::builder().body(Body::from("Ok")).unwrap()
}

fn build_default_url(mut url: Url, tenant: &str, app: &str) -> Url {
    let host = url.host_str().map(|h| format!("{}.{}.{}", tenant, app, h));
    if let Err(e) = url.set_host(host.as_deref()) {
        error!("Default url set_host failed, reason = {:?}", e);
    }
    url
}

fn build_back_url<B>(request: &Request<B>) -> Result<Uri, AppError> {
    let path = request
        .uri()
        .path_and_query()
        // We dont need query, only path
        .map(|v| v.path())
        .ok_or_else(|| anyhow!("No path and query in uri"))
        .error(AppErrorKind::InvalidParameter)?
        .to_owned();

    let host = request
        .headers()
        .get("Host")
        .and_then(|host| host.to_str().ok())
        .ok_or_else(|| anyhow!("Invalid host header"))
        .error(AppErrorKind::InvalidParameter)?;

    let absolute_uri = hyper::Uri::builder()
        .authority(host)
        .path_and_query(path)
        // Ingress terminates https so set it back.
        .scheme("https")
        .build()
        .map_err(|e| anyhow!("Failed to build back_url, e = {:?}", e))
        .error(AppErrorKind::InvalidParameter)?;

    Ok(absolute_uri)
}

pub mod v1;
