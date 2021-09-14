use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::Deserialize;
use sqlx::Acquire;
use tide::{Request, Response};
use uuid::Uuid;

use super::{extract_id, find, validate_token, AppResult};
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::BoundedDateTimeTuple;
use crate::db::class::ClassType;
use crate::db::class::Object as WebinarObject;
use crate::{app::api::v1::AppError, db::class::AsClassType};

#[derive(Deserialize)]
pub struct WebinarRecreate {
    #[serde(default, with = "crate::serde::ts_seconds_option_bound_tuple")]
    time: Option<BoundedDateTimeTuple>,
}

pub async fn recreate<T: AsClassType>(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: WebinarRecreate = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let id = extract_id(&req).error(AppErrorKind::InvalidParameter)?;
    let state = req.state();

    let webinar = find::<T>(state.as_ref(), id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();

    let time = body.time.unwrap_or((Bound::Unbounded, Bound::Unbounded));

    state
        .authz()
        .authorize(
            webinar.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
        .measure()?;

    let (event_room_id, conference_room_id) =
        create_event_and_conference::<T>(req.state().as_ref(), &webinar, &time).await?;

    let query = crate::db::class::RecreateQuery::new(
        webinar.id(),
        time.into(),
        event_room_id,
        conference_room_id,
    );

    let webinar = {
        let mut conn = req
            .state()
            .get_conn()
            .await
            .error(AppErrorKind::DbQueryFailed)?;
        let mut txn = conn
            .begin()
            .await
            .context("Failed to acquire transaction")
            .error(AppErrorKind::DbQueryFailed)?;

        let webinar = query
            .execute(&mut txn)
            .await
            .with_context(|| format!("Failed to update {}", T::as_str()))
            .error(AppErrorKind::DbQueryFailed)?;

        crate::db::recording::DeleteQuery::new(webinar.id())
            .execute(&mut txn)
            .await
            .context("Failed to delete recording")
            .error(AppErrorKind::DbQueryFailed)?;

        txn.commit()
            .await
            .context("Convert transaction failed")
            .error(AppErrorKind::DbQueryFailed)?;

        webinar
    };

    let body = serde_json::to_string(&webinar)
        .context("Failed to serialize webinar")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder(200).body(body).build();

    Ok(response)
}

async fn create_event_and_conference<T: AsClassType>(
    state: &dyn AppContext,
    webinar: &WebinarObject,
    time: &BoundedDateTimeTuple,
) -> Result<(Uuid, Uuid), AppError> {
    let conference_time = match time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), Bound::Unbounded),
        Bound::Unbounded => (Bound::Included(Utc::now()), Bound::Unbounded),
    };

    let policy = match T::as_class_type() {
        ClassType::Webinar => Some("shared".to_string()),
        ClassType::Minigroup => Some("owned".to_string()),
        ClassType::P2P => None,
        ClassType::Chat => None,
    };
    let conference_fut = state.conference_client().create_room(
        conference_time,
        webinar.audience().to_owned(),
        policy,
        webinar.reserve(),
        webinar.tags().map(ToOwned::to_owned),
        Some(webinar.id())
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = state.event_client().create_room(
        event_time,
        webinar.audience().to_owned(),
        Some(true),
        webinar.tags().map(ToOwned::to_owned),
        Some(webinar.id())
    );

    let (event_room_id, conference_room_id) = event_fut
        .try_join(conference_fut)
        .await
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    Ok((event_room_id, conference_room_id))
}
