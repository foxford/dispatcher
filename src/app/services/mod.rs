use std::ops::Bound;

use anyhow::Context;
use chrono::Utc;
use tracing::error;
use uuid::Uuid;

use crate::app::api::v1::AppError;
use crate::app::error::ErrorExt;
use crate::app::error::ErrorKind as AppErrorKind;
use crate::app::AppContext;
use crate::clients::event::{EventRoomCreatePayload, LockedTypes};
use crate::clients::{
    conference::RoomUpdate as ConfRoomUpdate, event::RoomUpdate as EventRoomUpdate,
};
use crate::db::class::{BoundedDateTimeTuple, ClassType, RtcSharingPolicy};

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

pub async fn create_event_room(
    state: &dyn AppContext,
    webinar: &impl Creatable,
) -> Result<Uuid, AppError> {
    state
        .event_client()
        .create_room(EventRoomCreatePayload {
            audience: webinar.audience().to_owned(),
            time: (Bound::Included(Utc::now()), Bound::Unbounded),
            preserve_history: Some(true),
            tags: webinar.tags().map(ToOwned::to_owned),
            classroom_id: Some(webinar.id()),
            kind: webinar.kind(),
        })
        .await
        .context("Request to event")
        .error(AppErrorKind::MqttRequestFailed)
}

pub async fn create_conference_room(
    state: &dyn AppContext,
    webinar: &impl Creatable,
    time: &BoundedDateTimeTuple,
) -> Result<Uuid, AppError> {
    let conference_time = match time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), Bound::Unbounded),
        Bound::Unbounded => (Bound::Included(Utc::now()), Bound::Unbounded),
    };

    let policy = webinar
        .rtc_sharing_policy()
        .as_ref()
        .map(ToString::to_string);

    state
        .conference_client()
        .create_room(
            conference_time,
            webinar.audience().to_owned(),
            policy,
            webinar.reserve(),
            webinar.tags().map(ToOwned::to_owned),
            Some(webinar.id()),
        )
        .await
        .context("Request to conference")
        .error(AppErrorKind::MqttRequestFailed)
}

pub async fn create_event_and_conference_rooms(
    state: &dyn AppContext,
    webinar: &impl Creatable,
    time: &BoundedDateTimeTuple,
) -> Result<(Uuid, Uuid), AppError> {
    let event_room_id = create_event_room(state, webinar).await?;
    let conference_room_id = create_conference_room(state, webinar, time).await?;

    Ok((event_room_id, conference_room_id))
}

pub trait Creatable {
    fn id(&self) -> Uuid;
    fn audience(&self) -> &str;
    fn reserve(&self) -> Option<i32>;
    fn tags(&self) -> Option<&serde_json::Value>;
    fn rtc_sharing_policy(&self) -> Option<RtcSharingPolicy>;
    fn kind(&self) -> ClassType;
}

pub async fn lock_interaction(state: &dyn AppContext, event_room_id: Uuid, types: LockedTypes) {
    if let Err(e) = state
        .event_client()
        .update_locked_types(event_room_id, types)
        .await
    {
        error!(
            %event_room_id,
            "Failed to lock interaction in event room, err = {:?}", e
        );
    }
}
