use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::postgres::PgPool;

use crate::config::{self};
use api::{
    redirect_to_frontend, rollback, v1::healthz, v1::redirect_to_frontend as redirect_to_frontend2,
    v1::webinar::create as create_webinar, v1::webinar::read as read_webinar,
};
use info::{list_frontends, list_scopes};
pub use tide_state::{AppContext, TideState};

pub async fn run(db: PgPool) -> Result<()> {
    let config = config::load().context("Failed to load config")?;
    info!(crate::LOG, "App config: {:?}", config);
    tide::log::start();

    let state = TideState::new(db, config.clone())?;
    let state = Arc::new(state) as Arc<dyn AppContext>;
    let mut app = tide::with_state(state);
    app.at("/info/scopes").get(list_scopes);
    app.at("/info/frontends").get(list_frontends);
    app.at("/redirs/tenants/:tenant/apps/:app")
        .get(redirect_to_frontend);
    app.at("/api/scopes/:scope/rollback").post(rollback);

    app.at("/api/v1/healthz").get(healthz);
    app.at("/api/v1/scopes/:scope/rollback").post(rollback);
    app.at("/api/v1/redirs").get(redirect_to_frontend2);

    app.at("/api/v1/webinars/:id").get(read_webinar);
    app.at("/api/v1/webinars").post(create_webinar);
    app.listen(config.http.listener_address).await?;
    Ok(())
}

mod api;
mod info;
mod tide_state;
