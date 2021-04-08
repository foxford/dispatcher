use std::ops::Bound;

use chrono::{DateTime, Utc};
use sqlx::postgres::{types::PgRange, PgConnection};
use svc_agent::AgentId;
use uuid::Uuid;

use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct Object {
    id: Uuid,
    class_id: Uuid,
    rtc_id: Uuid,
    stream_uri: String,
    segments: Segments,
    modified_segments: Option<Segments>,
    started_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    adjusted_at: Option<DateTime<Utc>>,
    transcoded_at: Option<DateTime<Utc>>,
    created_by: AgentId,
}

impl Object {
    pub fn stream_uri(&self) -> &str {
        &self.stream_uri
    }

    pub fn rtc_id(&self) -> Uuid {
        self.rtc_id
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    pub fn segments(&self) -> &Segments {
        &self.segments
    }

    pub fn adjusted_at(&self) -> Option<DateTime<Utc>> {
        self.adjusted_at
    }

    pub fn transcoded_at(&self) -> Option<DateTime<Utc>> {
        self.transcoded_at
    }

    pub fn created_by(&self) -> &AgentId {
        &self.created_by
    }
}

pub type BoundedOffsetTuples = Vec<(Bound<i64>, Bound<i64>)>;

#[derive(Clone, Debug, Deserialize, Serialize, sqlx::Type)]
#[sqlx(transparent)]
#[serde(from = "BoundedOffsetTuples")]
#[serde(into = "BoundedOffsetTuples")]
pub struct Segments(Vec<PgRange<i64>>);

impl From<BoundedOffsetTuples> for Segments {
    fn from(segments: BoundedOffsetTuples) -> Self {
        Self(segments.into_iter().map(PgRange::from).collect())
    }
}

impl From<Segments> for BoundedOffsetTuples {
    fn from(segments: Segments) -> Self {
        segments.0.into_iter().map(|s| (s.start, s.end)).collect()
    }
}

impl From<Segments> for Vec<PgRange<i64>> {
    fn from(segments: Segments) -> Self {
        segments.0
    }
}

pub struct RecordingReadQuery {
    class_id: Uuid,
}

impl RecordingReadQuery {
    pub fn new(class_id: Uuid) -> Self {
        Self { class_id }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            SELECT
                id,
                class_id,
                rtc_id,
                stream_uri,
                segments AS "segments!: Segments",
                modified_segments AS "modified_segments!: Option<Segments>",
                started_at,
                created_at,
                adjusted_at,
                transcoded_at,
                created_by AS "created_by: AgentId"
            FROM recording
            WHERE class_id = $1
            "#,
            self.class_id
        )
        .fetch_optional(conn)
        .await
    }
}

pub struct RecordingInsertQuery {
    class_id: Uuid,
    rtc_id: Uuid,
    segments: Segments,
    started_at: DateTime<Utc>,
    stream_uri: String,
    modified_segments: Option<Segments>,
    adjusted_at: Option<DateTime<Utc>>,
    transcoded_at: Option<DateTime<Utc>>,
}

impl RecordingInsertQuery {
    pub fn new(
        class_id: Uuid,
        rtc_id: Uuid,
        segments: Segments,
        started_at: DateTime<Utc>,
        stream_uri: String,
    ) -> Self {
        Self {
            class_id,
            rtc_id,
            segments,
            started_at,
            stream_uri,
            modified_segments: None,
            adjusted_at: None,
            transcoded_at: None,
        }
    }

    #[cfg(test)]
    pub fn modified_segments(self, modified_segments: Segments) -> Self {
        Self {
            modified_segments: Some(modified_segments),
            ..self
        }
    }

    #[cfg(test)]
    pub fn adjusted_at(self, adjusted_at: DateTime<Utc>) -> Self {
        Self {
            adjusted_at: Some(adjusted_at),
            ..self
        }
    }

    #[cfg(test)]
    pub fn transcoded_at(self, transcoded_at: DateTime<Utc>) -> Self {
        Self {
            transcoded_at: Some(transcoded_at),
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO recording (
                class_id, rtc_id, stream_uri, segments, modified_segments, started_at, adjusted_at,
                transcoded_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING
                id,
                class_id,
                rtc_id,
                stream_uri,
                segments AS "segments!: Segments",
                started_at,
                modified_segments AS "modified_segments!: Option<Segments>",
                created_at,
                adjusted_at,
                transcoded_at,
                created_by AS "created_by: AgentId"
            "#,
            self.class_id,
            self.rtc_id,
            self.stream_uri,
            self.segments as Segments,
            self.modified_segments as Option<Segments>,
            self.started_at,
            self.adjusted_at,
            self.transcoded_at
        )
        .fetch_one(conn)
        .await
    }
}

pub struct AdjustWebinarUpdateQuery {
    webinar_id: Uuid,
    modified_segments: Segments,
}

impl AdjustWebinarUpdateQuery {
    pub fn new(webinar_id: Uuid, modified_segments: Segments) -> Self {
        Self {
            webinar_id,
            modified_segments,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE recording
            SET modified_segments = $2,
                adjusted_at = NOW()
            WHERE class_id = $1
            RETURNING
                id,
                class_id,
                rtc_id,
                stream_uri,
                segments AS "segments!: Segments",
                started_at,
                modified_segments AS "modified_segments!: Option<Segments>",
                created_at,
                adjusted_at,
                transcoded_at,
                created_by AS "created_by: AgentId"
            "#,
            self.webinar_id,
            self.modified_segments as Segments,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct AdjustMinigroupUpdateQuery {
    minigroup_id: Uuid,
}

impl AdjustMinigroupUpdateQuery {
    pub fn new(minigroup_id: Uuid) -> Self {
        Self { minigroup_id }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Vec<Object>> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE recording
            SET modified_segments = segments,
                adjusted_at = NOW()
            WHERE class_id = $1
            RETURNING
                id,
                class_id,
                rtc_id,
                stream_uri,
                segments AS "segments!: Segments",
                started_at,
                modified_segments AS "modified_segments!: Option<Segments>",
                created_at,
                adjusted_at,
                transcoded_at,
                created_by AS "created_by: AgentId"
            "#,
            self.minigroup_id,
        )
        .fetch_all(conn)
        .await
    }
}

pub struct TranscodingUpdateQuery {
    class_id: Uuid,
}

impl TranscodingUpdateQuery {
    pub fn new(class_id: Uuid) -> Self {
        Self { class_id }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            UPDATE recording
            SET transcoded_at = NOW()
            WHERE class_id = $1
            RETURNING
                id,
                class_id,
                rtc_id,
                stream_uri,
                segments AS "segments!: Segments",
                started_at,
                modified_segments AS "modified_segments!: Option<Segments>",
                created_at,
                adjusted_at,
                transcoded_at,
                created_by AS "created_by: AgentId"
            "#,
            self.class_id,
        )
        .fetch_one(conn)
        .await
    }
}

pub struct RecordingConvertInsertQuery {
    class_id: Uuid,
    rtc_id: Uuid,
    segments: Segments,
    modified_segments: Segments,
    stream_uri: String,
}

impl RecordingConvertInsertQuery {
    pub fn new(
        class_id: Uuid,
        rtc_id: Uuid,
        segments: Segments,
        modified_segments: Segments,
        stream_uri: String,
    ) -> Self {
        Self {
            class_id,
            rtc_id,
            segments,
            modified_segments,
            stream_uri,
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Object> {
        sqlx::query_as!(
            Object,
            r#"
            INSERT INTO recording (class_id, rtc_id, segments, modified_segments, stream_uri, started_at, adjusted_at, transcoded_at)
            VALUES ($1, $2, $3, $4, $5, NOW(), NOW(), NOW())
            RETURNING
                id,
                class_id,
                rtc_id,
                stream_uri,
                segments AS "segments!: Segments",
                started_at,
                modified_segments AS "modified_segments!: Option<Segments>",
                created_at,
                adjusted_at,
                transcoded_at,
                created_by AS "created_by: AgentId"
            "#,
            self.class_id,
            self.rtc_id,
            self.segments as Segments,
            self.modified_segments as Segments,
            self.stream_uri,

        )
        .fetch_one(conn)
        .await
    }
}

pub(crate) mod serde {
    pub(crate) mod segments {
        use super::super::{BoundedOffsetTuples, Segments};
        use crate::serde::milliseconds_bound_tuples;
        use serde::{de, ser};

        pub(crate) fn serialize<S>(value: &Segments, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: ser::Serializer,
        {
            let bounded_offset_tuples: BoundedOffsetTuples = value.to_owned().into();
            milliseconds_bound_tuples::serialize(&bounded_offset_tuples, serializer)
        }

        pub(crate) fn deserialize<'de, D>(d: D) -> Result<Segments, D::Error>
        where
            D: de::Deserializer<'de>,
        {
            milliseconds_bound_tuples::deserialize(d).map(Segments::from)
        }
    }

    pub(crate) fn segments_option<S>(
        opt: &Option<super::Segments>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        match opt {
            Some(value) => segments::serialize(value, serializer),
            None => serializer.serialize_none(),
        }
    }
}
