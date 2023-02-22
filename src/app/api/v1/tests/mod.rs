use std::collections::HashMap;

use hyper::http::Request;
use svc_authn::jose::ConfigMap;
use tower::ServiceExt;

use super::*;
use crate::app::http;
use crate::test_helpers::prelude::*;

#[tokio::test]
async fn test_healthz() {
    let state = TestState::new(TestAuthz::new()).await;
    let state = Arc::new(state) as Arc<dyn AppContext>;
    let app = http::router(state, HashMap::new());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"Ok");
}

#[tokio::test]
async fn test_api_rollback() {
    let agent = TestAgent::new("web", "user123", USR_AUDIENCE);
    let token = agent.token();
    let mut authz = TestAuthz::new();
    authz.set_audience(SVC_AUDIENCE);
    authz.allow(agent.account_id(), vec!["scopes"], "rollback");

    let state = TestState::new(authz).await;
    let state = Arc::new(state) as Arc<dyn AppContext>;
    let app = crate::app::http::router(state.clone(), make_authn());

    let scope = shared_helpers::random_string();

    {
        let mut conn = state.get_conn().await.expect("Failed to get conn");

        let frontend = factory::Frontend::new("http://v2.testing00.foxford.ru".into())
            .execute(&mut conn)
            .await
            .expect("Failed to seed frontend");

        factory::Scope::new(scope.clone(), frontend.id, "webinar".into())
            .execute(&mut conn)
            .await
            .expect("Failed to seed scope");
    }

    let path = format!("/api/scopes/{}/rollback", scope);

    let req = Request::post(path)
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    //assert_eq!(resp.status(), 200);
    let body = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"Ok");
}

fn make_authn() -> ConfigMap {
    use crate::test_helpers::*;

    serde_json::from_str(&format!(
        r###"
    {{
        "{}": {{
            "algorithm": "ES256",
            "audience": ["{}"],
            "key": "{}"
        }}

    }}
    "###,
        TOKEN_ISSUER, USR_AUDIENCE, PUBKEY_PATH
    ))
    .unwrap()
}
