use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

pub(crate) async fn create_pool(
    url: &str,
    size: u32,
    idle_size: Option<u32>,
    timeout: u64,
    max_lifetime: u64,
) -> PgPool {
    PgPoolOptions::new()
        .max_connections(size)
        .min_connections(idle_size.unwrap_or(1))
        .acquire_timeout(Duration::from_secs(timeout))
        .max_lifetime(Duration::from_secs(max_lifetime))
        .connect(url)
        .await
        .expect("Failed to create sqlx database pool")
}

pub(crate) mod account;
pub(crate) mod authz;
pub(crate) mod class;
pub(crate) mod frontend;
pub(crate) mod record_timestamp;
pub(crate) mod recording;
pub(crate) mod scope;
