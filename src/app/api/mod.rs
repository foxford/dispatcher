use std::sync::Arc;

use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde_derive::Deserialize;
use svc_agent::{
    mqtt::{
        IntoPublishableMessage, OutgoingEvent, OutgoingEventProperties, ShortTermTimingProperties,
    },
    Authenticable,
};
use tide::http::url::Url;
use tide::{Request, Response};

use crate::app::authz::AuthzObject;
use crate::app::AppContext;

use super::metrics::AuthorizeMetrics;

const FEATURE_POLICY: &str = "autoplay *; camera *; microphone *; display-capture *; fullscreen *";

#[derive(Deserialize)]
struct RedirQuery {
    pub scope: String,
}

pub async fn redirect_to_frontend(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let (tenant, app) = (req.param("tenant"), req.param("app"));

    match (tenant, app) {
        (Err(e), _) => {
            error!(crate::LOG, "No tenant specified: {}", e);
            Ok(tide::Response::builder(404).build())
        }
        (_, Err(e)) => {
            error!(crate::LOG, "No app specified: {}", e);
            Ok(tide::Response::builder(404).build())
        }
        (Ok(tenant), Ok(app)) => {
            let base_url = match req.query::<RedirQuery>() {
                Err(e) => {
                    error!(crate::LOG, "Failed to parse query: {}", e);
                    None
                }
                Ok(query) => {
                    let conn = req.state().get_conn().await;
                    match conn {
                        Err(e) => {
                            error!(crate::LOG, "Failed to acquire conn: {}", e);
                            None
                        }
                        Ok(mut conn) => {
                            let fe = crate::db::frontend::FrontendByScopeQuery::new(
                                query.scope,
                                app.to_owned(),
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
                    }
                }
            };

            let mut url = base_url.unwrap_or_else(|| {
                build_default_url(req.state().default_frontend_base(), tenant, app)
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
    }
}

pub async fn rollback(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let scope = req.param("scope")?.to_owned();

    let maybe_token = req
        .header("Authorization")
        .and_then(|h| h.get(0))
        .map(|header| header.to_string());

    let state = req.state();
    match state.validate_token(maybe_token.as_deref()) {
        Ok(account_id) => {
            let object = AuthzObject::new(&["scopes"]).into();

            if let Err(err) = state
                .authz()
                .authorize(
                    state.agent_id().as_account_id().audience().to_string(),
                    account_id.clone(),
                    object,
                    "rollback".into(),
                )
                .await
                .measure()
            {
                error!(crate::LOG, "Failed to authorize action, reason = {:?}", err);
                return Ok(tide::Response::builder(403).body("Access denied").build());
            }

            match state.get_conn().await {
                Err(err) => {
                    error!(crate::LOG, "Failed to get db conn, reason = {:?}", err);

                    return Ok(tide::Response::builder(500)
                        .body(format!("Failed to acquire conn: {}", err))
                        .build());
                }
                Ok(mut conn) => {
                    let r = crate::db::scope::DeleteQuery::new(scope.clone())
                        .execute(&mut conn)
                        .await;

                    if let Err(err) = r {
                        error!(
                            crate::LOG,
                            "Failed to delete scope from db, reason = {:?}", err
                        );

                        return Ok(tide::Response::builder(500)
                            .body(format!("Failed to delete scope: {}", err))
                            .build());
                    }

                    let timing = ShortTermTimingProperties::new(chrono::Utc::now());
                    let props = OutgoingEventProperties::new("scope.frontend.rollback", timing);
                    let path = format!("scopes/{}/events", scope);
                    let event = OutgoingEvent::broadcast("", props, &path);
                    let e = Box::new(event) as Box<dyn IntoPublishableMessage + Send>;

                    if let Err(err) = state.publisher().publish(e) {
                        error!(
                            crate::LOG,
                            "Failed to publish scope.frontend.rollback event, reason = {:?}", err
                        );
                    }
                }
            }
        }
        Err(e) => {
            error!(
                crate::LOG,
                "Failed to process Authorization header, header = {:?}, err = {:?}",
                req.header("Authorization"),
                e
            );
            return Ok(tide::Response::builder(403).body("Access denied").build());
        }
    }

    Ok("Ok".into())
}

fn build_default_url(mut url: Url, tenant: &str, app: &str) -> Url {
    let host = url.host_str().map(|h| format!("{}.{}.{}", tenant, app, h));
    if let Err(e) = url.set_host(host.as_deref()) {
        error!(crate::LOG, "Default url set_host failed, reason = {:?}", e);
    }
    url
}

pub mod v1;
