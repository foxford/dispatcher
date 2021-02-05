use std::fmt::Write;
use std::sync::Arc;

use tide::Request;

use super::AppContext;

pub async fn list_scopes(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    match req.state().get_conn().await {
        Err(e) => Ok(tide::Response::builder(500)
            .body(format!("Failed to acquire conn: {}", e))
            .build()),
        Ok(mut conn) => {
            let scopes = crate::db::scope::ListQuery::new().execute(&mut conn).await;
            match scopes {
                Err(e) => Ok(format!("Failed query: {}", e).into()),
                Ok(scopes) => {
                    let mut s = String::new();
                    writeln!(&mut s, "Frontends list:")?;
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
                    Ok(s.into())
                }
            }
        }
    }
}

pub async fn list_frontends(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    match req.state().get_conn().await {
        Err(e) => Ok(tide::Response::builder(500)
            .body(format!("Failed to acquire conn: {}", e))
            .build()),
        Ok(mut conn) => {
            let frontends = crate::db::frontend::ListQuery::new()
                .execute(&mut conn)
                .await;
            match frontends {
                Err(e) => Ok(format!("Failed query: {}", e).into()),
                Ok(frontends) => {
                    let mut s = String::new();
                    writeln!(&mut s, "Frontends list:")?;
                    for fe in frontends {
                        if let Err(e) = writeln!(&mut s, "{}\t{}\t{}", fe.id, fe.url, fe.created_at)
                        {
                            error!(
                                crate::LOG,
                                "Failed to write response to buf string, reason = {:?}", e
                            );
                        }
                    }
                    Ok(s.into())
                }
            }
        }
    }
}
