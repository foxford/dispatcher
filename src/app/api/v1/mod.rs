use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, Path, Query, TypedHeader};
use headers::{authorization::Bearer, Authorization};
use hyper::{Body, Request, Response};
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde_derive::Deserialize;
use serde_json::Value as JsonValue;
use svc_agent::AccountId;
use tracing::error;
use url::Url;
use uuid::Uuid;

use super::FEATURE_POLICY;

use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::AsClassType;

pub type AppError = crate::app::error::Error;
pub type AppResult = Result<Response<Body>, AppError>;

pub async fn healthz() -> &'static str {
    "Ok"
}

pub async fn create_event(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(mut payload): Json<JsonValue>,
) -> AppResult {
    let account_id =
        validate_token(ctx.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;

    let class = find_class(ctx.as_ref(), id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();

    ctx.authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    payload["room_id"] = serde_json::to_value(class.event_room_id()).unwrap();

    let result = ctx.event_client().create_event(payload).await;
    if let Err(e) = &result {
        error!(
            classroom_id = ?id,
            "Failed to create event in event room, err = {:?}", e
        );
    }
    result
        .map_err(|e| anyhow!("Failed to create event, reason = {:?}", e))
        .error(AppErrorKind::InvalidPayload)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from("{}"))
        .unwrap();

    Ok(response)
}

pub async fn find_class(
    state: &dyn AppContext,
    id: Uuid,
) -> anyhow::Result<crate::db::class::Object> {
    let webinar = {
        let mut conn = state.get_conn().await?;
        crate::db::class::ReadQuery::by_id(id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find class"))?
    };
    Ok(webinar)
}

#[derive(Deserialize)]
pub struct RedirQuery {
    pub scope: String,
    pub app: String,
    pub audience: String,
}

pub async fn redirect_to_frontend(
    ctx: Extension<Arc<dyn AppContext>>,
    Query(query): Query<RedirQuery>,
    request: Request<Body>,
) -> AppResult {
    let conn = ctx.get_conn().await;
    let base_url = match conn {
        Err(e) => {
            error!("Failed to acquire conn: {:?}", e);
            None
        }
        Ok(mut conn) => {
            let fe = crate::db::frontend::FrontendByScopeQuery::new(
                query.scope.clone(),
                query.app.clone(),
            )
            .execute(&mut conn)
            .await;
            match fe {
                Err(e) => {
                    error!("Failed to find frontend: {:?}", e);
                    None
                }
                Ok(Some(frontend)) => {
                    let u = Url::parse(&frontend.url);
                    u.ok()
                }
                Ok(None) => None,
            }
        }
    };

    let mut url = base_url.unwrap_or_else(|| {
        super::build_default_url(ctx.default_frontend_base(), &query.audience, &query.app)
    });

    url.set_query(request.uri().query());

    // Add dispatcher base URL as `backurl` get parameter.
    let back_url = crate::app::api::build_back_url(&request)?.to_string();

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
        .body(Body::empty())
        .unwrap();

    Ok(response)
}

fn validate_token<T: std::ops::Deref<Target = dyn AppContext>>(
    ctx: T,
    token: &str,
) -> anyhow::Result<AccountId> {
    let account_id = ctx
        .validate_token(token)
        .context("Token authentication failed")?;

    Ok(account_id)
}

async fn find<T: AsClassType>(
    state: &dyn AppContext,
    id: Uuid,
) -> anyhow::Result<crate::db::class::Object> {
    let webinar = {
        let mut conn = state.get_conn().await?;
        crate::db::class::GenericReadQuery::<T>::by_id(id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find {}", T::as_str()))?
    };
    Ok(webinar)
}

async fn find_by_scope<T: AsClassType>(
    state: &dyn AppContext,
    audience: &str,
    scope: &str,
) -> anyhow::Result<crate::db::class::Object> {
    let webinar = {
        let mut conn = state.get_conn().await?;
        crate::db::class::GenericReadQuery::<T>::by_scope(audience, scope)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| anyhow!("Failed to find {} by scope", T::as_str()))?
    };
    Ok(webinar)
}

pub mod authz;
pub mod class;
pub mod minigroup;
pub mod p2p;
#[cfg(test)]
mod tests;
pub mod webinar;
