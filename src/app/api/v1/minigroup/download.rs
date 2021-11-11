use axum::extract::{Extension, Path, TypedHeader};
use headers::{authorization::Bearer, Authorization};
use hyper::{Body, Response};
use uuid::Uuid;

use super::*;

use super::validate_token;
use crate::app::api::v1::{find, AppResult};

use crate::db::class::{MinigroupType, Object as Class};
use crate::{app::metrics::AuthorizeMetrics, config::StorageConfig};

pub async fn download(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
) -> AppResult {
    let account_id =
        validate_token(ctx.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;

    let minigroup = find::<MinigroupType>(ctx.as_ref(), id)
        .await
        .error(AppErrorKind::WebinarNotFound)?;

    let object = AuthzObject::new(&["classrooms", &minigroup.id().to_string()]).into();
    ctx.authz()
        .authorize(
            minigroup.audience().to_owned(),
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

    let recordings = crate::db::recording::RecordingListQuery::new(minigroup.id())
        .execute(&mut conn)
        .await
        .context("Failed to query minigroup recordings")
        .error(AppErrorKind::DbQueryFailed)?;

    recordings
        .iter()
        .all(|recording| recording.transcoded_at().is_some())
        .then(|| ())
        .ok_or_else(|| anyhow!("Minigroup recordings were not transcoded"))
        .error(AppErrorKind::RecordingNotFound)?;

    let body = serde_json::json!({ "url": format_url(ctx.storage_config(), &minigroup) });

    let body = serde_json::to_string(&body).expect("Never fails");
    let response = Response::builder().body(Body::from(body)).unwrap();
    Ok(response)
}

fn format_url(config: &StorageConfig, minigroup: &Class) -> String {
    let mut url = config.base_url.clone();
    url.set_path(&format!(
        "/api/v2/backends/yandex/sets/ms.minigroup.{}::{}/objects/mp4",
        minigroup.audience(),
        minigroup.scope()
    ));

    url.to_string()
}