use super::*;

use crate::app::api::v1::AppError;
use crate::db::class::Object as WebinarObject;
use sqlx::Acquire;

pub async fn recreate(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    recreate_inner(req)
        .await
        .or_else(|e| Ok(e.to_tide_response()))
}

async fn recreate_inner(mut req: Request<Arc<dyn AppContext>>) -> AppResult {
    let body: WebinarUpdate = req.body_json().await.error(AppErrorKind::InvalidPayload)?;

    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let state = req.state();

    let webinar = find_webinar(&req)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();

    state
        .authz()
        .authorize(
            webinar.audience().to_owned(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await?;

    let (event_room_id, conference_room_id) =
        create_event_and_conference(req.state().as_ref(), &webinar, &body.time).await?;

    let query = crate::db::class::WebinarRecreateQuery::new(
        webinar.id(),
        body.time.into(),
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
            .context("Failed to update webinar")
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

async fn create_event_and_conference(
    state: &dyn AppContext,
    webinar: &WebinarObject,
    time: &BoundedDateTimeTuple,
) -> Result<(Uuid, Uuid), AppError> {
    let conference_time = match time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), Bound::Unbounded),
        Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
    };
    let conference_fut = state.conference_client().create_room(
        conference_time,
        webinar.audience().to_owned(),
        Some("shared".into()),
        webinar.reserve(),
        webinar.tags().map(ToOwned::to_owned),
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = state.event_client().create_room(
        event_time,
        webinar.audience().to_owned(),
        Some(true),
        webinar.tags().map(ToOwned::to_owned),
    );

    let (event_room_id, conference_room_id) = event_fut
        .try_join(conference_fut)
        .await
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    Ok((event_room_id, conference_room_id))
}
