use super::*;

use crate::app::api::v1::class::{
    update as update_generic, update_by_scope as update_by_scope_generic,
};

pub async fn update(req: Request<Arc<dyn AppContext>>) -> AppResult {
    update_generic::<MinigroupType>(req).await
}

pub async fn update_by_scope(req: Request<Arc<dyn AppContext>>) -> AppResult {
    update_by_scope_generic::<MinigroupType>(req).await
}
