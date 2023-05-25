use svc_events::{
    ban::{
        BanAcceptedV1, BanCollaborationCompletedV1, BanCompletedV1, BanIntentV1,
        BanVideoStreamingCompletedV1,
    },
    EventId,
};

use crate::{
    app::{
        error::{Error, ErrorExt, ErrorKind as AppErrorKind},
        AppContext,
    },
    clients::nats,
    db::ban_account_op,
};

use super::{FailureKind, HandleMsgFailure};

const ENTITY_TYPE: &str = "ban";

pub async fn start(
    ctx: &dyn AppContext,
    intent: BanIntentV1,
    intent_id: EventId,
) -> Result<(), Error> {
    let event_id = (ENTITY_TYPE.to_owned(), intent_id.sequence_id()).into();
    let event = BanAcceptedV1::from(intent);
    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}

pub async fn handle_video_complete(
    ctx: &dyn AppContext,
    video_streaming_banned: BanVideoStreamingCompletedV1,
) -> Result<(), HandleMsgFailure<Error>> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)
        .transient()?;

    let op = ban_account_op::UpsertQuery::new_video_streaming_banned(
        video_streaming_banned.user_account.clone(),
        video_streaming_banned.op_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?
    .ok_or(Error::from(AppErrorKind::OperationFailure))
    .permanent()?;

    if op.is_completed() {
        let original_event_id = video_streaming_banned.parent.clone();
        finish(ctx, video_streaming_banned, original_event_id)
            .await
            .transient()?;
    }

    Ok(())
}

pub async fn handle_collaboration_banned(
    ctx: &dyn AppContext,
    collaboration_banned: BanCollaborationCompletedV1,
) -> Result<(), HandleMsgFailure<Error>> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)
        .transient()?;

    let op = ban_account_op::UpsertQuery::new_collaboration_banned(
        collaboration_banned.user_account.clone(),
        collaboration_banned.op_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?
    .ok_or(Error::from(AppErrorKind::OperationFailure))
    .permanent()?;

    if op.is_completed() {
        let original_event_id = collaboration_banned.parent.clone();
        finish(ctx, collaboration_banned, original_event_id)
            .await
            .transient()?;
    }

    Ok(())
}

async fn finish(
    ctx: &dyn AppContext,
    event: impl Into<BanCompletedV1>,
    original_event_id: EventId,
) -> Result<(), Error> {
    let event_id = (ENTITY_TYPE.to_owned(), original_event_id.sequence_id()).into();
    let event: BanCompletedV1 = event.into();
    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}
