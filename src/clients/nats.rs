use uuid::Uuid;

use svc_events::{Event, EventId};

use crate::app::{
    error::{Error, ErrorExt, ErrorKind as AppErrorKind},
    AppContext,
};

const SUBJECT_PREFIX: &str = "classroom";

pub async fn publish_event(
    ctx: &dyn AppContext,
    classroom_id: Uuid,
    id: &EventId,
    event: Event,
) -> Result<(), Error> {
    let subject = svc_nats_client::Subject::new(
        SUBJECT_PREFIX.to_string(),
        classroom_id,
        id.entity_type().to_string(),
    );

    let payload = serde_json::to_vec(&event).error(AppErrorKind::InternalFailure)?;

    let event = svc_nats_client::event::Builder::new(
        subject,
        payload,
        id.to_owned(),
        ctx.agent_id().to_owned(),
    )
    .build();

    ctx.nats_client()
        .ok_or_else(|| anyhow!("nats client not found"))
        .error(AppErrorKind::NatsClientNotFound)?
        .publish(&event)
        .await
        .error(AppErrorKind::NatsPublishFailed)
}
