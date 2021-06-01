use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgPool, PgPoolOptions, Postgres};
use testcontainers::{
    clients,
    images::{self, generic::WaitFor},
    Container, Docker,
};

pub struct PostgresHandle<'a> {
    connection_string: String,
    _container: Container<'a, clients::Cli, images::generic::GenericImage>,
}

pub struct LocalPostgres {
    docker: clients::Cli,
}

impl LocalPostgres {
    pub fn new() -> Self {
        Self {
            docker: clients::Cli::default(),
        }
    }

    pub fn run(&self) -> PostgresHandle {
        let image = images::generic::GenericImage::new("postgres")
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_DB", "dispatcher")
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres");
        let node = self.docker.run(image);
        let connection_string = format!(
            "postgres://postgres:postgres@localhost:{}/dispatcher",
            node.get_host_port(5432).unwrap(),
        );
        PostgresHandle {
            connection_string,
            _container: node,
        }
    }
}

#[derive(Clone)]
pub struct TestDb {
    pool: PgPool,
}

impl TestDb {
    pub async fn new_with_local_postgres(handle: &PostgresHandle<'_>) -> Self {
        let pool = PgPoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .connect(&handle.connection_string)
            .await
            .expect("Failed to connect to the DB");
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        Self { pool }
    }

    pub async fn get_conn(&self) -> PoolConnection<Postgres> {
        self.pool
            .acquire()
            .await
            .expect("Failed to get DB connection")
    }
}
