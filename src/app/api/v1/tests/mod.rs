use super::*;

use tide::http::{Method, Request, Url};

#[async_std::test]
async fn test_healthz() {
    let state = crate::test_helpers::TestState::new().await;
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

use crate::db::frontend::tests::InsertQuery as FrontendInsertQuery;
use crate::db::scope::tests::InsertQuery as ScopeInsertQuery;
#[async_std::test]
async fn test_api_rollback() {
    let state = crate::test_helpers::TestState::new().await;
    let state = Arc::new(state) as Arc<dyn AppContext>;
    let mut app = tide::with_state(state.clone());

    let scope = crate::test_helpers::random_string();
    let mut conn = state.get_conn().await.expect("Failed to get conn");
    let frontend = FrontendInsertQuery::new("http://v2.testing00.foxford.ru".into())
        .execute(&mut conn)
        .await
        .expect("Failed to seed frontend");
    ScopeInsertQuery::new(scope.clone(), frontend.id, "webinar".into())
        .execute(&mut conn)
        .await
        .expect("Failed to seed scope");

    drop(conn);

    let path = format!("test/api/scopes/{}/rollback", scope);
    app.at("test/api/scopes/:scope/rollback")
        .post(super::super::rollback);

    let req = Request::new(Method::Post, url(&path));
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
