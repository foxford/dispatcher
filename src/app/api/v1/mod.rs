use std::sync::Arc;

use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use serde_derive::Deserialize;
use tide::{Request, Response};

use crate::app::AppContext;

use super::FEATURE_POLICY;

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

pub mod chat;
pub mod classroom;
#[cfg(test)]
mod tests;
pub mod webinar;
