use std::sync::Arc;

use anyhow::Context;
use http::header;
use hyper::{Body, Response};

use crate::app::authz::AuthzObject;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::db::class::WebinarType;

use super::{find, validate_token, AppResult};

pub async fn options() -> Response<Body> {
    Response::builder()
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS, PUT")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
        .body(Body::empty())
        .unwrap()
}

pub use convert::convert;
pub use create::create;
pub use download::download;

mod convert;
mod create;
mod download;
