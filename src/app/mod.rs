use std::fmt::Write;

use anyhow::{Context, Result};
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde_derive::Deserialize;
use sqlx::postgres::PgPool;
use svc_agent::mqtt::{
    IntoPublishableMessage, OutgoingEvent, OutgoingEventProperties, ShortTermTimingProperties,
};
use tide::http::url::Url;
use tide::{Request, Response};

use crate::config::{self};
use tide_state::TideState;

const FEATURE_POLICY: &str = "autoplay *; camera *; microphone *; display-capture *; fullscreen *";

pub async fn run(db: PgPool) -> Result<()> {
    let config = config::load().context("Failed to load config")?;
    info!(crate::LOG, "App config: {:?}", config);
    tide::log::start();

    let state = TideState::new(db, config.clone())?;
    let mut app = tide::with_state(state);
    app.at("/scopes").get(list_scopes);
    app.at("/frontends").get(list_frontends);
    app.at("/redirs/tenants/:tenant/apps/:app")
        .get(redirect_to_frontend);
    app.at("/healthz").get(healthz);
    app.at("/api/scopes/:scope/rollback").post(rollback);
    app.listen(config.http.listener_address).await?;
    Ok(())
}

async fn healthz(_req: Request<TideState>) -> tide::Result {
    Ok("Ok".into())
}
#[derive(Deserialize)]
struct RedirQuery {
    pub scope: String,
}

async fn redirect_to_frontend(req: Request<TideState>) -> tide::Result {
    let (tenant, app) = (req.param("tenant"), req.param("app"));

    match (tenant, app) {
        (Err(e), _) => {
            error!(crate::LOG, "No tenant specified: {}", e);
            return Ok(tide::Response::builder(404).build());
        }
        (_, Err(e)) => {
            error!(crate::LOG, "No app specified: {}", e);
            return Ok(tide::Response::builder(404).build());
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

async fn list_scopes(req: Request<TideState>) -> tide::Result {
    match req.state().get_conn().await {
        Err(e) => Ok(tide::Response::builder(500)
            .body(format!("Failed to acquire conn: {}", e))
            .build()),
        Ok(mut conn) => {
            let scopes = crate::db::scope::ListQuery::new().execute(&mut conn).await;
            match scopes {
                Err(e) => Ok(format!("Failed query: {}", e).into()),
                Ok(scopes) => {
                    let mut s = String::new();
                    writeln!(&mut s, "Frontends list:")?;
                    for scope in scopes {
                        if let Err(e) = writeln!(
                            &mut s,
                            "{}\t{}\t{}\t{}",
                            scope.id, scope.scope, scope.frontend_id, scope.created_at
                        ) {
                            error!(
                                crate::LOG,
                                "Failed to write response to buf string, reason = {:?}", e
                            );
                        }
                    }
                    Ok(s.into())
                }
            }
        }
    }
}

async fn list_frontends(req: Request<TideState>) -> tide::Result {
    match req.state().get_conn().await {
        Err(e) => Ok(tide::Response::builder(500)
            .body(format!("Failed to acquire conn: {}", e))
            .build()),
        Ok(mut conn) => {
            let frontends = crate::db::frontend::ListQuery::new()
                .execute(&mut conn)
                .await;
            match frontends {
                Err(e) => Ok(format!("Failed query: {}", e).into()),
                Ok(frontends) => {
                    let mut s = String::new();
                    writeln!(&mut s, "Frontends list:")?;
                    for fe in frontends {
                        if let Err(e) = writeln!(&mut s, "{}\t{}\t{}", fe.id, fe.url, fe.created_at)
                        {
                            error!(
                                crate::LOG,
                                "Failed to write response to buf string, reason = {:?}", e
                            );
                        }
                    }
                    Ok(s.into())
                }
            }
        }
    }
}

async fn rollback(req: Request<TideState>) -> tide::Result {
    let scope = req.param("scope")?.to_owned();

    match req
        .header("Authorization")
        .and_then(|h| h.get(0))
        .map(|header| header.to_string())
        .map(|h| {
            eprintln!("H: {:?}", h);
            h
        }) {
        Some(token) if token.starts_with("Bearer ") => {
            let token = token.replace("Bearer ", "");
            if let Err(err) = req.state().validate_token(&token) {
                error!(crate::LOG, "Invalid token, reason = {:?}", err);

                return Ok(tide::Response::builder(403).body("Access denied").build());
            }
            match req.state().get_conn().await {
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

                    if let Err(err) = req.state().clone_agent().publish_publishable(e) {
                        error!(
                            crate::LOG,
                            "Failed to publish rollback event, reason = {:?}", err
                        );
                    }
                }
            }
        }
        _ => {
            error!(
                crate::LOG,
                "Failed to process Authorization header, header = {:?}",
                req.header("Authorization")
            );
            return Ok(tide::Response::builder(403).body("Access denied").build());
        }
    }
    Ok("Ok".into())
}

fn build_default_url(mut url: Url, tenant: &str, app: &str) -> Url {
    let host = url.host_str().map(|h| format!("{}.{}.{}", tenant, app, h));
    if let Err(e) = url.set_host(host.as_deref()) {
        warn!(crate::LOG, "Default url set_host failed, reason = {:?}", e);
    }
    url
}

mod tide_state;
