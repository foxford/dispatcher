use std::sync::Arc;

use axum::{extract::Path, response::Response, Extension, Json};
use hyper::Body;
use serde_derive::{Deserialize, Serialize};
use svc_authn::{AccountId, Authenticable};
use uuid::Uuid;

use svc_utils::extractors::AgentIdExtractor;

use crate::{
    app::{
        api::{v1::find_class, IntoJsonResponse},
        error::{ErrorExt, ErrorKind as AppErrorKind},
        metrics::AuthorizeMetrics,
        stage, AppContext, AuthzObject,
    },
    db::ban_account_op,
};

use super::AppResult;

#[derive(Deserialize)]
pub struct BanPayload {
    last_seen_op_id: i64,
    ban: bool,
    class_id: Uuid,
}

pub async fn ban(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(account_to_ban): Path<AccountId>,
    AgentIdExtractor(agent_id): AgentIdExtractor,
    Json(payload): Json<BanPayload>,
) -> AppResult {
    let class = find_class(ctx.as_ref(), payload.class_id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();
    ctx.authz()
        .authorize(
            class.audience().to_owned(),
            agent_id.as_account_id().clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;

    let last_ban_account_op = ban_account_op::ReadQuery::by_account_id(&account_to_ban)
        .execute(&mut conn)
        .await
        .error(AppErrorKind::DbQueryFailed)?;

    if let Some(op) = last_ban_account_op {
        if op.last_op_id != payload.last_seen_op_id {
            return Err(AppErrorKind::OperationIdObsolete.into());
        }

        if !op.is_completed() {
            return Err(AppErrorKind::OperationInProgress.into());
        }
    }

    stage::ban::save_intent(
        ctx.as_ref(),
        &mut conn,
        payload.ban,
        &class,
        agent_id,
        account_to_ban,
        payload.last_seen_op_id,
    )
    .await?;

    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}

#[derive(Serialize)]
pub struct GetLastBanOperationResponse {
    last_seen_op_id: i64,
}

pub async fn get_last_ban_operation(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(account_to_ban): Path<AccountId>,
) -> AppResult {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;

    let last_ban_account_op = ban_account_op::ReadQuery::by_account_id(&account_to_ban)
        .execute(&mut conn)
        .await
        .error(AppErrorKind::DbQueryFailed)?;

    let r = GetLastBanOperationResponse {
        last_seen_op_id: last_ban_account_op.map(|l| l.last_op_id).unwrap_or(0),
    };

    r.into_json_response(
        "failed to serialize last ban operation",
        http::StatusCode::OK,
    )
}
