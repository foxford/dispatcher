use slog::{error, info, warn};
use tide::{Middleware, Next, Request};

use crate::LOG;

#[derive(Debug, Default, Clone)]
pub struct LogMiddleware {}

impl LogMiddleware {
    pub fn new() -> Self {
        Self {}
    }

    /// Log a request and a response.
    async fn log<'a, State: Clone + Send + Sync + 'static>(
        &'a self,
        req: Request<State>,
        next: Next<'a, State>,
    ) -> tide::Result {
        let path = req.url().path().to_owned();
        let method = req.method().to_string();
        let start = std::time::Instant::now();
        let response = next.run(req).await;
        let status = response.status();
        if status.is_server_error() {
            if let Some(error) = response.error() {
                error!(LOG, "HTTP response";
                    "message" => format!("{:?}", error),
                    "error_type" => error.type_name(),
                    "method" => method,
                    "path" => path,
                    "status" => status as u16,
                    "duration" => format!("{:?}", start.elapsed()),
                );
            } else {
                error!(LOG, "HTTP response";
                    "method" => method,
                    "path" => path,
                    "status" => status as u16,
                    "duration" => format!("{:?}", start.elapsed()),
                );
            }
        } else if status.is_client_error() {
            if let Some(error) = response.error() {
                warn!(LOG, "HTTP response";
                    "message" => format!("{:?}", error),
                    "error_type" => error.type_name(),
                    "method" => method,
                    "path" => path,
                    "status" => status as u16,
                    "duration" => format!("{:?}", start.elapsed()),
                );
            } else {
                warn!(LOG, "HTTP response";
                    "method" => method,
                    "path" => path,
                    "status" => status as u16,
                    "duration" => format!("{:?}", start.elapsed()),
                );
            }
        } else {
            info!(LOG, "HTTP response";
                "method" => method,
                "path" => path,
                "status" => status as u16,
                "duration" => format!("{:?}", start.elapsed()),
            );
        }
        Ok(response)
    }
}

#[async_trait::async_trait]
impl<State: Clone + Send + Sync + 'static> Middleware<State> for LogMiddleware {
    async fn handle(&self, req: Request<State>, next: Next<'_, State>) -> tide::Result {
        self.log(req, next).await
    }
}
