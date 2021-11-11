use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, TypedHeader};
use chrono::Utc;
use headers::{authorization::Bearer, Authorization};
use hyper::{Body, Response};
use serde_derive::Deserialize;
use svc_agent::AccountId;
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::metrics::AuthorizeMetrics;
use crate::app::AppContext;
use crate::db::class;

use super::{validate_token, AppResult};

#[derive(Deserialize)]
pub struct P2P {
    scope: String,
    audience: String,
    tags: Option<serde_json::Value>,
    #[serde(default = "class::default_whiteboard")]
    whiteboard: bool,
}

pub async fn create(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(payload): Json<P2P>,
) -> AppResult {
    let account_id =
        validate_token(ctx.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;

    do_create(ctx.as_ref(), &account_id, payload).await
}

async fn do_create(state: &dyn AppContext, account_id: &AccountId, body: P2P) -> AppResult {
    let log = crate::LOG.new(slog::o!(
        "audience" => body.audience.clone(),
        "scope" => body.scope.clone(),
    ));
    info!(
        log,
        "Creating p2p, audience = {}, scope = {}", body.audience, body.scope
    );

    let object = AuthzObject::new(&["classrooms"]).into();

    state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "create".into(),
        )
        .await
        .measure()?;

    info!(log, "Authorized p2p create");

    let conference_fut = state.conference_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        None,
        None,
        body.tags.clone(),
        None,
    );

    let event_fut = state.event_client().create_room(
        (Bound::Included(Utc::now()), Bound::Unbounded),
        body.audience.clone(),
        Some(false),
        body.tags.clone(),
        None,
    );

    let (event_room_id, conference_room_id) = tokio::try_join!(event_fut, conference_fut)
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    info!(
        log,
        "Created event room = {}, conference room = {}", event_room_id, conference_room_id
    );

    let query = crate::db::class::P2PInsertQuery::new(
        body.scope,
        body.audience,
        conference_room_id,
        event_room_id,
    );

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };
    let p2p = {
        let mut conn = state
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;
        query
            .execute(&mut conn)
            .await
            .context("Failed to insert p2p")
            .error(AppErrorKind::DbQueryFailed)?
    };
    info!(log, "Inserted p2p into db, id = {}", p2p.id());

    if body.whiteboard {
        if let Err(e) = state.event_client().create_whiteboard(event_room_id).await {
            error!(
                crate::LOG,
                "Failed to create whiteboard in event room, id = {:?}, err = {:?}",
                event_room_id,
                e
            );
        }
    }

    crate::app::services::update_classroom_id(
        state,
        p2p.id(),
        p2p.event_room_id(),
        p2p.conference_room_id(),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    info!(log, "Successfully updated classroom room id");

    let body = serde_json::to_string_pretty(&p2p)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

#[derive(Deserialize)]
pub struct P2PConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    conference_room_id: Uuid,
    tags: Option<serde_json::Value>,
}

pub async fn convert(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    TypedHeader(Authorization(token)): TypedHeader<Authorization<Bearer>>,
    Json(body): Json<P2PConvertObject>,
) -> AppResult {
    let account_id =
        validate_token(ctx.as_ref(), token.token()).error(AppErrorKind::Unauthorized)?;

    let object = AuthzObject::new(&["classrooms"]).into();

    ctx.authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "convert".into(),
        )
        .await
        .measure()?;

    let query = crate::db::class::P2PInsertQuery::new(
        body.scope,
        body.audience,
        body.conference_room_id,
        body.event_room_id,
    );

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };

    let p2p = {
        let mut conn = ctx
            .get_conn()
            .await
            .error(AppErrorKind::DbConnAcquisitionFailed)?;

        let p2p = query
            .execute(&mut conn)
            .await
            .context("Failed to find recording")
            .error(AppErrorKind::DbQueryFailed)?;

        p2p
    };

    crate::app::services::update_classroom_id(
        ctx.as_ref(),
        p2p.id(),
        p2p.event_room_id(),
        p2p.conference_room_id(),
    )
    .await
    .error(AppErrorKind::MqttRequestFailed)?;

    let body = serde_json::to_string(&p2p)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::class::P2PReadQuery, test_helpers::prelude::*};
    use mockall::predicate as pred;
    use uuid::Uuid;

    #[tokio::test]
    async fn create_p2p() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let mut authz = TestAuthz::new();
        authz.allow(agent.account_id(), vec!["classrooms"], "create");

        let mut state = TestState::new(authz).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        create_p2p_mocks(&mut state, event_room_id, conference_room_id);

        let scope = random_string();

        let state = Arc::new(state);
        let body = P2P {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            tags: None,
            whiteboard: true,
        };

        let r = do_create(state.as_ref(), agent.account_id(), body).await;
        r.expect("Failed to create p2p");

        // Assert DB changes.
        let mut conn = state.get_conn().await.expect("Failed to get conn");

        P2PReadQuery::by_scope(USR_AUDIENCE, &scope)
            .execute(&mut conn)
            .await
            .expect("Failed to fetch p2p")
            .expect("p2p not found");
    }

    #[tokio::test]
    async fn create_p2p_unauthorized() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let state = TestState::new(TestAuthz::new()).await;

        let scope = random_string();

        let state = Arc::new(state);
        let body = P2P {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            tags: None,
            whiteboard: true,
        };

        do_create(state.as_ref(), agent.account_id(), body)
            .await
            .expect_err("Unexpectedly succeeded");
    }

    fn create_p2p_mocks(state: &mut TestState, event_room_id: Uuid, conference_room_id: Uuid) {
        state
            .event_client_mock()
            .expect_create_room()
            .with(
                pred::always(),
                pred::always(),
                pred::always(),
                pred::always(),
                pred::always(),
            )
            .returning(move |_, _, _, _, _| Ok(event_room_id));

        state
            .event_client_mock()
            .expect_create_whiteboard()
            .with(pred::eq(event_room_id))
            .returning(move |_room_id| Ok(()));

        state
            .event_client_mock()
            .expect_update_room()
            .with(pred::eq(event_room_id), pred::always())
            .returning(move |_room_id, _| Ok(()));

        state
            .conference_client_mock()
            .expect_create_room()
            .withf(move |_, _, _, _, _, _| true)
            .returning(move |_, _, _, _, _, _| Ok(conference_room_id));

        state
            .conference_client_mock()
            .expect_update_room()
            .with(pred::eq(conference_room_id), pred::always())
            .returning(move |_room_id, _| Ok(()));
    }
}
