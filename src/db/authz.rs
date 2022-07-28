use sqlx::postgres::PgConnection;
use uuid::Uuid;

enum AuthzClassQueryState {
    Event(Uuid),
    Id(Uuid),
    Conference(Uuid),
    RecordingRtcId(Uuid),
    Scope { audience: String, scope: String },
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AuthzClass {
    pub id: String,
}

pub struct AuthzReadQuery {
    state: AuthzClassQueryState,
}

impl AuthzReadQuery {
    pub fn by_event(id: Uuid) -> Self {
        Self {
            state: AuthzClassQueryState::Event(id),
        }
    }

    pub fn by_conference(id: Uuid) -> Self {
        Self {
            state: AuthzClassQueryState::Conference(id),
        }
    }

    pub fn by_id(id: Uuid) -> Self {
        Self {
            state: AuthzClassQueryState::Id(id),
        }
    }

    pub fn by_rtc_id(id: Uuid) -> Self {
        Self {
            state: AuthzClassQueryState::RecordingRtcId(id),
        }
    }

    pub fn by_scope(audience: String, scope: String) -> Self {
        Self {
            state: AuthzClassQueryState::Scope { audience, scope },
        }
    }

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<AuthzClass>> {
        match self.state {
            AuthzClassQueryState::Event(id) => {
                sqlx::query_as!(
                    AuthzClass,
                    r#"
                        SELECT
                            id::text AS "id!: String"
                        FROM class
                        WHERE event_room_id = $1
                            OR original_event_room_id = $1
                            OR modified_event_room_id = $1
                    "#,
                    id,
                )
                .fetch_optional(conn)
                .await
            }
            AuthzClassQueryState::Conference(id) => {
                sqlx::query_as!(
                    AuthzClass,
                    r#"
                        SELECT
                            id::text AS "id!: String"
                        FROM class
                        WHERE conference_room_id = $1
                    "#,
                    id,
                )
                .fetch_optional(conn)
                .await
            }
            AuthzClassQueryState::RecordingRtcId(id) => {
                sqlx::query_as!(
                    AuthzClass,
                    r#"
                        SELECT
                            class.id::text AS "id!: String"
                        FROM class
                        INNER JOIN recording r
                        ON r.class_id = class.id
                        WHERE rtc_id = $1
                    "#,
                    id,
                )
                .fetch_optional(conn)
                .await
            }
            AuthzClassQueryState::Scope { scope, audience } => {
                sqlx::query_as!(
                    AuthzClass,
                    r#"
                        SELECT
                            id::text AS "id!: String"
                        FROM class
                        WHERE audience = $1
                        AND scope = $2
                    "#,
                    audience,
                    scope
                )
                .fetch_optional(conn)
                .await
            }
            AuthzClassQueryState::Id(id) => {
                sqlx::query_as!(
                    AuthzClass,
                    r#"
                        SELECT
                            id::text AS "id!: String"
                        FROM class
                        WHERE id = $1
                    "#,
                    id,
                )
                .fetch_optional(conn)
                .await
            }
        }
    }
}
