use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Path};
use http::{StatusCode, Uri};
use hyper::{Body, Request, Response};
use serde_derive::Deserialize;
use svc_agent::{
    mqtt::{
        IntoPublishableMessage, OutgoingEvent, OutgoingEventProperties, ShortTermTimingProperties,
    },
    Authenticable,
};
use svc_utils::extractors::AccountIdExtractor;
use tracing::error;

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

pub async fn rollback(
    ctx: Extension<Arc<dyn AppContext>>,
    Path(scope): Path<String>,
    AccountIdExtractor(account_id): AccountIdExtractor,
) -> Response<Body> {
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

    Response::builder().body(Body::from("Ok")).unwrap()
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
        .context("Failed to build back_url")
        .error(AppErrorKind::InvalidParameter)?;

    Ok(absolute_uri)
}

trait IntoJsonResponse
where
    Self: Sized,
{
    fn into_json_response(self, ctx: &'static str, status_code: StatusCode) -> AppResult;
}

impl<T> IntoJsonResponse for T
where
    T: serde::Serialize,
{
    fn into_json_response(self, ctx: &'static str, status_code: StatusCode) -> AppResult {
        let body = serde_json::to_string(&self)
            .context(ctx)
            .error(AppErrorKind::SerializationFailed)?;
        let response = Response::builder()
            .status(status_code)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        Ok(response)
    }
}

pub mod v1;
