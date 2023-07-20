use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, Postgres};

#[derive(Clone)]
pub struct TestDb {
    pool: PgPool,
}

impl TestDb {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get_conn(&self) -> PoolConnection<Postgres> {
        self.pool
            .acquire()
            .await
            .expect("Failed to get DB connection")
    }
}
