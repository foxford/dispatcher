use std::str::FromStr;
use std::sync::Arc;

use crate::app::AppContext;
use anyhow::Context;
use async_trait::async_trait;
use futures::Future;
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use svc_agent::AccountId;
use tide::{Endpoint, Request, Response};
use uuid::Uuid;

use super::FEATURE_POLICY;

type AppError = crate::app::error::Error;
type AppResult = Result<tide::Response, AppError>;

pub struct AppEndpoint<E>(pub E);

#[async_trait]
impl<E, S, F> Endpoint<S> for AppEndpoint<E>
where
    E: Fn(tide::Request<S>) -> F + Send + Sync + 'static,
    F: Future<Output = AppResult> + Send + 'static,
    S: Clone + Send + Sync + 'static,
{
    async fn call(&self, req: tide::Request<S>) -> tide::Result {
        let resp = (self.0)(req).await;
        Ok(match resp {
            Ok(resp) => resp,
            Err(err) => {
                let mut tide_resp = err.to_tide_response();
                tide_resp.set_error(err);
                tide_resp
            }
        })
    }
}

pub async fn healthz(_req: Request<Arc<dyn AppContext>>) -> tide::Result {
    Ok("Ok".into())
}

#[derive(Deserialize)]
struct RedirQuery {
    pub scope: String,
    pub app: String,
    pub audience: String,
}

pub async fn redirect_to_frontend(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let query = match req.query::<RedirQuery>() {
        Ok(query) => query,
        Err(e) => {
            error!(crate::LOG, "Failed to parse query: {:?}", e);
            return Ok(Response::builder(tide::StatusCode::NotFound)
                .body(format!("Failed to parse query: {:?}", e))
                .build());
        }
    };

    let conn = req.state().get_conn().await;
    let base_url = match conn {
        Err(e) => {
            error!(crate::LOG, "Failed to acquire conn: {}", e);
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
                    error!(crate::LOG, "Failed to find frontend: {}", e);
                    None
                }
                Ok(Some(frontend)) => {
                    let u = tide::http::url::Url::parse(&frontend.url);
                    u.ok()
                }
                Ok(None) => None,
            }
        }
    };

    let mut url = base_url.unwrap_or_else(|| {
        super::build_default_url(
            req.state().default_frontend_base(),
            &query.audience,
            &query.app,
        )
    });

    url.set_query(req.url().query());

    // Add dispatcher base URL as `backurl` get parameter.
    let mut back_url = req.url().to_owned();
    back_url.set_query(None);

    // Ingress terminates https so set it back.
    back_url
        .set_scheme("https")
        .map_err(|()| anyhow!("Failed to set https scheme"))?;

    // Percent-encode it since it's being passed as a get parameter.
    let urlencoded_back_url =
        percent_encode(back_url.as_str().as_bytes(), NON_ALPHANUMERIC).to_string();

    url.query_pairs_mut()
        .append_pair("backurl", &urlencoded_back_url);

    let url = url.to_string();

    let response = Response::builder(307)
        .header("Location", &url)
        .header("Feature-Policy", FEATURE_POLICY)
        .build();

    Ok(response)
}

fn validate_token<T: std::ops::Deref<Target = dyn AppContext>>(
    req: &Request<T>,
) -> anyhow::Result<AccountId> {
    let token = req
        .header("Authorization")
        .and_then(|h| h.get(0))
        .map(|header| header.to_string());

    let state = req.state();
    let account_id = state
        .validate_token(token.as_deref())
        .context("Token authentication failed")?;

    Ok(account_id)
}

fn extract_param<'a>(req: &'a Request<Arc<dyn AppContext>>, key: &str) -> anyhow::Result<&'a str> {
    req.param(key)
        .map_err(|e| anyhow!("Failed to get {}, reason = {:?}", key, e))
}

fn extract_id(req: &Request<Arc<dyn AppContext>>) -> anyhow::Result<Uuid> {
    let id = extract_param(req, "id")?;
    let id = Uuid::from_str(id)
        .map_err(|e| anyhow!("Failed to convert id to uuid, reason = {:?}", e))?;

    Ok(id)
}

#[derive(Serialize)]
pub struct ClassroomVersion {
    version: &'static str,
    event_room_id: Uuid,
    // TODO: this is deprecated and should be removed eventually
    // right now its necessary to generate HLS links
    stream_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    room_events_uri: Option<String>,
}

#[derive(Serialize)]
pub struct RealTimeObject {
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtc_id: Option<Uuid>,
}

impl RealTimeObject {
    pub fn set_rtc_id(&mut self, rtc_id: Uuid) {
        self.rtc_id = Some(rtc_id);
    }
}

pub mod authz;
pub mod chat;
pub mod minigroup;
pub mod p2p;
#[cfg(test)]
mod tests;
pub mod webinar;
