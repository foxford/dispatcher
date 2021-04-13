use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context;
use futures::AsyncReadExt;
use serde_derive::{Deserialize, Serialize};
use svc_authn::{AccountId, Authenticable};
use tide::{Request, Response};
use uuid::Uuid;

use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;

use crate::db::authz::{AuthzClass, AuthzReadQuery};

use super::{extract_param, validate_token, AppError, AppResult};

#[derive(Deserialize, Debug, Serialize)]
struct AuthzRequest {
    subject: Subject,
    object: Object,
    action: String,
}

#[derive(Deserialize, Debug, Serialize)]
struct Subject {
    namespace: String,
    value: String,
}

#[derive(Deserialize, Debug, Serialize)]
struct Object {
    namespace: String,
    value: Vec<String>,
}

pub async fn proxy(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    proxy_inner(req).await.or_else(|e| Ok(e.to_tide_response()))
}
async fn proxy_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let request_audience = extract_param(&req, "audience")
        .error(AppErrorKind::InvalidPayload)?
        .to_owned();

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;

    let q = validate_client(&account_id, req.state().as_ref())?;

    let mut authz_req: AuthzRequest = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    info!(crate::LOG, "Authz proxy: raw request {:?}", authz_req);
    let old_action = authz_req.action.clone();

    transform_authz_request(&mut authz_req, &account_id);

    substitute_class(&mut authz_req, req.state().as_ref(), q).await?;

    let http_proxy = req.state().authz().http_proxy(&request_audience);

    let response = proxy_request(&authz_req, http_proxy, &old_action).await?;

    let response = Response::builder(200).body(response).build();
    Ok(response)
}

fn validate_client(
    account_id: &AccountId,
    state: &dyn AppContext,
) -> Result<impl Fn(Uuid) -> AuthzReadQuery, AppError> {
    let audience = state.agent_id().as_account_id().audience().to_owned();

    let account_audience = account_id.audience().split(':').next().unwrap();
    let q = match account_id.label() {
        "event" if account_audience == audience => |id| AuthzReadQuery::by_event(id),
        "conference" if account_audience == audience => |id| AuthzReadQuery::by_conference(id),
        _ => Err(anyhow!("Not allowed")).error(AppErrorKind::Unauthorized)?,
    };

    Ok(q)
}

async fn proxy_request(
    authz_req: &AuthzRequest,
    http_proxy: Option<svc_authz::HttpProxy>,
    old_action: &str,
) -> Result<String, AppError> {
    if let Some(http_proxy) = http_proxy {
        let payload = serde_json::to_string(&authz_req)
            .context("Failed to serialize authz request")
            .error(AppErrorKind::SerializationFailed)?;

        let mut resp = http_proxy
            .send_async(payload)
            .await
            .context("Authz proxied request failed")
            .error(AppErrorKind::AuthorizationFailed)?;

        let mut body = String::new();
        resp.body_mut()
            .read_to_string(&mut body)
            .await
            .context("Authz proxied request body fetch failed")
            .error(AppErrorKind::AuthorizationFailed)?;

        info!(
            crate::LOG,
            "Authz proxy: adjusted request {:?}, response = {}", authz_req, body
        );

        let body = match serde_json::from_str::<Vec<String>>(&body) {
            Ok(v) if v.contains(&authz_req.action) => serde_json::to_string(&[old_action]).unwrap(),
            Ok(_) => {
                return Err(anyhow!("Not allowed")).error(AppErrorKind::AuthorizationFailed);
            }
            Err(_) => {
                return Err(anyhow!("Invalid response format"))
                    .error(AppErrorKind::AuthorizationFailed);
            }
        };

        Ok(body)
    } else {
        return Err(anyhow!("No proxy for non http authz backend"))
            .error(AppErrorKind::AuthorizationFailed);
    }
}

fn transform_authz_request(authz_req: &mut AuthzRequest, account_id: &AccountId) {
    match account_id.label() {
        "event" => transform_event_authz_request(authz_req),
        "conference" => transform_conference_authz_request(authz_req),
        _ => {}
    }
}

fn transform_event_authz_request(authz_req: &mut AuthzRequest) {
    let act = &mut authz_req.action;

    // ["rooms", ROOM_ID, "agents"]::list       => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "events"]::list       => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "events"]::subscribe  => ["rooms", ROOM_ID]::read
    match authz_req.object.value.get_mut(0..) {
        None => {}
        Some([obj, _, v]) if act == "list" && obj == "rooms" && v == "agents" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([obj, _, v]) if act == "list" && obj == "rooms" && v == "events" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([obj, _, v]) if act == "subscribe" && obj == "rooms" && v == "events" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some(_) => {}
    }
}

fn transform_conference_authz_request(authz_req: &mut AuthzRequest) {
    let act = &mut authz_req.action;

    // ["rooms", ROOM_ID, "agents"]::list       => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "rtcs"]::list         => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "events"]::subscribe  => ["rooms", ROOM_ID]::read
    match authz_req.object.value.get_mut(0..) {
        None => {}
        Some([obj, _, v]) if act == "list" && obj == "rooms" && v == "agents" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([obj, _, v]) if act == "list" && obj == "rooms" && v == "rtcs" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([obj, _, v, _rtc_id]) if act == "read" && obj == "rooms" && v == "rtcs" => {
            authz_req.object.value.truncate(2);
        }
        Some([obj, _, v]) if act == "subscribe" && obj == "rooms" && v == "events" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some(_) => {}
    }
}

async fn substitute_class(
    authz_req: &mut AuthzRequest,
    state: &dyn AppContext,
    q: impl Fn(Uuid) -> AuthzReadQuery,
) -> Result<(), AppError> {
    match authz_req.object.value.get_mut(0..2) {
        Some([ref mut obj, ref mut room_id]) if obj == "rooms" => match Uuid::from_str(room_id) {
            Err(_) => Ok(()),
            Ok(id) => {
                let mut conn = state
                    .get_conn()
                    .await
                    .error(AppErrorKind::DbConnAcquisitionFailed)?;
                match q(id)
                    .execute(&mut conn)
                    .await
                    .context("Failed to insert chat")
                    .error(AppErrorKind::DbQueryFailed)?
                {
                    None => Ok(()),
                    Some(AuthzClass { kind, id }) => {
                        *obj = kind;
                        *room_id = id;
                        authz_req.object.namespace = state.agent_id().as_account_id().to_string();

                        Ok(())
                    }
                }
            }
        },
        _ => Ok(()),
    }
}
