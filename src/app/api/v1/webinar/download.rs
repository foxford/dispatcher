use axum::extract::{Extension, Path};
use hyper::{Body, Response};
use svc_agent::Authenticable;
use svc_utils::extractors::AuthnExtractor;
use uuid::Uuid;

use super::*;

use crate::db::class::Object as Class;
use crate::db::recording::Object as Recording;
use crate::{app::metrics::AuthorizeMetrics, config::StorageConfig};

pub async fn download(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    AuthnExtractor(agent_id): AuthnExtractor,
) -> AppResult {
    let account_id = agent_id.as_account_id();

    let webinar = find::<WebinarType>(ctx.as_ref(), id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();
    ctx.authz()
        .authorize(
            webinar.audience().to_owned(),
            account_id.clone(),
            object,
            "download".into(),
        )
        .await
        .measure()?;

    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;

    let recordings = crate::db::recording::RecordingListQuery::new(webinar.id())
        .execute(&mut conn)
        .await
        .context("Failed to query webinar recordings")
        .error(AppErrorKind::DbQueryFailed)?;

    let recording = recordings
        .first()
        .ok_or_else(|| anyhow!("Zero webinar recordings"))
        .error(AppErrorKind::RecordingNotFound)?;

    let body = serde_json::json!({ "url": format_url(ctx.storage_config(), &webinar, recording) });

    let body = serde_json::to_string(&body).expect("Never fails");
    let response = Response::builder().body(Body::from(body)).unwrap();
    Ok(response)
}

fn format_url(config: &StorageConfig, webinar: &Class, recording: &Recording) -> String {
    let mut url = config.base_url.clone();
    url.set_path(&format!(
        "/api/v2/backends/yandex/sets/ms.webinar.{}::{}/objects/mp4",
        webinar.audience(),
        recording.rtc_id()
    ));

    url.to_string()
}
