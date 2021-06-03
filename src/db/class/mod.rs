use std::ops::Bound;

use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use sqlx::postgres::{types::PgRange, PgConnection};
use uuid::Uuid;

use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub type BoundedDateTimeTuple = (Bound<DateTime<Utc>>, Bound<DateTime<Utc>>);

#[derive(Clone, Copy, Debug, sqlx::Type)]
#[sqlx(rename = "class_type", rename_all = "lowercase")]
pub enum ClassType {
    Webinar,
    P2P,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<JsonValue>,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_event_room_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_event_room_id: Option<Uuid>,
    preserve_history: bool,
    reserve: Option<i32>,
    room_events_uri: Option<String>,
}

impl Object {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn kind(&self) -> ClassType {
        self.kind
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn event_room_id(&self) -> Uuid {
        self.event_room_id
    }

    pub fn conference_room_id(&self) -> Uuid {
        self.conference_room_id
    }

    pub fn audience(&self) -> &str {
        &self.audience
    }

    pub fn tags(&self) -> Option<&JsonValue> {
        self.tags.as_ref()
    }

    pub fn original_event_room_id(&self) -> Option<Uuid> {
        self.original_event_room_id
    }

    pub fn modified_event_room_id(&self) -> Option<Uuid> {
        self.modified_event_room_id
    }

    pub fn reserve(&self) -> Option<i32> {
        self.reserve
    }

    #[cfg(test)]
    pub fn time(&self) -> &Time {
        &self.time
    }

    pub fn room_events_uri(&self) -> Option<&String> {
        self.room_events_uri.as_ref()
    }
}

////////////////////////////////////////////////////////////////////////////////

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

impl From<Time> for PgRange<DateTime<Utc>> {
    fn from(time: Time) -> Self {
        time.0
    }
}

impl From<&Time> for PgRange<DateTime<Utc>> {
    fn from(time: &Time) -> PgRange<DateTime<Utc>> {
        time.0.clone()
    }
}

////////////////////////////////////////////////////////////////////////////////

enum ReadQueryPredicate {
    Scope { audience: String, scope: String },
    ConferenceRoom(Uuid),
    EventRoom(Uuid),
    Id(Uuid),
}

pub struct ReadQuery {
    condition: ReadQueryPredicate,
}

impl ReadQuery {
    pub fn by_scope(audience: &str, scope: &str) -> Self {
        Self {
            condition: ReadQueryPredicate::Scope {
                audience: audience.to_owned(),
                scope: scope.to_owned(),
            },
        }
    }

    pub fn by_conference_room(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::ConferenceRoom(id),
        }
    }

    pub fn by_event_room(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::EventRoom(id),
        }
    }

    pub fn by_id(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::Id(id),
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        use quaint::ast::{Comparable, Select};
        use quaint::visitor::{Postgres, Visitor};

        let q = Select::from_table("class");

        let q = match self.condition {
            ReadQueryPredicate::Scope { .. } => q
                .and_where("audience".equals("_placeholder_"))
                .and_where("scope".equals("_placeholder_")),
            ReadQueryPredicate::ConferenceRoom(_) => {
                q.and_where("conference_room_id".equals("_placeholder_"))
            }
            ReadQueryPredicate::EventRoom(_) => {
                q.and_where("event_room_id".equals("_placeholder_"))
            }
            ReadQueryPredicate::Id(_) => q.and_where("id".equals("_placeholder_")),
        };

        let (sql, _bindings) = Postgres::build(q);
        let query = sqlx::query_as(&sql);

        let query = match self.condition {
            ReadQueryPredicate::Scope { audience, scope } => query.bind(audience).bind(scope),
            ReadQueryPredicate::ConferenceRoom(id) => query.bind(id),
            ReadQueryPredicate::EventRoom(id) => query.bind(id),
            ReadQueryPredicate::Id(id) => query.bind(id),
        };

        query.fetch_optional(conn).await
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct UpdateDumpEventsQuery {
    modified_event_room_id: Uuid,
    room_events_uri: String,
}

impl UpdateDumpEventsQuery {
    pub fn new(modified_event_room_id: Uuid, room_events_uri: String) -> Self {
        Self {
            modified_event_room_id,
            room_events_uri,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<()> {
        sqlx::query!(
            r"
                UPDATE class
                SET room_events_uri = $1
                WHERE modified_event_room_id = $2
            ",
            self.room_events_uri,
            self.modified_event_room_id,
        )
        .execute(conn)
        .await?;
        Ok(())
    }
}

pub struct UpdateQuery {
    id: Uuid,
    original_event_room_id: Uuid,
    modified_event_room_id: Uuid,
}

impl UpdateQuery {
    pub fn new(id: Uuid, original_event_room_id: Uuid, modified_event_room_id: Uuid) -> Self {
        Self {
            id,
            original_event_room_id,
            modified_event_room_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET original_event_room_id = $2,
                modified_event_room_id = $3
            WHERE id = $1
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                time AS "time!: Time",
                tags,
                preserve_history,
                created_at,
                event_room_id,
                conference_room_id,
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri
            "#,
            self.id,
            self.original_event_room_id,
            self.modified_event_room_id,
        )
        .fetch_one(conn)
        .await
    }
}

////////////////////////////////////////////////////////////////////////////////

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

mod minigroup;
mod p2p;
mod webinar;

pub use minigroup::*;
pub use p2p::*;
pub use webinar::*;
