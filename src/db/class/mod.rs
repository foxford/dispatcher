use std::{marker::PhantomData, ops::Bound};

use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use sqlx::postgres::{types::PgRange, PgConnection};
use svc_agent::AgentId;
use uuid::Uuid;

use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub type BoundedDateTimeTuple = (Bound<DateTime<Utc>>, Bound<DateTime<Utc>>);

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct KeyValueProperties(serde_json::Map<String, JsonValue>);

impl KeyValueProperties {
    pub fn new() -> Self {
        Self(serde_json::Map::new())
    }

    pub fn into_json(self) -> JsonValue {
        JsonValue::Object(self.0)
    }
}

impl From<serde_json::Map<String, JsonValue>> for KeyValueProperties {
    fn from(map: serde_json::Map<String, JsonValue>) -> Self {
        Self(map)
    }
}

impl std::ops::Deref for KeyValueProperties {
    type Target = serde_json::Map<String, JsonValue>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for KeyValueProperties {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl sqlx::Encode<'_, sqlx::Postgres> for KeyValueProperties {
    fn encode_by_ref(
        &self,
        buf: &mut <sqlx::Postgres as sqlx::database::HasArguments>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
        self.clone().into_json().encode_by_ref(buf)
    }
}

impl sqlx::Decode<'_, sqlx::Postgres> for KeyValueProperties {
    fn decode(
        value: <sqlx::Postgres as sqlx::database::HasValueRef<'_>>::ValueRef,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let raw_value = serde_json::Value::decode(value)?;
        match raw_value {
            JsonValue::Object(map) => Ok(map.into()),
            _ => Err("failed to decode jsonb value as json object"
                .to_owned()
                .into()),
        }
    }
}

impl sqlx::Type<sqlx::Postgres> for KeyValueProperties {
    fn type_info() -> <sqlx::Postgres as sqlx::Database>::TypeInfo {
        <JsonValue as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

#[derive(Clone, Copy, Debug, sqlx::Type, PartialEq, Eq, Serialize)]
#[sqlx(type_name = "class_type", rename_all = "lowercase")]
pub enum ClassType {
    Webinar,
    P2P,
    Minigroup,
}

#[derive(Clone, Copy, Debug)]
pub enum RtcSharingPolicy {
    Shared,
    Owned,
}

impl ToString for RtcSharingPolicy {
    fn to_string(&self) -> String {
        match self {
            RtcSharingPolicy::Shared => "shared".to_string(),
            RtcSharingPolicy::Owned => "owned".to_string(),
        }
    }
}

impl From<ClassType> for Option<RtcSharingPolicy> {
    fn from(v: ClassType) -> Self {
        match v {
            ClassType::Webinar => Some(RtcSharingPolicy::Shared),
            ClassType::Minigroup => Some(RtcSharingPolicy::Owned),
            ClassType::P2P => None,
        }
    }
}

pub struct WebinarType;
pub struct P2PType;
pub struct MinigroupType;

pub trait AsClassType {
    fn as_class_type() -> ClassType;
    fn as_str() -> &'static str;
}

impl AsClassType for WebinarType {
    fn as_class_type() -> ClassType {
        ClassType::Webinar
    }

    fn as_str() -> &'static str {
        "webinar"
    }
}

impl AsClassType for P2PType {
    fn as_class_type() -> ClassType {
        ClassType::P2P
    }

    fn as_str() -> &'static str {
        "p2p"
    }
}

impl AsClassType for MinigroupType {
    fn as_class_type() -> ClassType {
        ClassType::Minigroup
    }

    fn as_str() -> &'static str {
        "minigroup"
    }
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
    properties: KeyValueProperties,
    conference_room_id: Uuid,
    event_room_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_event_room_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_event_room_id: Option<Uuid>,
    preserve_history: bool,
    reserve: Option<i32>,
    room_events_uri: Option<String>,
    host: Option<AgentId>,
    timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_class_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

pub fn default_locked_chat() -> bool {
    true
}

pub fn default_locked_questions() -> bool {
    true
}

pub fn default_whiteboard() -> bool {
    true
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

    pub fn properties(&self) -> &KeyValueProperties {
        &self.properties
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

    pub fn time(&self) -> &Time {
        &self.time
    }

    pub fn room_events_uri(&self) -> Option<&String> {
        self.room_events_uri.as_ref()
    }

    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    #[cfg(test)]
    pub fn host(&self) -> Option<&AgentId> {
        self.host.as_ref()
    }

    pub fn rtc_sharing_policy(&self) -> Option<RtcSharingPolicy> {
        self.kind.into()
    }

    pub fn original_class_id(&self) -> Option<Uuid> {
        self.original_class_id
    }

    pub fn content_id(&self) -> Option<&String> {
        self.content_id.as_ref()
    }
}

impl crate::app::services::Creatable for Object {
    fn id(&self) -> Uuid {
        self.id()
    }

    fn audience(&self) -> &str {
        self.audience()
    }
    fn reserve(&self) -> Option<i32> {
        self.reserve()
    }
    fn tags(&self) -> Option<&serde_json::Value> {
        self.tags()
    }
    fn rtc_sharing_policy(&self) -> Option<RtcSharingPolicy> {
        self.rtc_sharing_policy()
    }

    fn kind(&self) -> ClassType {
        self.kind
    }
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct WrongKind {
    id: Uuid,
    source: ClassType,
    destination: ClassType,
}

impl WrongKind {
    fn new(value: &Object, destination: ClassType) -> Self {
        Self {
            id: value.id(),
            source: value.kind(),
            destination,
        }
    }
}

impl std::fmt::Display for WrongKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Error converting class = {} of type {:?} to {:?}",
            self.id, self.source, self.destination
        )
    }
}

impl std::error::Error for WrongKind {}

////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, sqlx::Type)]
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

impl Time {
    pub fn end(&self) -> Option<&DateTime<Utc>> {
        use std::ops::RangeBounds;
        match self.0.end_bound() {
            Bound::Included(t) => Some(t),
            Bound::Excluded(t) => Some(t),
            Bound::Unbounded => None,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

enum ReadQueryPredicate {
    Id(Uuid),
    Scope { audience: String, scope: String },
    ConferenceRoom(Uuid),
    EventRoom(Uuid),
    OriginalEventRoom(Uuid),
}

pub struct ReadQuery {
    condition: ReadQueryPredicate,
    original: bool,
}

impl ReadQuery {
    pub fn by_scope(audience: &str, scope: &str) -> Self {
        Self {
            condition: ReadQueryPredicate::Scope {
                audience: audience.to_owned(),
                scope: scope.to_owned(),
            },
            original: false,
        }
    }

    pub fn by_conference_room(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::ConferenceRoom(id),
            original: false,
        }
    }

    pub fn by_event_room(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::EventRoom(id),
            original: false,
        }
    }

    pub fn by_original_event_room(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::OriginalEventRoom(id),
            original: false,
        }
    }

    pub fn by_id(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::Id(id),
            original: false,
        }
    }

    pub fn original(self) -> Self {
        Self {
            original: true,
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        use quaint::ast::{Comparable, Select};
        use quaint::visitor::{Postgres, Visitor};

        let q = Select::from_table("class");

        let mut q = match self.condition {
            ReadQueryPredicate::Id(_) => q.and_where("id".equals("_placeholder_")),
            ReadQueryPredicate::Scope { .. } => q
                .and_where("audience".equals("_placeholder_"))
                .and_where("scope".equals("_placeholder_")),
            ReadQueryPredicate::ConferenceRoom(_) => {
                q.and_where("conference_room_id".equals("_placeholder_"))
            }
            ReadQueryPredicate::EventRoom(_) => {
                q.and_where("event_room_id".equals("_placeholder_"))
            }
            ReadQueryPredicate::OriginalEventRoom(_) => {
                q.and_where("original_event_room_id".equals("_placeholder_"))
            }
        };

        if self.original {
            q = q.and_where("original_class_id".is_null())
        }

        let (sql, _bindings) = Postgres::build(q);
        let query = sqlx::query_as(&sql);

        let query = match self.condition {
            ReadQueryPredicate::Id(id) => query.bind(id),
            ReadQueryPredicate::Scope { audience, scope } => query.bind(audience).bind(scope),
            ReadQueryPredicate::ConferenceRoom(id) => query.bind(id),
            ReadQueryPredicate::EventRoom(id) => query.bind(id),
            ReadQueryPredicate::OriginalEventRoom(id) => query.bind(id),
        };

        query.fetch_optional(conn).await
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct GenericReadQuery<T: AsClassType> {
    condition: ReadQueryPredicate,
    class_type: ClassType,
    phantom: PhantomData<T>,
}

impl<T: AsClassType> GenericReadQuery<T> {
    pub fn by_id(id: Uuid) -> Self {
        Self {
            condition: ReadQueryPredicate::Id(id),
            class_type: T::as_class_type(),
            phantom: PhantomData,
        }
    }

    pub fn by_scope(audience: &str, scope: &str) -> Self {
        Self {
            condition: ReadQueryPredicate::Scope {
                audience: audience.to_owned(),
                scope: scope.to_owned(),
            },
            class_type: T::as_class_type(),
            phantom: PhantomData,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        use quaint::ast::{Comparable, Select};
        use quaint::visitor::{Postgres, Visitor};

        let q = Select::from_table("class");

        let q = match self.condition {
            ReadQueryPredicate::Id(_) => q.and_where("id".equals("_placeholder_")),
            ReadQueryPredicate::Scope { .. } => q
                .and_where("audience".equals("_placeholder_"))
                .and_where("scope".equals("_placeholder_")),
            ReadQueryPredicate::ConferenceRoom(_) => {
                q.and_where("conference_room_id".equals("_placeholder_"))
            }
            ReadQueryPredicate::EventRoom(_) => {
                q.and_where("event_room_id".equals("_placeholder_"))
            }
            ReadQueryPredicate::OriginalEventRoom(_) => {
                q.and_where("original_event_room_id".equals("_placeholder_"))
            }
        };

        let q = q.and_where("kind".equals("_placeholder_"));

        let (sql, _bindings) = Postgres::build(q);
        let query = sqlx::query_as(&sql);

        let query = match self.condition {
            ReadQueryPredicate::Id(id) => query.bind(id),
            ReadQueryPredicate::Scope { audience, scope } => query.bind(audience).bind(scope),
            ReadQueryPredicate::ConferenceRoom(id) => query.bind(id),
            ReadQueryPredicate::EventRoom(id) => query.bind(id),
            ReadQueryPredicate::OriginalEventRoom(id) => query.bind(id),
        };

        let query = query.bind(self.class_type);

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

pub struct UpdateAdjustedRoomsQuery {
    id: Uuid,
    original_event_room_id: Uuid,
    modified_event_room_id: Uuid,
}

impl UpdateAdjustedRoomsQuery {
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
                properties AS "properties: _",
                preserve_history,
                created_at,
                event_room_id AS "event_room_id!: Uuid",
                conference_room_id AS "conference_room_id!: Uuid",
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri,
                host AS "host: AgentId",
                timed_out,
                original_class_id,
                content_id
            "#,
            self.id,
            self.original_event_room_id,
            self.modified_event_room_id,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct EstablishQuery {
    id: Uuid,
    event_room_id: Uuid,
    conference_room_id: Uuid,
}

impl EstablishQuery {
    pub fn new(id: Uuid, event_room_id: Uuid, conference_room_id: Uuid) -> Self {
        Self {
            id,
            event_room_id,
            conference_room_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET event_room_id = $2,
                conference_room_id = $3,
                established = 't'
            WHERE id = $1
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                time AS "time!: Time",
                tags,
                properties AS "properties: _",
                preserve_history,
                created_at,
                event_room_id AS "event_room_id!: Uuid",
                conference_room_id AS "conference_room_id!: Uuid",
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri,
                host AS "host: AgentId",
                timed_out,
                original_class_id,
                content_id
            "#,
            self.id,
            self.event_room_id,
            self.conference_room_id,
        )
        .fetch_one(conn)
        .await
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct RecreateQuery {
    id: Uuid,
    time: Time,
    event_room_id: Uuid,
    conference_room_id: Uuid,
}

impl RecreateQuery {
    pub fn new(id: Uuid, time: Time, event_room_id: Uuid, conference_room_id: Uuid) -> Self {
        Self {
            id,
            time,
            event_room_id,
            conference_room_id,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        let time: PgRange<DateTime<Utc>> = self.time.into();

        sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET time = $2,
                event_room_id = $3,
                conference_room_id = $4,
                original_event_room_id = NULL,
                modified_event_room_id = NULL
            WHERE id = $1
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                time AS "time!: Time",
                tags,
                properties AS "properties: _",
                preserve_history,
                created_at,
                event_room_id AS "event_room_id!: Uuid",
                conference_room_id AS "conference_room_id!: Uuid",
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri,
                host AS "host: AgentId",
                timed_out,
                original_class_id,
                content_id
            "#,
            self.id,
            time,
            self.event_room_id,
            Some(self.conference_room_id),
        )
        .fetch_one(conn)
        .await
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct ClassUpdateQuery {
    id: Uuid,
    time: Option<Time>,
    reserve: Option<i32>,
    host: Option<AgentId>,
    properties: Option<KeyValueProperties>,
}

impl ClassUpdateQuery {
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            time: None,
            reserve: None,
            host: None,
            properties: None,
        }
    }

    pub fn time(mut self, time: Time) -> Self {
        self.time = Some(time);
        self
    }

    pub fn reserve(mut self, reserve: i32) -> Self {
        self.reserve = Some(reserve);
        self
    }

    pub fn host(mut self, host: AgentId) -> Self {
        self.host = Some(host);
        self
    }

    pub fn properties(self, properties: KeyValueProperties) -> Self {
        Self {
            properties: Some(properties),
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        let time: Option<PgRange<DateTime<Utc>>> = self.time.map(Into::into);
        let query = sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET
                time = COALESCE($2, time),
                reserve = COALESCE($3, reserve),
                host = COALESCE($4, host),
                properties = COALESCE($5, properties)
            WHERE id = $1
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                time AS "time!: Time",
                tags,
                properties AS "properties!: KeyValueProperties",
                preserve_history,
                created_at,
                event_room_id AS "event_room_id!: Uuid",
                conference_room_id AS "conference_room_id!: Uuid",
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri,
                host AS "host: AgentId",
                timed_out,
                original_class_id,
                content_id
            "#,
            self.id,
            time,
            self.reserve,
            self.host as Option<AgentId>,
            self.properties.unwrap_or_default() as KeyValueProperties,
        );

        query.fetch_one(conn).await
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct RoomCloseQuery {
    id: Uuid,
    timed_out: bool,
}

impl RoomCloseQuery {
    pub fn new(id: Uuid, timed_out: bool) -> Self {
        Self { id, timed_out }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE class
            SET time = TSTZRANGE(LOWER(time),
                LEAST(UPPER(time), NOW())),
                timed_out = $2
            WHERE id = $1
            RETURNING
                id,
                scope,
                kind AS "kind!: ClassType",
                audience,
                time AS "time!: Time",
                tags,
                properties AS "properties: _",
                preserve_history,
                created_at,
                event_room_id AS "event_room_id!: Uuid",
                conference_room_id AS "conference_room_id!: Uuid",
                original_event_room_id,
                modified_event_room_id,
                reserve,
                room_events_uri,
                host AS "host: AgentId",
                timed_out,
                original_class_id,
                content_id
            "#,
            self.id,
            self.timed_out
        )
        .fetch_one(conn)
        .await
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct DeleteQuery {
    id: Uuid,
}

impl DeleteQuery {
    pub fn new(id: Uuid) -> Self {
        Self { id }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<u64> {
        sqlx::query_as!(
            Object,
            r#"
            DELETE FROM class
            WHERE id = $1
            "#,
            self.id,
        )
        .execute(conn)
        .await
        .map(|r| r.rows_affected())
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

mod insert_query;
mod minigroup;
mod p2p;
mod webinar;

pub use insert_query::{Dummy, InsertQuery};
pub use minigroup::*;
pub use p2p::*;
pub use webinar::*;
