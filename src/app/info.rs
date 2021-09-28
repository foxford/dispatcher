use std::fmt::Write;
use std::sync::Arc;

use axum::extract;
use hyper::{Body, Response};

use super::AppContext;

pub async fn list_scopes(ctx: extract::Extension<Arc<dyn AppContext>>) -> Response<Body> {
    match ctx.get_conn().await {
        Err(e) => Response::builder()
            .status(500)
            .body(Body::from(format!("Failed to acquire conn: {}", e)))
            .unwrap(),
        Ok(mut conn) => {
            let scopes = crate::db::scope::ListQuery::new().execute(&mut conn).await;
            match scopes {
                Err(e) => Response::builder()
                    .body(Body::from(format!("Failed query: {}", e)))
                    .unwrap(),
                Ok(scopes) => {
                    let mut s = String::new();
                    writeln!(&mut s, "Frontends list:").unwrap();
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
                    Response::builder().body(Body::from(s)).unwrap()
                }
            }
        }
    }
}

pub async fn list_frontends(ctx: extract::Extension<Arc<dyn AppContext>>) -> Response<Body> {
    match ctx.get_conn().await {
        Err(e) => Response::builder()
            .status(500)
            .body(Body::from(format!("Failed to acquire conn: {}", e)))
            .unwrap(),
        Ok(mut conn) => {
            let frontends = crate::db::frontend::ListQuery::new()
                .execute(&mut conn)
                .await;
            match frontends {
                Err(e) => Response::builder()
                    .status(500)
                    .body(Body::from(format!("Failed query: {}", e)))
                    .unwrap(),
                Ok(frontends) => {
                    let mut s = String::new();
                    writeln!(&mut s, "Frontends list:").unwrap();
                    for fe in frontends {
                        if let Err(e) = writeln!(&mut s, "{}\t{}\t{}", fe.id, fe.url, fe.created_at)
                        {
                            error!(
                                crate::LOG,
                                "Failed to write response to buf string, reason = {:?}", e
                            );
                        }
                    }
                    Response::builder().body(Body::from(s)).unwrap()
                }
            }
        }
    }
}
