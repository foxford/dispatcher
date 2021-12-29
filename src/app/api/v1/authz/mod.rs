use std::sync::Arc;
use std::{str::FromStr, time::Duration};

use anyhow::Context;
use axum::extract::{Extension, Json, Path};
use hyper::{Body, Response};
use serde_derive::{Deserialize, Serialize};
use serde_json::json;
use svc_authn::{AccountId, Authenticable};
use svc_utils::extractors::AuthnExtractor;
use tracing::info;
use uuid::Uuid;

use crate::app::{error, AppContext};
use crate::app::{error::ErrorExt, metrics::AuthMetrics};
use crate::{app::error::ErrorKind as AppErrorKind, utils::single_retry};

use crate::db::authz::{AuthzClass, AuthzReadQuery};

use super::{AppError, AppResult};

#[derive(Deserialize, Debug, Serialize)]
pub struct AuthzRequest {
    subject: Subject,
    object: Object,
    action: String,
}

#[derive(Deserialize, Debug, Serialize)]
struct Subject {
    namespace: String,
    value: SubjectValue,
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(untagged)]
enum SubjectValue {
    New(String),
    Old(Vec<String>),
}

#[derive(Deserialize, Debug, Serialize)]
struct Object {
    namespace: String,
    value: Vec<String>,
}

type Finder = Box<dyn FnOnce(&str) -> Result<AuthzReadQuery, anyhow::Error> + Send>;

pub async fn proxy(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(request_audience): Path<String>,
    AuthnExtractor(agent_id): AuthnExtractor,
    Json(mut authz_req): Json<AuthzRequest>,
) -> AppResult {
    let account_id = agent_id.as_account_id();

    validate_client(account_id, ctx.as_ref())?;

    let q = make_finder(account_id, request_audience.clone())?;

    info!("Authz proxy: raw request {:?}", authz_req);
    let old_action = authz_req.action.clone();

    transform_authz_request(&mut authz_req, account_id);

    substitute_class(&mut authz_req, ctx.as_ref(), q).await?;

    let http_proxy = ctx.authz().http_proxy(&request_audience);

    let retry_delay = ctx.config().retry_delay;
    let response = proxy_request(&authz_req, http_proxy, &old_action, retry_delay).await?;

    let response = Response::builder().body(Body::from(response)).unwrap();
    Ok(response)
}

const BUCKETS: [&str; 4] = ["hls.", "origin.", "ms.", "meta."];

fn validate_client(account_id: &AccountId, state: &dyn AppContext) -> Result<(), AppError> {
    let audience = state.agent_id().as_account_id().audience().to_owned();
    let account_audience = account_id.audience().split(':').next().unwrap();

    if account_audience != audience {
        Err(anyhow!("Not allowed")).error(AppErrorKind::Unauthorized)?;
    }

    Ok(())
}

fn make_finder(account_id: &AccountId, request_audience: String) -> Result<Finder, AppError> {
    let q = match account_id.label() {
        "event" => Box::new(|id: &str| {
            let id = Uuid::from_str(id)?;
            Ok(AuthzReadQuery::by_event(id))
        }) as Finder,
        "conference" => Box::new(|id: &str| {
            let id = Uuid::from_str(id)?;
            Ok(AuthzReadQuery::by_conference(id))
        }) as Finder,
        "storage" => Box::new(|id: &str| {
            if id.starts_with("content.") {
                match extract_audience_and_scope(id) {
                    Some(AudienceScope { audience, scope }) => {
                        Ok(AuthzReadQuery::by_scope(audience, scope))
                    }
                    None => Err(anyhow!("Access to set {:?} isnt proxied", id)),
                }
            } else if id.starts_with("eventsdump.") {
                match extract_event_room_id(id) {
                    Some(event_room_id) => {
                        let event_room_id = Uuid::from_str(event_room_id)?;
                        Ok(AuthzReadQuery::by_event(event_room_id))
                    }
                    None => Err(anyhow!("Access to bucket {:?} isnt proxied", id)),
                }
            } else if BUCKETS.iter().any(|prefix| id.starts_with(prefix)) {
                if id.contains("minigroup") {
                    extract_audience_and_scope(id)
                        .map(|AudienceScope { audience, scope }| {
                            AuthzReadQuery::by_scope(audience, scope)
                        })
                        .ok_or_else(|| anyhow!("Access to set {:?} isnt proxied", id))
                } else {
                    extract_rtc_id(id)
                        .ok_or_else(|| anyhow!("Access to set {:?} isnt proxied", id))
                        .and_then(|rtc_id| {
                            Uuid::from_str(rtc_id)
                                .map(AuthzReadQuery::by_rtc_id)
                                .map_err(|e| e.into())
                        })
                }
            } else {
                Err(anyhow!("Access to bucket {:?} isnt proxied", id))
            }
        }) as Finder,
        "nats-gatekeeper" => {
            Box::new(|id: &str| Ok(AuthzReadQuery::by_scope(request_audience, id.to_string())))
                as Finder
        }
        _ => Err(anyhow!("No finder")).error(AppErrorKind::Unauthorized)?,
    };

    Ok(q)
}

async fn proxy_request(
    authz_req: &AuthzRequest,
    http_proxy: Option<svc_authz::HttpProxy>,
    old_action: &str,
    retry_delay: Duration,
) -> Result<String, AppError> {
    let _timer = AuthMetrics::start_timer();
    if let Some(http_proxy) = http_proxy {
        let payload = serde_json::to_string(&authz_req)
            .context("Failed to serialize authz request")
            .error(AppErrorKind::SerializationFailed)?;
        let get_response = || async {
            let resp = http_proxy
                .send_async(payload.clone())
                .await
                .context("Authz proxied request failed")
                .error(AppErrorKind::AuthorizationFailed)?;

            let body = String::from_utf8(resp.to_vec())
                .map_err(|e| {
                    anyhow!(
                        "Authz proxied request body conversion to utf8 failed, err = {:?}",
                        e
                    )
                })
                .error(AppErrorKind::AuthorizationFailed)?;
            Ok::<_, error::Error>(body)
        };
        let body = single_retry(get_response, retry_delay).await?;

        info!(
            "Authz proxy: adjusted request {:?}, response = {}",
            authz_req, body
        );

        let json_body = match serde_json::from_str::<Vec<String>>(&body) {
            Ok(v) if v.contains(&authz_req.action) => json!([old_action]),
            Ok(_) => json!([]),
            Err(_) => {
                return Err(anyhow!("Invalid response format"))
                    .error(AppErrorKind::AuthorizationFailed);
            }
        };

        let body = serde_json::to_string(&json_body).unwrap();

        Ok(body)
    } else {
        Err(anyhow!("No proxy for non http authz backend")).error(AppErrorKind::AuthorizationFailed)
    }
}

fn transform_authz_request(authz_req: &mut AuthzRequest, account_id: &AccountId) {
    match account_id.label() {
        "event" => transform_event_authz_request(authz_req),
        "conference" => transform_conference_authz_request(authz_req),
        "storage" => transform_storage_authz_request(authz_req),
        "nats-gatekeeper" => transform_nats_gatekeeper_authz_request(authz_req),
        _ => {}
    }
}

fn transform_event_authz_request(authz_req: &mut AuthzRequest) {
    let act = &mut authz_req.action;

    // only transform rooms/* objects
    if authz_req.object.value.get(0).map(|s| s.as_ref()) != Some("rooms") {
        return;
    }

    // ["rooms", ROOM_ID, "agents"]::list       => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "events"]::list       => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "events"]::subscribe  => ["rooms", ROOM_ID]::read
    match authz_req.object.value.get_mut(0..) {
        None => {}
        Some([_rooms, _room_id, v]) if act == "list" && v == "agents" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([_rooms, _room_id, v]) if act == "list" && v == "events" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([_rooms, _room_id, v]) if act == "subscribe" && v == "events" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some(_) => {}
    }
}

fn transform_conference_authz_request(authz_req: &mut AuthzRequest) {
    let act = &mut authz_req.action;

    // only transform rooms/* objects
    if authz_req.object.value.get(0).map(|s| s.as_ref()) != Some("rooms") {
        return;
    }

    // ["rooms", ROOM_ID, "agents"]::list       => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "rtcs"]::list         => ["rooms", ROOM_ID]::read
    // ["rooms", ROOM_ID, "events"]::subscribe  => ["rooms", ROOM_ID]::read
    match authz_req.object.value.get_mut(0..) {
        None => {}
        Some([_rooms, _room_id, v]) if act == "list" && v == "agents" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([_rooms, _room_id, v]) if act == "list" && v == "rtcs" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some([_rooms, _room_id, v, _rtc_id]) if act == "read" && v == "rtcs" => {
            authz_req.object.value.truncate(2);
        }
        Some([_rooms, _room_id, v]) if act == "subscribe" && v == "events" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some(_) => {}
    }
}

fn transform_storage_authz_request(authz_req: &mut AuthzRequest) {
    let act = &mut authz_req.action;

    // only transform sets/* objects
    if authz_req.object.value.get(0).map(|s| s.as_ref()) != Some("sets") {
        return;
    }

    match authz_req.object.value.get_mut(0..) {
        None => {}
        // ["sets", "origin" <> _]         | *           | [CLASS_TYPE, CLASS_ID]                              | upload
        Some([_sets, v]) if v.starts_with("origin.") => {
            *act = "upload".into();
            authz_req.object.value.truncate(2);
        }
        // ["sets", "ms" <> _]             | *           | [CLASS_TYPE, CLASS_ID]                              | download
        Some([_sets, v]) if v.starts_with("ms.") => {
            *act = "download".into();
            authz_req.object.value.truncate(2);
        }
        // ["sets", "meta" <> _]           | read        | [CLASS_TYPE, CLASS_ID]                              | read
        // ["sets", "hls" <> _]            | read        | [CLASS_TYPE, CLASS_ID]                              | read
        // ["sets", "content" <> _]        | read        | [CLASS_TYPE, CLASS_ID]                              | read
        Some([_sets, v, _rtc_id])
            if act == "read"
                && (v.starts_with("meta.")
                    || v.starts_with("hls.")
                    || v.starts_with("content.")) =>
        {
            authz_req.object.value.truncate(2);
        }
        // ["sets", "content" <> _]        | create      | [CLASS_TYPE, CLASS_ID, content]                     | update
        // ["sets", "content" <> _]        | delete      | [CLASS_TYPE, CLASS_ID, content]                     | update
        // ["sets", "content" <> _]        | update      | [CLASS_TYPE, CLASS_ID, content]                     | update
        Some([_sets, v])
            if v.starts_with("content.")
                && (act == "create" || act == "delete" || act == "update") =>
        {
            *act = "update".into();
            authz_req.object.value.truncate(2);
            authz_req.object.value.push("content".into())
        }
        Some(_) => {}
    }
}

fn transform_nats_gatekeeper_authz_request(authz_req: &mut AuthzRequest) {
    let act = &mut authz_req.action;

    // only transform scopes/* objects
    if authz_req.object.value.get(0).map(|s| s.as_ref()) != Some("scopes") {
        return;
    }

    // ["scopes", SCOPE, "nats"]::connect       => ["scopes", SCOPE]::read
    match authz_req.object.value.get_mut(0..) {
        None => {}
        Some([_scopes, _scope, v]) if act == "connect" && v == "nats" => {
            *act = "read".into();
            authz_req.object.value.truncate(2);
        }
        Some(_) => {}
    }
}

async fn substitute_class(
    authz_req: &mut AuthzRequest,
    state: &dyn AppContext,
    q: impl FnOnce(&str) -> Result<AuthzReadQuery, anyhow::Error>,
) -> Result<(), AppError> {
    match authz_req.object.value.get_mut(0..2) {
        Some([ref mut obj, ref mut set_id]) if obj == "sets" => {
            let query = match q(set_id) {
                Ok(query) => query,
                Err(_e) => {
                    return Ok(());
                }
            };

            let mut conn = state
                .get_conn()
                .await
                .error(AppErrorKind::DbConnAcquisitionFailed)?;

            match query
                .execute(&mut conn)
                .await
                .context("Failed to find classroom")
                .error(AppErrorKind::DbQueryFailed)?
            {
                None => Ok(()),
                Some(AuthzClass { id }) => {
                    *obj = "classrooms".into();
                    *set_id = id;
                    authz_req.object.namespace = state.agent_id().as_account_id().to_string();

                    Ok(())
                }
            }
        }
        Some([ref mut obj, ref mut room_id]) if obj == "rooms" => {
            let query = match q(room_id) {
                Ok(query) => query,
                Err(_) => {
                    return Ok(());
                }
            };

            let mut conn = state
                .get_conn()
                .await
                .error(AppErrorKind::DbConnAcquisitionFailed)?;

            match query
                .execute(&mut conn)
                .await
                .context("Failed to find classroom")
                .error(AppErrorKind::DbQueryFailed)?
            {
                None => Ok(()),
                Some(AuthzClass { id }) => {
                    *obj = "classrooms".into();
                    *room_id = id;
                    authz_req.object.namespace = state.agent_id().as_account_id().to_string();

                    Ok(())
                }
            }
        }
        Some([obj, ..]) if obj == "classrooms" => {
            authz_req.object.namespace = state.agent_id().as_account_id().to_string();
            Ok(())
        }
        Some([ref mut obj, ref mut scope]) if obj == "scopes" => {
            let query = match q(scope) {
                Ok(query) => query,
                Err(_e) => {
                    return Ok(());
                }
            };

            let mut conn = state
                .get_conn()
                .await
                .error(AppErrorKind::DbConnAcquisitionFailed)?;

            match query
                .execute(&mut conn)
                .await
                .context("Failed to find classroom")
                .error(AppErrorKind::DbQueryFailed)?
            {
                None => Ok(()),
                Some(AuthzClass { id }) => {
                    *obj = "classrooms".into();
                    *scope = id;
                    authz_req.object.namespace = state.agent_id().as_account_id().to_string();

                    Ok(())
                }
            }
        }
        _ => Ok(()),
    }
}

struct AudienceScope {
    pub audience: String,
    pub scope: String,
}

fn extract_audience_and_scope(set_id: &str) -> Option<AudienceScope> {
    set_id.find("::").and_then(|idx| {
        let bucket = &set_id[..idx];
        let bucket_split = bucket.split('.').collect::<Vec<&str>>();
        bucket_split.get(2..).map(|v| {
            let audience = v.join(".");
            let scope = set_id[idx + 2..].to_owned();

            AudienceScope { audience, scope }
        })
    })
}

fn extract_rtc_id(set_id: &str) -> Option<&str> {
    set_id.find("::").and_then(|idx| set_id.get(idx + 2..))
}

fn extract_event_room_id(set_id: &str) -> Option<&str> {
    set_id.find("::").and_then(|idx| set_id.get(idx + 2..))
}

#[test]
fn test_extract_audience_and_scope() {
    let r = extract_audience_and_scope("content.webinar.testing01.foxford.ru::p2p_48wmpa")
        .expect("Failed to extract audience and scope");
    assert_eq!(r.audience, "testing01.foxford.ru");
    assert_eq!(r.scope, "p2p_48wmpa");
}

#[test]
fn test_extract_rtc_id() {
    let r = extract_rtc_id("ms.webinar.testing01.foxford.ru::14aa9730-26e1-487c-9153-bc8cb28d8eb0")
        .expect("Failed to extract rtc_id");
    assert_eq!(r, "14aa9730-26e1-487c-9153-bc8cb28d8eb0");
}
