use std::ops::Bound;
use std::str::FromStr;
use std::sync::Arc;

use async_std::prelude::FutureExt;
use chrono::Utc;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::Acquire;
use tide::{http::Error as HttpError, Request, Response};
use uuid::Uuid;

use crate::app::authz::AuthzObject;
use crate::app::AppContext;
use crate::db::class::Object as Class;
use crate::db::recording::Segments;

#[derive(Serialize)]
struct WebinarObject {
    id: String,
    real_time: RealTimeObject,
    on_demand: Vec<WebinarVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

impl WebinarObject {
    pub fn add_version(&mut self, version: WebinarVersion) {
        self.on_demand.push(version);
    }

    pub fn set_status(&mut self, status: &str) {
        self.status = Some(status.to_owned());
    }
}

#[derive(Serialize)]
struct WebinarVersion {
    version: &'static str,
    event_room_id: Uuid,
    stream_id: Uuid,
    tags: Option<JsonValue>,
}

#[derive(Serialize)]
struct RealTimeObject {
    conference_room_id: Uuid,
    event_room_id: Uuid,
    fallback_uri: Option<String>,
}

impl From<Class> for WebinarObject {
    fn from(obj: Class) -> WebinarObject {
        WebinarObject {
            id: obj.scope(),
            real_time: RealTimeObject {
                fallback_uri: None,
                conference_room_id: obj.conference_room_id(),
                event_room_id: obj.event_room_id(),
            },
            on_demand: vec![],
            status: None,
        }
    }
}

pub async fn read(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let maybe_token = fetch_token(&req);
    let state = req.state();
    let account_id = match state.validate_token(maybe_token.as_deref()) {
        Ok(id) => id,
        Err(e) => {
            error!(
                crate::LOG,
                "Failed to process Authorization header, header = {:?}, err = {:?}",
                req.header("Authorization"),
                e
            );
            return Ok(access_denied_response());
        }
    };

    let webinar = find_webinar(&req).await?;

    let object = AuthzObject::new(&["webinars", &webinar.id().to_string()]).into();

    if let Err(err) = state
        .authz()
        .authorize(
            webinar.audience(),
            account_id.clone(),
            object,
            "read".into(),
        )
        .await
    {
        error!(crate::LOG, "Failed to authorize action, reason = {:?}", err);
        return Ok(access_denied_response());
    }

    let mut conn = req.state().get_conn().await?;
    let recording = crate::db::recording::RecordingReadQuery::new(webinar.id())
        .execute(&mut conn)
        .await
        .map_err(|e| HttpError::new(500, e))?;

    let mut webinar_obj: WebinarObject = webinar.clone().into();

    if let Some(recording) = recording {
        if let Some(og_event_id) = webinar.original_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "original",
                stream_id: recording.rtc_id(),
                event_room_id: og_event_id,
                tags: webinar.tags(),
            });
        }

        if let Some(md_event_id) = webinar.modified_event_room_id() {
            webinar_obj.add_version(WebinarVersion {
                version: "modified",
                stream_id: recording.rtc_id(),
                event_room_id: md_event_id,
                tags: webinar.tags(),
            });
        }
        if recording.transcoded_at().is_some() {
            webinar_obj.set_status("transcoded");
        } else if recording.adjusted_at().is_some() {
            webinar_obj.set_status("adjusted");
        } else {
            webinar_obj.set_status("finished");
        }
    } else {
        webinar_obj.set_status("real-time");
    }

    let body = serde_json::to_string(&webinar_obj).map_err(|e| HttpError::new(500, e))?;
    let response = Response::builder(200).body(body).build();
    Ok(response)
}

#[derive(Deserialize)]
struct Webinar {
    title: String,
    scope: String,
    audience: String,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: crate::db::class::BoundedDateTimeTuple,
    tags: Option<serde_json::Value>,
    preserve_history: Option<bool>,
    reserve: Option<i32>,
    backend: Option<String>,
    #[serde(default)]
    locked_chat: bool,
}

lazy_static! {
    static ref CONFERENCE_PRESTART_SIGNALING_WINDOW: chrono::Duration =
        chrono::Duration::minutes(10);
}

pub async fn create(mut req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let body: Webinar = req.body_json().await?;

    let maybe_token = fetch_token(&req);
    let state = req.state();
    let account_id = match state.validate_token(maybe_token.as_deref()) {
        Ok(id) => id,
        Err(e) => {
            error!(
                crate::LOG,
                "Failed to process Authorization header, header = {:?}, err = {:?}",
                req.header("Authorization"),
                e
            );
            return Ok(access_denied_response());
        }
    };

    let object = AuthzObject::new(&["webinars"]).into();

    if let Err(err) = state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "create".into(),
        )
        .await
    {
        error!(crate::LOG, "Failed to authorize action, reason = {:?}", err);
        return Ok(access_denied_response());
    }

    let conference_time = match body.time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), Bound::Unbounded),
        Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
    };
    let conference_fut = req.state().conference_client().create_room(
        conference_time,
        body.audience.clone(),
        body.backend,
        body.reserve,
        body.tags.clone(),
    );

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = req.state().event_client().create_room(
        event_time,
        body.audience.clone(),
        body.preserve_history,
        body.tags.clone(),
    );

    let (event_room_id, conference_room_id) = event_fut
        .try_join(conference_fut)
        .await
        .map_err(|e| HttpError::new(500, anyhow!("{:?}", e)))?;

    let query = crate::db::class::WebinarInsertQuery::new(
        body.title,
        body.scope,
        body.audience,
        body.time.into(),
        conference_room_id,
        event_room_id,
    );

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };

    let query = if let Some(preserve_history) = body.preserve_history {
        query.preserve_history(preserve_history)
    } else {
        query
    };

    let mut conn = req.state().get_conn().await?;
    let webinar = query.execute(&mut conn).await?;

    if body.locked_chat {
        if let Err(e) = req.state().event_client().lock_chat(event_room_id).await {
            error!(
                crate::LOG,
                "Failed to lock chat in event room, id = {:?}, err = {:?}", event_room_id, e
            );
        }
    }

    let body = serde_json::to_string_pretty(&webinar)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct WebinarUpdate {
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: crate::db::class::BoundedDateTimeTuple,
}

pub async fn update(mut req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let body: WebinarUpdate = req.body_json().await?;

    let maybe_token = fetch_token(&req);
    let state = req.state();
    let account_id = match state.validate_token(maybe_token.as_deref()) {
        Ok(id) => id,
        Err(e) => {
            error!(
                crate::LOG,
                "Failed to process Authorization header, header = {:?}, err = {:?}",
                req.header("Authorization"),
                e
            );
            return Ok(access_denied_response());
        }
    };

    let webinar = find_webinar(&req).await?;

    let object = AuthzObject::new(&["webinars", &webinar.id().to_string()]).into();

    if let Err(err) = state
        .authz()
        .authorize(
            webinar.audience(),
            account_id.clone(),
            object,
            "update".into(),
        )
        .await
    {
        error!(crate::LOG, "Failed to authorize action, reason = {:?}", err);
        return Ok(access_denied_response());
    }

    let conference_time = match body.time.0 {
        Bound::Included(t) | Bound::Excluded(t) => (Bound::Included(t), body.time.1),
        Bound::Unbounded => (Bound::Unbounded, Bound::Unbounded),
    };
    let conference_fut = req
        .state()
        .conference_client()
        .update_room(webinar.id(), conference_time);

    let event_time = (Bound::Included(Utc::now()), Bound::Unbounded);
    let event_fut = req
        .state()
        .event_client()
        .update_room(webinar.id(), event_time);

    event_fut
        .try_join(conference_fut)
        .await
        .map_err(|e| HttpError::new(500, anyhow!("{:?}", e)))?;

    let query = crate::db::class::WebinarTimeUpdateQuery::new(webinar.id(), body.time.into());

    let mut conn = req.state().get_conn().await?;
    let webinar = query.execute(&mut conn).await?;
    let body = serde_json::to_string_pretty(&webinar)?;

    let response = Response::builder(200).body(body).build();

    Ok(response)
}

#[derive(Deserialize)]
struct WebinarConvertObject {
    title: String,
    scope: String,
    audience: String,
    event_room_id: Uuid,
    conference_room_id: Uuid,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: crate::db::class::BoundedDateTimeTuple,
    tags: Option<serde_json::Value>,
    original_event_room_id: Option<Uuid>,
    modified_event_room_id: Option<Uuid>,
    recording: RecordingConvertObject,
}

#[derive(Deserialize)]
struct RecordingConvertObject {
    stream_id: Uuid,
    segments: Segments,
    modified_segments: Segments,
    uri: String,
}

pub async fn convert(mut req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let body: WebinarConvertObject = req.body_json().await?;

    let maybe_token = fetch_token(&req);
    let state = req.state();
    let account_id = match state.validate_token(maybe_token.as_deref()) {
        Ok(id) => id,
        Err(e) => {
            error!(
                crate::LOG,
                "Failed to process Authorization header, header = {:?}, err = {:?}",
                req.header("Authorization"),
                e
            );
            return Ok(access_denied_response());
        }
    };

    let object = AuthzObject::new(&["webinars"]).into();

    if let Err(err) = state
        .authz()
        .authorize(
            body.audience.clone(),
            account_id.clone(),
            object,
            "convert".into(),
        )
        .await
    {
        error!(crate::LOG, "Failed to authorize action, reason = {:?}", err);
        return Ok(access_denied_response());
    }

    let query = crate::db::class::WebinarInsertQuery::new(
        body.title,
        body.scope,
        body.audience,
        body.time.into(),
        body.conference_room_id,
        body.event_room_id,
    );

    let query = if let Some(tags) = body.tags {
        query.tags(tags)
    } else {
        query
    };

    let query = if let Some(id) = body.original_event_room_id {
        query.original_event_room_id(id)
    } else {
        query
    };

    let query = if let Some(id) = body.modified_event_room_id {
        query.modified_event_room_id(id)
    } else {
        query
    };

    let webinar = {
        let mut conn = req.state().get_conn().await?;
        let mut txn = conn.begin().await?;
        let webinar = query.execute(&mut txn).await?;
        crate::db::recording::RecordingConvertInsertQuery::new(
            webinar.id(),
            body.recording.stream_id,
            body.recording.segments,
            body.recording.modified_segments,
            body.recording.uri,
        )
        .execute(&mut txn)
        .await?;
        webinar
    };

    let body = serde_json::to_string_pretty(&webinar)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}

fn fetch_token<T>(req: &Request<T>) -> Option<String> {
    req.header("Authorization")
        .and_then(|h| h.get(0))
        .map(|header| header.to_string())
}

async fn find_webinar(
    req: &Request<Arc<dyn AppContext>>,
) -> Result<crate::db::class::Object, HttpError> {
    let id = req.param("id")?;
    let id = Uuid::from_str(id).map_err(|e| HttpError::new(404, e))?;

    let webinar = {
        let mut conn = req.state().get_conn().await?;
        crate::db::class::WebinarReadQuery::new(id)
            .execute(&mut conn)
            .await?
            .ok_or_else(|| HttpError::new(404, anyhow!("Room not found, id = {:?}", id)))?
    };
    Ok(webinar)
}

fn access_denied_response() -> Response {
    tide::Response::builder(403).body("Access denied").build()
}
