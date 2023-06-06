use std::{convert::TryFrom, str::FromStr, sync::Arc};

use anyhow::Context;
use svc_events::{Event, EventV1};
use svc_nats_client::{
    consumer::{FailureKind, FailureKindExt, HandleMessageFailure},
    Subject,
};

use crate::db;

use super::AppContext;

pub mod ban;

pub async fn route_message(
    ctx: Arc<dyn AppContext>,
    msg: Arc<svc_nats_client::Message>,
) -> Result<(), HandleMessageFailure<anyhow::Error>> {
    let subject = Subject::from_str(&msg.subject)
        .context("parse nats subject")
        .permanent()?;

    let event = serde_json::from_slice::<Event>(msg.payload.as_ref())
        .context("parse nats payload")
        .permanent()?;

    let classroom_id = subject.classroom_id();
    let _room = {
        let mut conn = ctx
            .get_conn()
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .transient()?;

        db::class::ReadQuery::by_id(classroom_id)
            .execute(&mut conn)
            .await
            .context("find room by classroom_id")
            .transient()?
            .ok_or(anyhow!(
                "failed to get room by classroom_id: {}",
                classroom_id
            ))
            .permanent()?
    };

    let headers = svc_nats_client::Headers::try_from(msg.headers.clone().unwrap_or_default())
        .context("parse nats headers")
        .permanent()?;
    let _agent_id = headers.sender_id();
    let event_id = headers.event_id();

    let r = match event {
        Event::V1(e) => match e {
            EventV1::BanIntent(intent) => {
                ban::handle_intent(ctx.as_ref(), intent, event_id.clone()).await
            }
            EventV1::BanVideoStreamingCompleted(event) => {
                ban::handle_video_streaming_banned(ctx.as_ref(), event).await
            }
            EventV1::BanCollaborationCompleted(event) => {
                ban::handle_collaboration_banned(ctx.as_ref(), event).await
            }
            _ => Ok(()),
        },
    };

    FailureKindExt::map_err(r, |e| anyhow::anyhow!(e))
}
