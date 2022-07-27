use super::*;

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Dummy {
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
    properties: ClassProperties,
    preserve_history: bool,
    reserve: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_class_id: Option<Uuid>,
    content_id: String,
}

impl Dummy {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn audience(&self) -> &str {
        &self.audience
    }

    #[cfg(test)]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn tags(&self) -> Option<&JsonValue> {
        self.tags.as_ref()
    }

    pub fn reserve(&self) -> Option<i32> {
        self.reserve
    }

    pub fn rtc_sharing_policy(&self) -> Option<RtcSharingPolicy> {
        self.kind.into()
    }

    pub fn time(&self) -> Time {
        self.time.clone()
    }
}

impl crate::app::services::Creatable for Dummy {
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
}

pub struct InsertQuery {
    kind: ClassType,
    scope: String,
    audience: String,
    time: Time,
    tags: Option<JsonValue>,
    properties: Option<ClassProperties>,
    preserve_history: bool,
    conference_room_id: Option<Uuid>,
    event_room_id: Option<Uuid>,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
    reserve: Option<i32>,
    room_events_uri: Option<String>,
    established: bool,
    original_class_id: Option<Uuid>,
    content_id: String,
}

impl InsertQuery {
    pub fn new(kind: ClassType, scope: String, audience: String, time: Time) -> Self {
        Self {
            kind,
            scope: scope.clone(),
            audience,
            time,
            tags: None,
            properties: None,
            preserve_history: true,
            conference_room_id: None,
            event_room_id: None,
            original_event_room_id: None,
            modified_event_room_id: None,
            reserve: None,
            room_events_uri: None,
            established: false,
            original_class_id: None,
            content_id: scope,
        }
    }

    pub fn tags(self, tags: JsonValue) -> Self {
        Self {
            tags: Some(tags),
            ..self
        }
    }

    pub fn properties(self, properties: ClassProperties) -> Self {
        Self {
            properties: Some(properties),
            ..self
        }
    }

    pub fn reserve(self, reserve: i32) -> Self {
        Self {
            reserve: Some(reserve),
            ..self
        }
    }

    pub fn preserve_history(self, preserve_history: bool) -> Self {
        Self {
            preserve_history,
            ..self
        }
    }

    pub fn original_class_id(self, class_id: Uuid) -> Self {
        Self {
            original_class_id: Some(class_id),
            ..self
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Dummy> {
        let time: PgRange<DateTime<Utc>> = self.time.into();

        sqlx::query_as!(
            Dummy,
            r#"
            INSERT INTO class (
                scope, audience, time, tags, preserve_history, kind,
                conference_room_id, event_room_id,
                original_event_room_id, modified_event_room_id, reserve, room_events_uri,
                established, properties, original_class_id, content_id
            )
            VALUES ($1, $2, $3, $4, $5, $6::class_type, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT (scope, audience)
            DO UPDATE
            SET time = EXCLUDED.time,
                tags = EXCLUDED.tags,
                preserve_history = EXCLUDED.preserve_history,
                reserve = EXCLUDED.reserve,
                properties = EXCLUDED.properties
            WHERE class.established = 'f'
            RETURNING
                id,
                kind AS "kind!: ClassType",
                scope,
                time AS "time!: Time",
                audience,
                created_at,
                tags,
                preserve_history,
                reserve,
                properties AS "properties: _",
                original_class_id,
                content_id
            "#,
            self.scope,
            self.audience,
            time,
            self.tags,
            self.preserve_history,
            self.kind as ClassType,
            self.conference_room_id,
            self.event_room_id,
            self.original_event_room_id,
            self.modified_event_room_id,
            self.reserve,
            self.room_events_uri,
            self.established,
            self.properties.unwrap_or_default() as ClassProperties,
            self.original_class_id,
            self.content_id
        )
        .fetch_one(conn)
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::SubsecRound;
    use chrono::Utc;

    use super::*;
    use crate::test_helpers::prelude::*;

    #[tokio::test]
    async fn insert_already_established_webinar() {
        let db = TestDb::new().await;
        let mut conn = db.get_conn().await;

        let webinar = {
            factory::Webinar::new(
                random_string(),
                USR_AUDIENCE.to_string(),
                (Bound::Unbounded, Bound::Unbounded).into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .insert(&mut conn)
            .await
        };

        let t = Utc::now().trunc_subsecs(0);

        InsertQuery::new(
            ClassType::Webinar,
            webinar.scope().to_owned(),
            webinar.audience().to_owned(),
            (Bound::Included(t), Bound::Unbounded).into(),
        )
        .execute(&mut conn)
        .await
        .expect_err("Should conflict with already existing webinar");

        let w = WebinarReadQuery::by_id(webinar.id())
            .execute(&mut conn)
            .await
            .unwrap()
            .unwrap();

        let time: BoundedDateTimeTuple = w.time().clone().into();
        assert_eq!(time.0, Bound::Unbounded);
    }

    #[tokio::test]
    async fn insert_not_established_webinar() {
        let db = TestDb::new().await;
        let mut conn = db.get_conn().await;

        let dummy = InsertQuery::new(
            ClassType::Webinar,
            random_string(),
            USR_AUDIENCE.to_string(),
            (Bound::Unbounded, Bound::Unbounded).into(),
        )
        .execute(&mut conn)
        .await
        .unwrap();

        let t = Utc::now().trunc_subsecs(0);

        let r = InsertQuery::new(
            ClassType::Webinar,
            dummy.scope().to_owned(),
            dummy.audience().to_owned(),
            (Bound::Included(t), Bound::Unbounded).into(),
        )
        .execute(&mut conn)
        .await
        .expect("Should be ok");

        let time: BoundedDateTimeTuple = r.time.into();
        assert_eq!(time.0, Bound::Included(t));
    }
}
