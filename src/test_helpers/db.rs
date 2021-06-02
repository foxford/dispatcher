use std::env::var;

use sqlx::postgres::{PgPool, PgPoolOptions, Postgres};
use sqlx::{pool::PoolConnection, Executor};

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

        // todo: we should actually run every test in transaction, but thats not possible for now, maybe in sqlx 0.6
        {
            let mut conn = pool.acquire().await.expect("Failed to get DB connection");

            conn.execute("TRUNCATE class CASCADE;")
                .await
                .expect("Failed to truncate class table");
        }
        Self { pool }
    }

    pub async fn get_conn(&self) -> PoolConnection<Postgres> {
        self.pool
            .acquire()
            .await
            .expect("Failed to get DB connection")
    }
}
