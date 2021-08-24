use super::*;

use crate::db::class::Object as Class;
use crate::db::recording::Object as Recording;
use crate::{app::metrics::AuthorizeMetrics, config::StorageConfig};

pub async fn download(req: Request<Arc<dyn AppContext>>) -> AppResult {
    let account_id = validate_token(&req).error(AppErrorKind::Unauthorized)?;
    let id = extract_id(&req).error(AppErrorKind::InvalidParameter)?;
    let state = req.state();

    let webinar = find::<WebinarType>(state.as_ref(), id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &webinar.id().to_string()]).into();
    state
        .authz()
        .authorize(
            webinar.audience().to_owned(),
            account_id.clone(),
            object,
            "download".into(),
        )
        .await
        .measure()?;

    let mut conn = req
        .state()
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;

    let recordings = crate::db::recording::RecordingListQuery::new(webinar.id())
        .execute(&mut conn)
        .await
        .context("Failed to find recording")
        .error(AppErrorKind::DbQueryFailed)?;

    let recording = recordings
        .first()
        .ok_or_else(|| anyhow!("Failed to find recording"))
        .error(AppErrorKind::RecordingNotFound)?;

    let body =
        serde_json::json!({ "url": format_url(req.state().storage_config(), &webinar, recording) });

    let body = serde_json::to_string(&body).expect("Never fails");
    let response = Response::builder(200).body(body).build();
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
