use std::sync::Arc;

use anyhow::Context;
use hyper::{Body, Response};

use crate::app::authz::AuthzObject;
use crate::app::AppContext;
use crate::app::{error::ErrorExt, error::ErrorKind as AppErrorKind};
use crate::db::class::WebinarType;

use super::{find, AppResult};

pub async fn options() -> Response<Body> {
    Response::builder().body(Body::empty()).unwrap()
}

pub use convert::convert as convert_webinar;
pub use create::*;
pub use download::download as download_webinar;

mod convert;
mod create;
mod download;
