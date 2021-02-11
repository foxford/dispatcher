use std::str::FromStr;
use std::sync::Arc;

use async_std::prelude::FutureExt;
use serde_derive::Deserialize;
use tide::{http::Error as HttpError, Request, Response};
use uuid::Uuid;

use crate::app::AppContext;

pub async fn read(req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let id = req.param("id")?;
    let id = Uuid::from_str(id).map_err(|e| HttpError::new(404, e))?;

    let mut conn = req.state().get_conn().await?;
    let webinar = crate::db::class::WebinarReadQuery::new(id)
        .execute(&mut conn)
        .await
        .map_err(|e| HttpError::new(500, e))?;

    match webinar {
        Some(webinar) => {
            let body = serde_json::to_string(&webinar).map_err(|e| HttpError::new(500, e))?;
            let response = Response::builder(200).body(body).build();
            Ok(response)
        }
        None => Err(HttpError::new(404, anyhow!("Webinar not found"))),
    }
}

#[derive(Deserialize)]
struct Webinar {
    scope: String,
    audience: String,
    #[serde(with = "crate::serde::ts_seconds_bound_tuple")]
    time: crate::db::class::BoundedDateTimeTuple,
    tags: Option<serde_json::Value>,
    preserve_history: Option<bool>,
    reserve: Option<i32>,
    backend: Option<String>,
}

pub async fn create(mut req: Request<Arc<dyn AppContext>>) -> tide::Result {
    let body: Webinar = req.body_json().await?;

    let conference_fut = req.state().conference_client().create_room(
        body.time,
        body.audience.clone(),
        body.backend,
        body.reserve,
        body.tags.clone(),
    );

    let event_fut = req.state().event_client().create_room(
        body.time,
        body.audience.clone(),
        body.preserve_history,
        body.tags.clone(),
    );

    let (event_room_id, conference_room_id) = event_fut
        .try_join(conference_fut)
        .await
        .map_err(|e| HttpError::new(500, anyhow!("{:?}", e)))?;

    let query = crate::db::class::WebinarInsertQuery::new(
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
    let body = serde_json::to_string_pretty(&webinar)?;

    let response = Response::builder(201).body(body).build();

    Ok(response)
}
