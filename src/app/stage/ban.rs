use svc_events::{
    ban::{BanEventAccessCompleteV1, BanEventV1, BanIntentEventV1, BanVideoCompleteV1},
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
    intent: BanIntentEventV1,
    intent_id: EventId,
) -> Result<(), Error> {
    let event_id = (ENTITY_TYPE.to_owned(), intent_id.sequence_id()).into();
    let event = BanEventV1::from(intent);
    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}

pub async fn handle_video_complete(
    ctx: &dyn AppContext,
    video_complete: BanVideoCompleteV1,
    event_id: EventId,
) -> Result<(), HandleMsgFailure<Error>> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)
        .transient()?;

    let op = ban_account_op::UpsertQuery::new_video_complete(
        video_complete.user_account.clone(),
        video_complete.op_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?
    .ok_or(Error::from(AppErrorKind::OperationFailure))
    .permanent()?;

    if op.complete() {
        // TODO: finish flow
    }

    Ok(())
}

pub async fn handle_event_access_complete(
    ctx: &dyn AppContext,
    event_access_complete: BanEventAccessCompleteV1,
    event_id: EventId,
) -> Result<(), HandleMsgFailure<Error>> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)
        .transient()?;

    let op = ban_account_op::UpsertQuery::new_event_access_complete(
        event_access_complete.user_account.clone(),
        event_access_complete.op_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?
    .ok_or(Error::from(AppErrorKind::OperationFailure))
    .permanent()?;

    if op.complete() {
        // TODO: finish flow
    }

    Ok(())
}
