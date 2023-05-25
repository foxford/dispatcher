use sqlx::PgConnection;
use svc_authn::AccountId;
use svc_events::{
    ban::{BanIntentV1, BanRejectedV1},
    EventId,
};
use uuid::Uuid;

use crate::{
    app::{
        error::{Error, ErrorExt, ErrorKind as AppErrorKind},
        AppContext,
    },
    clients::nats,
    db::{self, ban_account_op},
};

use super::{FailureKind, HandleMsgFailure};

const ENTITY_TYPE: &str = "ban-intent";

pub async fn start(
    ctx: &dyn AppContext,
    conn: &mut PgConnection,
    ban: bool,
    class: &db::class::Object,
    sender: AccountId,
    user_account: AccountId,
    last_op_id: Uuid,
) -> Result<(), Error> {
    let event_id = get_next_event_id(conn).await?;
    let event = BanIntentV1 {
        ban,
        classroom_id: class.id(),
        user_account,
        last_op_id,
        new_op_id: Uuid::new_v4(),
        sender,
    };
    nats::publish_event(ctx, class.id(), &event_id, event.into()).await
}

pub async fn handle(
    ctx: &dyn AppContext,
    intent: BanIntentV1,
    intent_id: EventId,
) -> Result<(), HandleMsgFailure<Error>> {
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
        intent.user_account.clone(),
        intent.last_op_id,
        intent.new_op_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)
    .transient()?;

    match op {
        Some(_) => {
            super::ban::start(ctx, intent, intent_id)
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

async fn reject(
    ctx: &dyn AppContext,
    event: BanIntentV1,
    original_event_id: &EventId,
) -> Result<(), Error> {
    let event = BanRejectedV1::from(event);
    let event_id = (
        "ban-intent-rejected".to_owned(),
        original_event_id.sequence_id(),
    )
        .into();
    // TODO: publish as personal notification
    nats::publish_event(ctx, event.classroom_id, &event_id, event.into()).await
}

async fn get_next_event_id(conn: &mut PgConnection) -> Result<EventId, Error> {
    let next_seq_id = ban_account_op::get_next_seq_id(conn)
        .await
        .error(AppErrorKind::DbQueryFailed)?;

    let event_id = (ENTITY_TYPE.to_owned(), next_seq_id.value).into();

    Ok(event_id)
}
