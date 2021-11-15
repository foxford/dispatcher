use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Path, TypedHeader};
use chrono::Utc;
use headers::{authorization::Bearer, Authorization};
use hyper::{Body, Response};
use svc_authn::AccountId;
use tracing::error;
use uuid::Uuid;

use super::*;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::{AsClassType, Object as Class};

pub async fn read<T: AsClassType>(
    ctx: Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
) -> AppResult {
    let account_id =
        validate_token(ctx.0.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;

    do_read::<T>(ctx.0.as_ref(), &account_id, id).await
}

async fn do_read<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    id: Uuid,
) -> AppResult {
    let class = find::<T>(state, id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    do_read_inner::<T>(state, account_id, class).await
}

pub async fn read_by_scope<T: AsClassType>(
    ctx: Extension<Arc<dyn AppContext>>,
    Path((audience, scope)): Path<(String, String)>,
    TypedHeader(token): TypedHeader<Authorization<Bearer>>,
) -> AppResult {
    let account_id =
        validate_token(ctx.0.as_ref(), token.0.token()).error(AppErrorKind::Unauthorized)?;

    do_read_by_scope::<T>(ctx.0.as_ref(), &account_id, &audience, &scope).await
}

async fn do_read_by_scope<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    audience: &str,
    scope: &str,
) -> AppResult {
    let class = match find_by_scope::<T>(state, audience, scope).await {
        Ok(class) => class,
        Err(e) => {
            error!("Failed to find a {}, err = {:?}", T::as_str(), e);
            return Ok(Response::builder()
                .status(404)
                .body(Body::from("Not found"))
                .unwrap());
        }
    };

    do_read_inner::<T>(state, account_id, class).await
}

async fn do_read_inner<T: AsClassType>(
    state: &dyn AppContext,
    account_id: &AccountId,
    class: Class,
) -> AppResult {
    let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();
    state
        .authz()
        .authorize(
            class.audience().to_owned(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await
        .measure()?;
    let recordings = {
        let mut conn = state
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;
        crate::db::recording::RecordingListQuery::new(class.id())
            .execute(&mut conn)
            .await
            .context("Failed to find recording")
            .error(AppErrorKind::DbQueryFailed)?
    };
    let mut class_body: ClassResponseBody = (&class).into();

    let class_end = class.time().end();
    if let Some(recording) = recordings.first() {
        // BEWARE: the order is significant
        // as of now its expected that modified version is second
        if let Some(og_event_id) = class.original_event_room_id() {
            class_body.add_version(ClassroomVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: class.tags().map(ToOwned::to_owned),
                room_events_uri: None,
            });
        }

        class_body.set_rtc_id(recording.rtc_id());

        if recording.transcoded_at().is_some() {
            if let Some(md_event_id) = class.modified_event_room_id() {
                class_body.add_version(ClassroomVersion {
                    version: "modified",
                    stream_id: recording.rtc_id(),
                    event_room_id: md_event_id,
                    tags: class.tags().map(ToOwned::to_owned),
                    room_events_uri: class.room_events_uri().cloned(),
                });
            }

            class_body.set_status(ClassStatus::Transcoded);
        } else if recording.adjusted_at().is_some() {
            class_body.set_status(ClassStatus::Adjusted);
        } else {
            class_body.set_status(ClassStatus::Finished);
        }
    } else if class_end.map(|t| Utc::now() > *t).unwrap_or(false) {
        class_body.set_status(ClassStatus::Closed);
    } else {
        class_body.set_status(ClassStatus::RealTime);
    }

    let body = serde_json::to_string(&class_body)
        .context("Failed to serialize minigroup")
        .error(AppErrorKind::SerializationFailed)?;
    let response = Response::builder().body(Body::from(body)).unwrap();
    Ok(response)
}
