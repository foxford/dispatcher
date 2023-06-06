use sqlx::PgConnection;
use svc_authn::AccountId;
use svc_events::{
    ban::{
        BanAcceptedV1, BanCollaborationCompletedV1, BanCompletedV1, BanIntentV1, BanRejectedV1,
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
    db::{self, ban_account_op},
};

use super::{FailureKind, HandleMessageFailure};

const ENTITY_TYPE: &str = "ban";

const INTENT_OP: &str = "intent";
const ACCEPTED_OP: &str = "accepted";
const REJECTED_OP: &str = "rejected";
const COMPLETED_OP: &str = "completed";

pub async fn save_intent(
    ctx: &dyn AppContext,
    conn: &mut PgConnection,
    ban: bool,
    class: &db::class::Object,
    sender: AccountId,
    target_account: AccountId,
    last_operation_id: i64,
) -> Result<(), Error> {
    let event_id = get_next_event_id(conn).await?;
    let event = BanIntentV1 {
        ban,
        classroom_id: class.id(),
        sender,
        target_account,
        last_operation_id,
    };
    nats::publish_event(ctx, class.id(), &event_id, event.into()).await
}

async fn get_next_event_id(conn: &mut PgConnection) -> Result<EventId, Error> {
    let next_seq_id = ban_account_op::get_next_seq_id(conn)
        .await
        .error(AppErrorKind::DbQueryFailed)?;

    let event_id = (
        ENTITY_TYPE.to_owned(),
        INTENT_OP.to_owned(),
        next_seq_id.value,
    )
        .into();

    Ok(event_id)
}

pub async fn handle_intent(
    ctx: &dyn AppContext,
    intent: BanIntentV1,
    intent_id: EventId,
) -> Result<(), HandleMessageFailure<Error>> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)
        .transient()?;

    // We need to update db here first, then schedule next stage, then acknowledge
    // current message. In case if we fail somewhere in between, we can continue
    // processing as if nothing happened -- subsequent upserts will be successful,
    // attempts to schedule the same message will fail (dedup).

    let op = ban_account_op::UpsertQuery::new_operation(
        intent.target_account.clone(),
        intent.last_operation_id,
        intent_id.sequence_id(),
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?;

    match op {
        Some(_) => {
            super::ban::accept(ctx, intent, intent_id)
                .await
                .transient()?;
        }
        // failed to upsert -- we've lost the race
        None => {
            reject(ctx, intent, &intent_id).await.transient()?;
        }
    }

    Ok(())
}

pub async fn accept(
    ctx: &dyn AppContext,
    intent: BanIntentV1,
    intent_id: EventId,
) -> Result<(), Error> {
    let event_id = (
        ENTITY_TYPE.to_owned(),
        ACCEPTED_OP.to_owned(),
        intent_id.sequence_id(),
    )
        .into();
    let event = BanAcceptedV1 {
        ban: intent.ban,
        classroom_id: intent.classroom_id,
        target_account: intent.target_account,
        operation_id: intent_id.sequence_id(),
    };

    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}

async fn reject(
    ctx: &dyn AppContext,
    intent: BanIntentV1,
    intent_id: &EventId,
) -> Result<(), Error> {
    let event = BanRejectedV1 {
        ban: intent.ban,
        classroom_id: intent.classroom_id,
        target_account: intent.target_account,
        operation_id: intent_id.sequence_id(),
    };
    let event_id = (
        ENTITY_TYPE.to_owned(),
        REJECTED_OP.to_owned(),
        intent_id.sequence_id(),
    )
        .into();
    // TODO: publish as personal notification
    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}

pub async fn handle_video_streaming_banned(
    ctx: &dyn AppContext,
    video_streaming_banned: BanVideoStreamingCompletedV1,
) -> Result<(), HandleMessageFailure<Error>> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)
        .transient()?;

    let op = ban_account_op::UpsertQuery::new_video_streaming_banned(
        video_streaming_banned.target_account.clone(),
        video_streaming_banned.operation_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?
    .ok_or(Error::from(AppErrorKind::OperationFailed))
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
) -> Result<(), HandleMessageFailure<Error>> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)
        .transient()?;

    let op = ban_account_op::UpsertQuery::new_collaboration_banned(
        collaboration_banned.target_account.clone(),
        collaboration_banned.operation_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?
    .ok_or(Error::from(AppErrorKind::OperationFailed))
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
    let event_id = (
        ENTITY_TYPE.to_owned(),
        COMPLETED_OP.to_owned(),
        original_event_id.sequence_id(),
    )
        .into();
    let event: BanCompletedV1 = event.into();
    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}
