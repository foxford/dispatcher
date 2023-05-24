use std::sync::Arc;

use axum::{extract::Path, response::Response, Extension, Json};
use hyper::Body;
use serde_derive::Deserialize;
use svc_authn::AccountId;
use uuid::Uuid;

use svc_utils::extractors::AccountIdExtractor;

use crate::{
    app::{
        api::v1::find_class,
        error::{ErrorExt, ErrorKind as AppErrorKind},
        metrics::AuthorizeMetrics,
        stage, AppContext, AuthzObject,
    },
    db::ban_account_op,
};

use super::AppResult;

#[derive(Deserialize)]
pub struct BanPayload {
    // TODO: maybe Option?
    last_seen_op_id: Uuid,
    ban: bool,
    class_id: Uuid,
}

pub async fn ban(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(account_to_ban): Path<AccountId>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Json(payload): Json<BanPayload>,
) -> AppResult {
    let class = find_class(ctx.as_ref(), payload.class_id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();
    ctx.authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;

    let last_ban_account_op = ban_account_op::ReadQuery::by_id(&account_to_ban)
        .execute(&mut conn)
        .await
        .error(AppErrorKind::DbQueryFailed)?;

    if let Some(op) = last_ban_account_op {
        if op.last_op_id != payload.last_seen_op_id {
            return Err(AppErrorKind::OperationIdObsolete.into());
        }

        if !op.complete() {
            return Err(AppErrorKind::OperationInProgress.into());
        }
    }

    stage::ban_intent::start(
        ctx.as_ref(),
        &mut conn,
        payload.ban,
        &class,
        account_to_ban,
        payload.last_seen_op_id,
    )
    .await?;

    Ok(Response::builder().status(200).body(Body::empty()).unwrap())
}
