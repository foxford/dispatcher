use std::ops::Bound;

use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use sqlx::postgres::types::PgRange;
use svc_agent::AccountId;
use uuid::Uuid;

use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub type BoundedDateTimeTuple = (Bound<DateTime<Utc>>, Bound<DateTime<Utc>>);

#[derive(Clone, Debug, sqlx::Type)]
#[sqlx(rename = "class_type", rename_all = "lowercase")]
pub enum ClassType {
    Webinar,
    Classroom,
    Minigroup,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Object {
    id: Uuid,
    #[serde(skip)]
    kind: ClassType,
    scope: String,
    #[serde(with = "serde::time")]
    time: Time,
    audience: String,
    #[serde(with = "ts_seconds")]
    created_at: DateTime<Utc>,
    tags: Option<JsonValue>,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
    preserve_history: bool,
    host: Option<AccountId>,
}

impl Object {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn scope(&self) -> String {
        self.scope.clone()
    }

    pub fn event_room_id(&self) -> Uuid {
        self.event_room_id
    }

    pub fn conference_room_id(&self) -> Uuid {
        self.conference_room_id
    }

    pub fn audience(&self) -> String {
        self.audience.clone()
    }

    pub fn tags(&self) -> Option<JsonValue> {
        self.tags.clone()
    }

    pub fn original_event_room_id(&self) -> Option<Uuid> {
        self.original_event_room_id
    }

    pub fn modified_event_room_id(&self) -> Option<Uuid> {
        self.modified_event_room_id
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, sqlx::Type)]
#[sqlx(transparent)]
#[serde(from = "BoundedDateTimeTuple")]
#[serde(into = "BoundedDateTimeTuple")]
pub struct Time(PgRange<DateTime<Utc>>);

impl From<Time> for BoundedDateTimeTuple {
    fn from(time: Time) -> BoundedDateTimeTuple {
        (time.0.start, time.0.end)
    }
}

impl From<BoundedDateTimeTuple> for Time {
    fn from(time: BoundedDateTimeTuple) -> Time {
        Self(PgRange::from(time))
    }
}

impl Into<PgRange<DateTime<Utc>>> for Time {
    fn into(self) -> PgRange<DateTime<Utc>> {
        self.0
    }
}

pub(crate) mod serde {
    pub(crate) mod time {
        use super::super::Time;
        use crate::serde::ts_seconds_bound_tuple;
        use serde::{de, ser};

        pub(crate) fn serialize<S>(value: &Time, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: ser::Serializer,
        {
            ts_seconds_bound_tuple::serialize(&value.to_owned().into(), serializer)
        }

        #[allow(dead_code)]
        pub(crate) fn deserialize<'de, D>(d: D) -> Result<Time, D::Error>
        where
            D: de::Deserializer<'de>,
        {
            let time = ts_seconds_bound_tuple::deserialize(d)?;
            Ok(Time::from(time))
        }
    }
}

mod classroom;
mod minigroup;
mod webinar;

pub use classroom::*;
pub use minigroup::*;
pub use webinar::*;
