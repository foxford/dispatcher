use axum::extract::{Extension, Path};
use hyper::{Body, Response};
use svc_utils::extractors::AccountIdExtractor;
use uuid::Uuid;

use super::*;

use crate::db::class::Object as Class;
use crate::db::recording::Object as Recording;
use crate::{app::metrics::AuthorizeMetrics, config::StorageConfig};

pub async fn download(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path(id): Path<Uuid>,
    AccountIdExtractor(account_id): AccountIdExtractor,
) -> AppResult {
    let webinar = find::<WebinarType>(ctx.as_ref(), id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

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
    let recording_id = format!("ms.webinar.{}::{}", webinar.audience(), recording.rtc_id());
    url.path_segments_mut()
        .expect("cannot-be-a-base URL")
        .extend(&[
            "api",
            "v2",
            "backends",
            "yandex",
            "sets",
            &recording_id,
            "objects",
            "mp4",
        ]);

    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::prelude::*;

    use chrono::{Duration, Utc};
    use hyper::body::to_bytes;

    #[tokio::test]
    async fn create_webinar_timestamp() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let db_pool = TestDb::new().await;

        let webinar = {
            let mut conn = db_pool.get_conn().await;
            let webinar = factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (
                    Bound::Unbounded,
                    Bound::Excluded(Utc::now() - Duration::seconds(10)),
                )
                    .into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await;

            factory::Recording::new(webinar.id(), Uuid::new_v4(), agent.agent_id().clone())
                .transcoded_at(Utc::now())
                .insert(&mut conn)
                .await;

            webinar
        };

        let mut authz = TestAuthz::new();
        authz.allow(
            agent.account_id(),
            vec!["classrooms", &webinar.id().to_string()],
            "download",
        );

        let state = TestState::new_with_pool(db_pool, authz);
        let state = Arc::new(state);

        let r = download(
            Extension(state.clone()),
            Path(webinar.id()),
            AccountIdExtractor(agent.account_id().to_owned()),
        )
        .await
        .expect("shouldn't fail");

        let body = to_bytes(r.into_body()).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(body["url"]
            .as_str()
            .unwrap()
            .starts_with(state.config().storage.base_url.as_str()));
    }
}
