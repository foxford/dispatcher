use sqlx::postgres::PgConnection;
use uuid::Uuid;

enum AuthzClassQueryState {
    Event(Uuid),
    Conference(Uuid),
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AuthzClass {
    pub kind: String,
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

    pub async fn execute(self, conn: &mut PgConnection) -> sqlx::Result<Option<AuthzClass>> {
        match self.state {
            AuthzClassQueryState::Event(id) => {
                sqlx::query_as!(
                    AuthzClass,
                    r#"
                        SELECT
                            id::text AS "id!: String",
                            (
                                CASE kind
                                    WHEN 'webinar' THEN 'webinars'
                                    WHEN 'classroom' THEN 'classrooms'
                                    ELSE ''
                                END
                            ) AS "kind!: String"
                        FROM class
                        WHERE event_room_id = $1
                            OR original_event_room_id = $1
                            OR modified_event_room_id = $1
                        UNION ALL
                        SELECT
                            id::text AS "id!: String",
                            'chats' AS kind
                        FROM chat
                        WHERE event_room_id = $1
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
                            id::text AS "id!: String",
                            (
                                CASE kind
                                    WHEN 'webinar' THEN 'webinars'
                                    WHEN 'classroom' THEN 'classrooms'
                                    ELSE ''
                                END
                            ) AS "kind!: String"
                        FROM class
                        WHERE conference_room_id = $1
                    "#,
                    id,
                )
                .fetch_optional(conn)
                .await
            }
        }
    }
}
