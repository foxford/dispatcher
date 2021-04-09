use std::env::var;

use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, PgPoolOptions, Postgres};

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
