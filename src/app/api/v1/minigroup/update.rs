use super::*;

use crate::app::api::v1::class::update as update_generic;

pub async fn update(req: Request<Arc<dyn AppContext>>) -> AppResult {
    update_generic::<MinigroupType>(req).await
}

pub async fn options(_req: Request<Arc<dyn AppContext>>) -> tide::Result {
    Ok(Response::builder(200).build())
}
