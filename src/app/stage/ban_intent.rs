use sqlx::PgConnection;
use svc_authn::AccountId;
use svc_nats_client::EventId;
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
) -> Result<(), Error> {
    let event_id = get_next_event_id(conn).await?;
    let event = svc_events::ban::BanIntentEventV1 {
        ban,
        classroom_id: class.id(),
        user_account,
        op_id: Uuid::new_v4(),
    };
    nats::publish_event(ctx, class.id(), &event_id, event.into()).await
}

async fn get_next_event_id(conn: &mut PgConnection) -> Result<EventId, Error> {
    let next_seq_id = ban_account_op::get_next_seq_id(conn)
        .await
        .error(AppErrorKind::DbQueryFailed)?;

    let event_id = (ENTITY_TYPE.to_owned(), next_seq_id.value).into();

    Ok(event_id)
}
