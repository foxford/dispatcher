use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Extension, Json, Path};
use chrono::Utc;
use hyper::{Body, Response};
use svc_agent::Authenticable;
use svc_authn::AccountId;
use svc_utils::extractors::AuthnExtractor;
use tracing::error;
use uuid::Uuid;

use super::*;
use crate::app::api::v1::find_class;
use crate::app::error::Error;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::app::{authz::AuthzObject, metrics::AuthorizeMetrics};

pub async fn read_property(
    ctx: Extension<Arc<dyn AppContext>>,
    Path((class_id, property_id)): Path<(Uuid, String)>,
    AuthnExtractor(agent_id): AuthnExtractor,
) -> AppResult {
    ReadProperty {
        class_id,
        property_id,
        state: ctx.0.as_ref(),
        account_id: agent_id.as_account_id(),
    }
    .run()
    .await
}

struct ReadProperty<'a> {
    state: &'a dyn AppContext,
    account_id: &'a AccountId,
    class_id: Uuid,
    property_id: String,
}

impl ReadProperty<'_> {
    async fn run(self) -> AppResult {
        let class = find_class(self.state, self.class_id)
            .await
            .error(AppErrorKind::ClassNotFound)?;

        let object = AuthzObject::new(&["classrooms", &class.id().to_string()]).into();
        self.state
            .authz()
            .authorize(
                class.audience().to_owned(),
                self.account_id.clone(),
                object,
                "read".into(),
            )
            .await
            .measure()?;

        let property = class
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
            })?
            .get(&self.property_id)
            .ok_or_else(|| {
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
}

// pub async fn update_property<T: AsClassType>(
//     Extension(ctx): Extension<Arc<dyn AppContext>>,
//     Path(id): Path<Uuid>,
//     AuthnExtractor(agent_id): AuthnExtractor,
//     Json(payload): Json<ClassUpdate>,
// ) -> AppResult {
//     let class = find::<T>(ctx.as_ref(), id)
//         .await
//         .error(AppErrorKind::ClassNotFound)?;
//     let updated_class =
//         do_update::<T>(ctx.as_ref(), agent_id.as_account_id(), class, payload).await?;
//     Ok(Response::builder()
//         .body(Body::from(
//             serde_json::to_string(&updated_class)
//                 .context("Failed to serialize minigroup")
//                 .error(AppErrorKind::SerializationFailed)?,
//         ))
//         .unwrap())
// }
