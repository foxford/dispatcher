use std::sync::Arc;

use anyhow::Context;
use hyper::{Body, Response};

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::class::WebinarType;

use super::{find, AppResult};

pub async fn options() -> Response<Body> {
    Response::builder().body(Body::empty()).unwrap()
}

pub use convert::convert;
pub use create::create;
pub use download::download;

mod convert;
mod create;
mod download;
