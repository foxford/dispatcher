use anyhow::Context;
use async_std::prelude::FutureExt;
use uuid::Uuid;

use crate::app::tide_state::AppContext;
use crate::clients::{
    conference::RoomUpdate as ConfRoomUpdate, event::RoomUpdate as EventRoomUpdate,
};

pub async fn update_classroom_id(
    state: &dyn AppContext,
    classroom_id: Uuid,
    event_id: Uuid,
    conference_id: Option<Uuid>,
) -> anyhow::Result<()> {
    let event_fut = state.event_client().update_room(
        event_id,
        EventRoomUpdate {
            time: None,
            classroom_id: Some(classroom_id),
        },
    );

    let result = if let Some(conference_id) = conference_id {
        let conference_fut = state.conference_client().update_room(
            conference_id,
            ConfRoomUpdate {
                time: None,
                classroom_id: Some(classroom_id),
            },
        );

        event_fut.try_join(conference_fut).await.map(|_| ())
    } else {
        event_fut.await
    };

    result.context("Services requests updating classroom_id failed")
}
