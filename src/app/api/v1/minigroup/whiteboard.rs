use crate::{
    app::{
        api::v1::{find, AppResult},
        error::{Error, ErrorExt, ErrorKind},
        metrics::AuthorizeMetrics,
        AppContext, AuthzObject,
    },
    db::class::MinigroupType,
};
use axum::extract::{Extension, Path};
use hyper::{Body, Response};
use std::sync::Arc;
use svc_utils::extractors::AccountIdExtractor;
use tracing::error;
use uuid::Uuid;

pub async fn create_whiteboard(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    AccountIdExtractor(account_id): AccountIdExtractor,
) -> AppResult {
    let minigroup = find::<MinigroupType>(ctx.as_ref(), id)
        .await
        .error(ErrorKind::ClassNotFound)?;

    let object = AuthzObject::new(&["classrooms", &minigroup.id().to_string()]).into();

    ctx.authz()
        .authorize(
            minigroup.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let event_room_id = minigroup.event_room_id();

    ctx.event_client()
        .create_whiteboard(event_room_id)
        .await
        .map_err(|e| {
            error!(
                ?event_room_id,
                "Failed to create whiteboard in event room, err = {:?}", e
            );
            Error::new(ErrorKind::CreationWhiteboardFailed, e.into())
        })?;

    let response = Response::builder().status(201).body(Body::empty()).unwrap();
    Ok(response)
}
