use tide::http::{Method, Request, Url};

use super::*;
use crate::test_helpers::prelude::*;

#[async_std::test]
async fn test_healthz() {
    let state = TestState::new(TestAuthz::new()).await;
    let state = Arc::new(state) as Arc<dyn AppContext>;
    let mut app = tide::with_state(state);
    app.at("/test/healthz").get(healthz);

    let req = Request::new(Method::Get, url("/test/healthz"));
    let mut resp: Response = app.respond(req).await.expect("Failed to get response");
    assert_eq!(resp.status(), 200);
    let body = resp
        .take_body()
        .into_string()
        .await
        .expect("Failed to get body");
    assert_eq!(body, "Ok");
}

#[async_std::test]
async fn test_api_rollback() {
    let agent = TestAgent::new("web", "user123", USR_AUDIENCE);
    let token = agent.token();
    let mut authz = TestAuthz::new();
    authz.set_audience(SVC_AUDIENCE);
    authz.allow(agent.account_id(), vec!["scopes"], "rollback");

    let state = TestState::new(authz).await;
    let state = Arc::new(state) as Arc<dyn AppContext>;
    let mut app = tide::with_state(state.clone());

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

    let path = format!("test/api/scopes/{}/rollback", scope);

    app.at("test/api/scopes/:scope/rollback")
        .post(super::super::rollback);

    let mut req = Request::new(Method::Post, url(&path));
    req.append_header("Authorization", format!("Bearer {}", token));
    let mut resp: Response = app.respond(req).await.expect("Failed to get response");

    let body = resp
        .take_body()
        .into_string()
        .await
        .expect("Failed to get body");

    assert_eq!(resp.status(), 200);
    assert_eq!(body, "Ok");
}

fn url(path: &str) -> Url {
    let mut url = Url::parse("http://example.com").expect("Wrong constant?");
    url.set_path(path);
    url
}
