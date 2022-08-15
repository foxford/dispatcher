use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Path};
use hyper::{Body, Response};
use svc_authn::AccountId;
use svc_utils::extractors::AccountIdExtractor;
use tracing::error;
use uuid::Uuid;

use super::*;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};

pub async fn commit_edition(
    ctx: Extension<Arc<dyn AppContext>>,
    Path((audience, scope, edition_id)): Path<(String, String, Uuid)>,
    AccountIdExtractor(account_id): AccountIdExtractor,
) -> AppResult {
    do_commit_edition(ctx.0.as_ref(), &account_id, &audience, &scope, edition_id).await
}

async fn do_commit_edition(
    state: &dyn AppContext,
    account_id: &AccountId,
    audience: &str,
    scope: &str,
    edition_id: Uuid,
) -> AppResult {
    let class = match find_class_by_scope(state, audience, scope).await {
        Ok(class) => class,
        Err(e) => {
            error!("Failed to find class, err = {:?}", e);
            return Ok(Response::builder()
                .status(404)
                .body(Body::from("Not found"))
                .unwrap());
        }
    };

    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();
    state
        .authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let offset = state
        .config()
        .tq_client
        .audience_settings
        .get(audience)
        .and_then(|s| s.preroll_offset)
        .unwrap_or_default();

    state
        .event_client()
        .commit_edition(edition_id, offset)
        .await
        .context("Failed to commit edition")
        .error(AppErrorKind::EditionFailed)?;

    let response = Response::builder()
        .status(http::StatusCode::ACCEPTED)
        .body(Body::empty())
        .unwrap();
    Ok(response)
}
