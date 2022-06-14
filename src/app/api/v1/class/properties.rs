use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, Path};
use hyper::{Body, Response};
use svc_agent::Authenticable;
use svc_authn::AccountId;
use svc_utils::extractors::AuthnExtractor;
use uuid::Uuid;

use super::*;
use crate::app::api::v1::find_class;
use crate::app::error::Error;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};
use crate::db::class::ClassProperties;

pub async fn read_property(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path((class_id, property_id)): Path<(Uuid, String)>,
    AuthnExtractor(agent_id): AuthnExtractor,
) -> AppResult {
    let account_id = agent_id.as_account_id();

    let class = find_class(ctx.as_ref(), class_id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

    AuthClassAction {
        state: ctx.as_ref(),
        account_id,
        class: &class,
        op: "read",
    }
    .check()
    .await?;

    let property = read_properties(&class)?.get(&property_id).ok_or_else(|| {
        Error::new(
            AppErrorKind::ClassPropertyNotFound,
            anyhow!("missing class property"),
        )
    })?;

    let body = serde_json::to_string(property)
        .context("Failed to serialize class property")
        .error(AppErrorKind::SerializationFailed)?;
    let response = Response::builder().body(Body::from(body)).unwrap();
    Ok(response)
}

pub async fn update_property(
    Extension(ctx): Extension<Arc<dyn AppContext>>,
    Path((class_id, property_id)): Path<(Uuid, String)>,
    AuthnExtractor(agent_id): AuthnExtractor,
    Json(payload): Json<serde_json::Value>,
) -> AppResult {
    let account_id = agent_id.as_account_id();

    let class = find_class(ctx.as_ref(), class_id)
        .await
        .error(AppErrorKind::ClassNotFound)?;

    AuthClassAction {
        state: ctx.as_ref(),
        account_id,
        class: &class,
        op: "update",
    }
    .check()
    .await?;

    let mut properties = read_properties(&class)?.clone();
    properties.insert(property_id, payload);

    // here to prevent excess `clone` of `properties`
    let body = serde_json::to_string(&properties)
        .context("Failed to serialize class property")
        .error(AppErrorKind::SerializationFailed)?;

    let query = crate::db::class::ClassUpdateQuery::new(class.id()).properties(properties);

    let mut conn = ctx.get_conn().await.error(AppErrorKind::DbQueryFailed)?;
    query
        .execute(&mut conn)
        .await
        .context("Failed to update class properties")
        .error(AppErrorKind::DbQueryFailed)?;

    let response = Response::builder().body(Body::from(body)).unwrap();
    Ok(response)
}

// TODO: tests

struct AuthClassAction<'a> {
    state: &'a dyn AppContext,
    account_id: &'a AccountId,
    class: &'a class::Object,
    op: &'static str,
}

impl AuthClassAction<'_> {
    async fn check(self) -> Result<(), Error> {
        let object = AuthzObject::new(&["classrooms", &self.class.id().to_string()]).into();
        self.state
            .authz()
            .authorize(
                self.class.audience().to_owned(),
                self.account_id.clone(),
                object,
                self.op.into(),
            )
            .await
            .measure()?;

        Ok(())
    }
}

fn read_properties(class: &class::Object) -> Result<&ClassProperties, Error> {
    let props = class
        .properties()
        .ok_or_else(|| {
            Error::new(
                AppErrorKind::NoClassProperties,
                anyhow!("no properties for this class"),
            )
        })?
        .as_object()
        .ok_or_else(|| {
            Error::new(
                AppErrorKind::InvalidClassProperties,
                anyhow!("invalid class properties"),
            )
        })?;

    Ok(props)
}
