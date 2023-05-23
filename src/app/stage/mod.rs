use std::sync::Arc;

use super::AppContext;

pub mod ban;
pub mod ban_intent;

pub async fn route_message<'a>(
    ctx: Arc<dyn AppContext>,
    msg: &'a svc_nats_client::Message,
) -> svc_nats_client::consumer::HandleMessageOutcome {
    todo!()
}
