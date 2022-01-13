use std::ops::Bound;

use anyhow::Context;
use chrono::Utc;
use tracing::error;
use uuid::Uuid;

use crate::app::api::v1::AppError;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::clients::{
    conference::RoomUpdate as ConfRoomUpdate, event::RoomUpdate as EventRoomUpdate,
};
use crate::db::class::{BoundedDateTimeTuple, RtcSharingPolicy};

pub async fn update_classroom_id(
    state: &dyn AppContext,
    classroom_id: Uuid,
    event_id: Uuid,
    conference_id: Uuid,
) -> anyhow::Result<()> {
    let event_fut = state.event_client().update_room(
        event_id,
        EventRoomUpdate {
            time: None,
            classroom_id: Some(classroom_id),
        },
    );

    let conference_fut = state.conference_client().update_room(
        conference_id,
        ConfRoomUpdate {
            time: None,
            reserve: None,
            classroom_id: Some(classroom_id),
            host: None,
        },
    );

    let result = tokio::try_join!(event_fut, conference_fut).map(|_| ());

    result.context("Services requests updating classroom_id failed")
}

pub async fn create_event_and_conference(
    state: &dyn AppContext,
    webinar: &impl Creatable,
    time: &BoundedDateTimeTuple,
) -> Result<(Uuid, Uuid), AppError> {
    let conference_time = match time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), Bound::Unbounded),
        Bound::Unbounded => (Bound::Included(Utc::now()), Bound::Unbounded),
    };

    let policy = webinar
        .rtc_sharing_policy()
        .as_ref()
        .map(ToString::to_string);

    let conference_fut = state.conference_client().create_room(
        conference_time,
        webinar.audience().to_owned(),
        policy,
        webinar.reserve(),
        webinar.tags().map(ToOwned::to_owned),
        Some(webinar.id()),
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = state.event_client().create_room(
        event_time,
        webinar.audience().to_owned(),
        Some(true),
        webinar.tags().map(ToOwned::to_owned),
        Some(webinar.id()),
    );

    let (event_room_id, conference_room_id) = tokio::try_join!(event_fut, conference_fut)
        .context("Services requests")
        .error(AppErrorKind::MqttRequestFailed)?;

    Ok((event_room_id, conference_room_id))
}

pub trait Creatable {
    fn id(&self) -> Uuid;
    fn audience(&self) -> &str;
    fn reserve(&self) -> Option<i32>;
    fn tags(&self) -> Option<&serde_json::Value>;
    fn rtc_sharing_policy(&self) -> Option<RtcSharingPolicy>;
}

pub async fn lock_chat(state: &dyn AppContext, event_room_id: Uuid) {
    if let Err(e) = state.event_client().lock_chat(event_room_id).await {
        error!(
            %event_room_id,
            "Failed to lock chat in event room, err = {:?}", e
        );
    }
}
