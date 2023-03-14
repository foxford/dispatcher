use std::sync::Arc;

use axum::{extract::Path, Extension, Json};
use http::Response;
use hyper::Body;
use serde::Deserialize;
use svc_utils::extractors::AccountIdExtractor;
use uuid::Uuid;

use crate::{
    app::{
        api::v1::AppResult,
        error::{Error as AppError, ErrorExt, ErrorKind},
        metrics::AuthorizeMetrics,
        AppContext, AuthzObject,
    },
    clients::tq::Priority,
};

#[derive(Debug, Deserialize)]
pub struct RestartTranscodingPayload {
    priority: Priority,
}

impl Default for RestartTranscodingPayload {
    fn default() -> Self {
        Self {
            priority: Priority::Normal,
        }
    }
}

pub async fn restart_transcoding(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    AccountIdExtractor(account_id): AccountIdExtractor,
    Path(id): Path<Uuid>,
    payload: Option<Json<RestartTranscodingPayload>>,
) -> AppResult {
    let mut conn = ctx.get_conn().await.error(ErrorKind::InternalFailure)?;

    let webinar = crate::db::class::ReadQuery::by_id(id)
        .execute(&mut conn)
        .await
        .error(ErrorKind::InternalFailure)?
        .ok_or_else(|| AppError::from(ErrorKind::ClassNotFound))?;

    let object = AuthzObject::new(&["classrooms", &id.to_string()]).into();
    ctx.authz()
        .authorize(
            webinar.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let payload = match payload {
        Some(Json(payload)) => payload,
        None => RestartTranscodingPayload::default(),
    };

    let r = crate::app::postprocessing_strategy::restart_webinar_transcoding(
        ctx,
        webinar,
        payload.priority,
    )
    .await;

    match r {
        Ok(_) => Ok(Response::new(Body::from(""))),
        Err(err) => Err(err).error(ErrorKind::InternalFailure),
    }
}
