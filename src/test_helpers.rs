use std::env::var;

use anyhow::Result;
use async_trait::async_trait;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, PgPoolOptions, Postgres};
use svc_agent::mqtt::Agent;
use svc_authn::Error;
use tide::http::url::Url;

use crate::app::AppContext;
use crate::config::Config;

#[derive(Clone)]
pub struct TestDb {
    pool: PgPool,
}

impl TestDb {
    pub async fn new() -> Self {
        let url = var("DATABASE_URL").expect("DATABASE_URL must be specified");

        let pool = PgPoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .connect(&url)
            .await
            .expect("Failed to connect to the DB");

        Self { pool }
    }

    pub async fn get_conn(&self) -> PoolConnection<Postgres> {
        self.pool
            .acquire()
            .await
            .expect("Failed to get DB connection")
    }
}

#[derive(Clone)]
pub struct TestState {
    db_pool: TestDb,
    config: Config,
}

impl TestState {
    pub async fn new() -> Self {
        let config = crate::config::load().expect("Failed to load config");

        let db_pool = TestDb::new().await;
        Self { db_pool, config }
    }
}

#[async_trait]
impl AppContext for TestState {
    async fn get_conn(&self) -> Result<PoolConnection<Postgres>> {
        let conn = self.db_pool.get_conn().await;

        Ok(conn)
    }

    fn default_frontend_base(&self) -> Url {
        self.config.default_frontend_base.clone()
    }

    fn validate_token(&self, _token: Option<&str>) -> Result<(), Error> {
        Ok(())
    }

    fn agent(&self) -> Option<Agent> {
        None
    }
}

pub fn random_string() -> String {
    use rand::distributions::Alphanumeric;
    use rand::{thread_rng, Rng};

    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(30)
        .map(char::from)
        .collect()
}
