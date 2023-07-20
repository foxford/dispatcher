use sqlx::PgConnection;
use svc_agent::AgentId;
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
    db::{self, ban_account_op, ban_history},
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
    sender: AgentId,
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
    nats::publish_event(
        ctx,
        class.id(),
        &event_id,
        event.into(),
        nats::Options::default(),
    )
    .await
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
        Some(op) => {
            if intent.ban {
                ban_history::InsertQuery::new(
                    intent.classroom_id,
                    &intent.target_account,
                    op.last_op_id,
                )
                .execute(&mut conn)
                .await
                .error(AppErrorKind::DbQueryFailed)
                .transient()?;
            } else {
                ban_history::FinishBanQuery::new(
                    intent.classroom_id,
                    &intent.target_account,
                    op.last_op_id,
                )
                .execute(&mut conn)
                .await
                .error(AppErrorKind::DbQueryFailed)
                .transient()?;
            }

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

    nats::publish_event(
        ctx,
        event.classroom_id,
        &event_id,
        event.into(),
        nats::Options::default(),
    )
    .await
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
    nats::publish_event(
        ctx,
        event.classroom_id,
        &event_id,
        event.into(),
        nats::Options::default().receiver_id(intent.sender),
    )
    .await
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

    let op = ban_account_op::UpdateQuery::new_video_streaming_banned(
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

    let op = ban_account_op::UpdateQuery::new_collaboration_banned(
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
    nats::publish_event(
        ctx,
        event.classroom_id,
        &event_id,
        event.into(),
        nats::Options::default(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use svc_events::{Event, EventV1};

    use super::*;

    use crate::app::AppContext;
    use crate::test_helpers::prelude::*;

    #[sqlx::test]
    async fn handles_intents(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;

        let minigroup = {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            factory::Minigroup::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await
        };

        let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);

        let intent = BanIntentV1 {
            classroom_id: minigroup.id(),
            ban: true,
            sender: agent1.agent_id().clone(),
            last_operation_id: 0,
            target_account: agent1.account_id().clone(),
        };
        let intent_id: EventId = ("ban".to_string(), "intent".to_string(), 0).into();

        handle_intent(&state, intent.clone(), intent_id)
            .await
            .expect("failed to handle intent");

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 1);

            let payload = serde_json::from_slice::<Event>(&pub_reqs[0].payload)
                .expect("failed to parse event");
            assert!(matches!(payload, Event::V1(EventV1::BanAccepted(..))));
        }

        // should fail b/c we already started an operation with another sequence id
        let intent_id: EventId = ("ban".to_string(), "intent".to_string(), 1).into();

        handle_intent(&state, intent.clone(), intent_id)
            .await
            .expect("failed to handle intent");

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 2);

            let payload = serde_json::from_slice::<Event>(&pub_reqs[1].payload)
                .expect("failed to parse event");
            assert!(matches!(payload, Event::V1(EventV1::BanRejected(..))));
        }
    }

    #[sqlx::test]
    async fn fails_to_handle_video_streaming_banned_if_there_was_no_intent(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;
        let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);

        let video_streaming_banned = BanVideoStreamingCompletedV1 {
            ban: true,
            classroom_id: Uuid::new_v4(),
            target_account: agent1.account_id().clone(),
            operation_id: 0,
            parent: ("ban".to_owned(), "accepted".to_owned(), 0).into(),
        };

        let r = handle_video_streaming_banned(&state, video_streaming_banned).await;
        assert!(matches!(
            r,
            Err(HandleMessageFailure::Permanent(
                e @ crate::app::error::Error { .. }
            )) if e.kind() == AppErrorKind::OperationFailed
        ));

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            let r = db::ban_account_op::ReadQuery::by_account_id(agent1.account_id())
                .execute(&mut conn)
                .await
                .expect("failed to fetch ban account op entry");
            assert!(r.is_none());
        }

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 0);
        }
    }

    #[sqlx::test]
    async fn fails_to_handle_collaboration_banned_if_there_was_no_intent(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;
        let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);

        let collaboration_banned = BanCollaborationCompletedV1 {
            ban: true,
            classroom_id: Uuid::new_v4(),
            target_account: agent1.account_id().clone(),
            operation_id: 0,
            parent: ("ban".to_owned(), "accepted".to_owned(), 0).into(),
        };

        let r = handle_collaboration_banned(&state, collaboration_banned).await;
        assert!(matches!(
            r,
            Err(HandleMessageFailure::Permanent(
                e @ crate::app::error::Error { .. }
            )) if e.kind() == AppErrorKind::OperationFailed
        ));

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            let r = db::ban_account_op::ReadQuery::by_account_id(agent1.account_id())
                .execute(&mut conn)
                .await
                .expect("failed to fetch ban account op entry");
            assert!(r.is_none());
        }

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 0);
        }
    }

    #[sqlx::test]
    async fn handles_video_streaming_banned(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;

        let minigroup = {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            factory::Minigroup::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await
        };

        let agent1 = TestAgent::new("web", "user-video", USR_AUDIENCE);

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            db::ban_account_op::UpsertQuery::new_operation(agent1.account_id().clone(), 0, 0)
                .execute(&mut conn)
                .await
                .expect("failed to run upsert query");
        };

        let video_streaming_banned = BanVideoStreamingCompletedV1 {
            ban: true,
            classroom_id: minigroup.id(),
            target_account: agent1.account_id().clone(),
            operation_id: 0,
            parent: ("ban".to_owned(), "accepted".to_owned(), 0).into(),
        };

        handle_video_streaming_banned(&state, video_streaming_banned)
            .await
            .expect("failed to handle video streaming banned");

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            let r = db::ban_account_op::ReadQuery::by_account_id(agent1.account_id())
                .execute(&mut conn)
                .await
                .expect("failed to fetch ban account op entry");
            assert!(matches!(
                r,
                Some(db::ban_account_op::Object {
                    last_op_id: 0,
                    is_video_streaming_banned: true,
                    ..
                })
            ));
        }

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 0);
        }
    }

    #[sqlx::test]
    async fn handles_collaboration_banned(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;

        let minigroup = {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            factory::Minigroup::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await
        };

        let agent1 = TestAgent::new("web", "user-collab", USR_AUDIENCE);

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            db::ban_account_op::UpsertQuery::new_operation(agent1.account_id().clone(), 0, 0)
                .execute(&mut conn)
                .await
                .expect("failed to run upsert query");
        };

        let collaboration_banned = BanCollaborationCompletedV1 {
            ban: true,
            classroom_id: minigroup.id(),
            target_account: agent1.account_id().clone(),
            operation_id: 0,
            parent: ("ban".to_owned(), "accepted".to_owned(), 0).into(),
        };

        handle_collaboration_banned(&state, collaboration_banned)
            .await
            .expect("failed to handle collaboration banned");

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            let r = db::ban_account_op::ReadQuery::by_account_id(agent1.account_id())
                .execute(&mut conn)
                .await
                .expect("failed to fetch ban account op entry");
            assert!(matches!(
                r,
                Some(db::ban_account_op::Object {
                    last_op_id: 0,
                    is_collaboration_banned: true,
                    ..
                })
            ));
        }

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 0);
        }
    }

    #[sqlx::test]
    async fn finishes_operation(pool: sqlx::PgPool) {
        let state = TestState::new(pool, TestAuthz::new()).await;

        let minigroup = {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            factory::Minigroup::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await
        };

        let agent1 = TestAgent::new("web", "user-finish", USR_AUDIENCE);

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            db::ban_account_op::UpsertQuery::new_operation(agent1.account_id().clone(), 0, 0)
                .execute(&mut conn)
                .await
                .expect("failed to run upsert query");
        };

        let video_streaming_banned = BanVideoStreamingCompletedV1 {
            ban: true,
            classroom_id: minigroup.id(),
            target_account: agent1.account_id().clone(),
            operation_id: 0,
            parent: ("ban".to_owned(), "accepted".to_owned(), 0).into(),
        };

        handle_video_streaming_banned(&state, video_streaming_banned)
            .await
            .expect("failed to handle video streaming banned");

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            let r = db::ban_account_op::ReadQuery::by_account_id(agent1.account_id())
                .execute(&mut conn)
                .await
                .expect("failed to fetch ban account op entry");
            assert!(matches!(
                r,
                Some(db::ban_account_op::Object {
                    last_op_id: 0,
                    is_video_streaming_banned: true,
                    ..
                })
            ));
        }

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 0);
        }

        let collaboration_banned = BanCollaborationCompletedV1 {
            ban: true,
            classroom_id: minigroup.id(),
            target_account: agent1.account_id().clone(),
            operation_id: 0,
            parent: ("ban".to_owned(), "accepted".to_owned(), 0).into(),
        };

        handle_collaboration_banned(&state, collaboration_banned)
            .await
            .expect("failed to handle collaboration banned");

        {
            let mut conn = state.get_conn().await.expect("Failed to fetch connection");
            let r = db::ban_account_op::ReadQuery::by_account_id(agent1.account_id())
                .execute(&mut conn)
                .await
                .expect("failed to fetch ban account op entry");
            assert!(matches!(
                r,
                Some(db::ban_account_op::Object {
                    last_op_id: 0,
                    is_collaboration_banned: true,
                    is_video_streaming_banned: true,
                    ..
                })
            ));
        }

        {
            let pub_reqs = state.inspect_nats_client().get_publish_requests();
            assert_eq!(pub_reqs.len(), 1);

            let payload = serde_json::from_slice::<Event>(&pub_reqs[0].payload)
                .expect("failed to parse event");
            assert!(matches!(payload, Event::V1(EventV1::BanCompleted(..))));
        }
    }
}
