use std::ops::Bound;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json};
use hyper::{Body, Response};
use serde_derive::Deserialize;
use svc_agent::AccountId;
use svc_agent::Authenticable;
use svc_utils::extractors::AuthnExtractor;
use tracing::{error, info, instrument};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::metrics::AuthorizeMetrics;
use crate::app::services;
use crate::app::AppContext;
use crate::db::class;
use crate::db::class::ClassProperties;
use crate::db::class::ClassType;

use super::AppError;
use super::AppResult;

#[derive(Deserialize)]
pub struct P2PCreatePayload {
    scope: String,
    audience: String,
    tags: Option<serde_json::Value>,
    #[serde(default)]
    properties: ClassProperties,
    #[serde(default = "class::default_whiteboard")]
    whiteboard: bool,
}

#[instrument(
    skip_all,
    fields(
        audience = ?body.audience,
        scope = ?body.scope
    )
)]
pub async fn create(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    AuthnExtractor(agent_id): AuthnExtractor,
    Json(body): Json<P2PCreatePayload>,
) -> AppResult {
    info!("Creating p2p");
    let r = do_create(ctx.as_ref(), agent_id.as_account_id(), body).await;
    if let Err(e) = &r {
        error!(error = ?e, "Failed to create p2p");
    }
    r
}

async fn do_create(
    state: &dyn AppContext,
    account_id: &AccountId,
    body: P2PCreatePayload,
) -> AppResult {
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

    info!("Authorized p2p create");

    let dummy = insert_p2p_dummy(state, &body).await?;

    let time = (Bound::Unbounded, Bound::Unbounded);
    let result = services::create_event_and_conference_rooms(state, &dummy, &time).await;
    let mut conn = state
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    let event_room_id = match result {
        Ok((event_id, conference_id)) => {
            info!(?event_id, ?conference_id, "Created rooms",);

            class::EstablishQuery::new(dummy.id(), event_id, conference_id)
                .execute(&mut conn)
                .await
                .context("Failed to establish webinar dummy")
                .error(AppErrorKind::DbQueryFailed)?;
            event_id
        }
        Err(e) => {
            info!("Failed to create rooms");

            class::DeleteQuery::new(dummy.id())
                .execute(&mut conn)
                .await
                .context("Failed to delete webinar dummy")
                .error(AppErrorKind::DbQueryFailed)?;
            return Err(e);
        }
    };

    if body.whiteboard {
        if let Err(e) = state.event_client().create_whiteboard(event_room_id).await {
            error!(
                ?event_room_id,
                "Failed to create whiteboard in event room, err = {:?}", e
            );
        }
    }

    let body = serde_json::to_string_pretty(&dummy)
        .context("Failed to serialize p2p")
        .error(AppErrorKind::SerializationFailed)?;

    let response = Response::builder()
        .status(201)
        .body(Body::from(body))
        .unwrap();

    Ok(response)
}

async fn insert_p2p_dummy(
    state: &dyn AppContext,
    body: &P2PCreatePayload,
) -> Result<class::Dummy, AppError> {
    let query = class::InsertQuery::new(
        ClassType::P2P,
        body.scope.clone(),
        body.audience.clone(),
        (Bound::Unbounded, Bound::Unbounded).into(),
    )
    .properties(body.properties.clone())
    .preserve_history(true);

    let query = if let Some(ref tags) = body.tags {
        query.tags(tags.clone())
    } else {
        query
    };

    let mut conn = state
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;
    query
        .execute(&mut conn)
        .await
        .context("Failed to insert p2p")
        .error(AppErrorKind::DbQueryFailed)
}

#[derive(Deserialize)]
pub struct P2PConvertObject {
    scope: String,
    audience: String,
    event_room_id: Uuid,
    conference_room_id: Uuid,
    tags: Option<serde_json::Value>,
    #[serde(default)]
    properties: ClassProperties,
}

pub async fn convert(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    AuthnExtractor(agent_id): AuthnExtractor,
    Json(body): Json<P2PConvertObject>,
) -> AppResult {
    let account_id = agent_id.as_account_id();

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

    let query = query.properties(body.properties);

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
        let body = P2PCreatePayload {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            tags: None,
            properties: ClassProperties::default(),
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
        let body = P2PCreatePayload {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            tags: None,
            properties: ClassProperties::default(),
            whiteboard: true,
        };

        do_create(state.as_ref(), agent.account_id(), body)
            .await
            .expect_err("Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn create_p2p_with_properties() {
        let agent = TestAgent::new("web", "user1", USR_AUDIENCE);

        let mut authz = TestAuthz::new();
        authz.allow(agent.account_id(), vec!["classrooms"], "create");

        let mut state = TestState::new(authz).await;
        let event_room_id = Uuid::new_v4();
        let conference_room_id = Uuid::new_v4();

        create_p2p_mocks(&mut state, event_room_id, conference_room_id);

        let scope = random_string();

        let mut properties: ClassProperties = serde_json::Map::new().into();
        properties.insert("is_adult".into(), true.into());

        let state = Arc::new(state);
        let body = P2PCreatePayload {
            scope: scope.clone(),
            audience: USR_AUDIENCE.to_string(),
            tags: None,
            properties: properties.clone(),
            whiteboard: true,
        };

        let r = do_create(state.as_ref(), agent.account_id(), body).await;
        r.expect("Failed to create p2p");

        // Assert DB changes.
        let mut conn = state.get_conn().await.expect("Failed to get conn");

        let new_p2p = P2PReadQuery::by_scope(USR_AUDIENCE, &scope)
            .execute(&mut conn)
            .await
            .expect("Failed to fetch p2p")
            .expect("P2P not found");

        assert_eq!(*new_p2p.properties(), properties);
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
