use svc_events::{
    ban::{BanEventV1, BanIntentEventV1},
    EventId,
};

use crate::{
    app::{error::Error, AppContext},
    clients::nats,
};

const ENTITY_TYPE: &str = "ban";

pub async fn start(
    ctx: &dyn AppContext,
    intent: BanIntentEventV1,
    intent_id: EventId,
) -> Result<(), Error> {
    let event_id = (ENTITY_TYPE.to_owned(), intent_id.sequence_id()).into();
    let event = BanEventV1::from(intent);
    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}
