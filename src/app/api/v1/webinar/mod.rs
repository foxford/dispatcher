use std::sync::Arc;

use anyhow::Context;
use tide::{Request, Response};

use crate::app::api::v1::class::{read as read_generic, read_by_scope as read_by_scope_generic};
use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::class::WebinarType;

use super::{extract_id, find, validate_token, AppResult};

pub async fn read(req: Request<Arc<dyn AppContext>>) -> AppResult {
    read_generic::<WebinarType>(req).await
}

pub async fn read_by_scope(req: Request<Arc<dyn AppContext>>) -> AppResult {
    read_by_scope_generic::<WebinarType>(req).await
}

pub async fn options(_req: Request<Arc<dyn AppContext>>) -> tide::Result {
    Ok(Response::builder(200).build())
}

pub use convert::convert;
pub use create::create;
pub use download::download;
pub use recreate::recreate;
pub use update::update;

mod convert;
mod create;
mod download;
mod recreate;
mod update;
