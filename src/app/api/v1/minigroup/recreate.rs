use super::*;

use crate::app::api::v1::class::recreate as recreate_generic;

pub async fn recreate(req: Request<Arc<dyn AppContext>>) -> AppResult {
    recreate_generic::<MinigroupType>(req).await
}
