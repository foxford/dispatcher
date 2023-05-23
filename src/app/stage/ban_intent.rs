use sqlx::PgConnection;
use svc_authn::AccountId;
use svc_events::{ban::BanIntentEventV1, EventId};
use uuid::Uuid;

use crate::{
    app::{
        error::{Error, ErrorExt, ErrorKind as AppErrorKind},
        AppContext,
    },
    clients::nats,
    db::{self, ban_account_op},
};

const ENTITY_TYPE: &str = "ban-intent";

pub async fn start(
    ctx: &dyn AppContext,
    conn: &mut PgConnection,
    ban: bool,
    class: &db::class::Object,
    user_account: AccountId,
    last_op_id: Uuid,
) -> Result<(), Error> {
    let event_id = get_next_event_id(conn).await?;
    let event = BanIntentEventV1 {
        ban,
        classroom_id: class.id(),
        user_account,
        last_op_id,
        new_op_id: Uuid::new_v4(),
    };
    nats::publish_event(ctx, class.id(), &event_id, event.into()).await
}

pub async fn handle(
    ctx: &dyn AppContext,
    intent: BanIntentEventV1,
    intent_id: EventId,
) -> Result<(), Error> {
    let mut conn = ctx
        .get_conn()
        .await
        .error(AppErrorKind::DbConnAcquisitionFailed)?;

    // We need to update db here first, then schedule next stage, then acknowledge
    // current message. In case if we fail somewhere in between, we can continue
    // processing as if nothing happened -- subsequent upserts will be successful,
    // attempts to schedule the same message will fail (dedup).

    ban_account_op::UpsertQuery::new_operation(
        intent.user_account.clone(),
        intent.last_op_id,
        intent.new_op_id,
    )
    .execute(&mut conn)
    .await
    .error(AppErrorKind::DbQueryFailed)?
    // failed to upsert -- we've lost the race
    .ok_or(Error::from(AppErrorKind::OperationIdObsolete))?;

    super::ban::start(ctx, intent, intent_id).await?;

    Ok(())
}

async fn get_next_event_id(conn: &mut PgConnection) -> Result<EventId, Error> {
    let next_seq_id = ban_account_op::get_next_seq_id(conn)
        .await
        .error(AppErrorKind::DbQueryFailed)?;

    let event_id = (ENTITY_TYPE.to_owned(), next_seq_id.value).into();

    Ok(event_id)
}
