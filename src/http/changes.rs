//! `GET /api/s/{code}/changes`: what happened in a session after a cursor, as JSON (long poll)
//! or as server-sent events.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use tokio::time::Instant;

use super::error::ApiResult;
use super::AppState;
use crate::services::error::ServiceError;

/// One wait inside an event stream.
const STREAM_WAIT: Duration = Duration::from_secs(25);
/// A stream ends after this long and the client reconnects with its last id. That keeps a
/// restart of the server from waiting on streams that never finish.
const STREAM_MAX: Duration = Duration::from_secs(600);

#[derive(Deserialize)]
pub struct ChangesQuery {
    /// Cursor: the `seq` of the last event seen. Without it, only what happens from now on.
    pub after: Option<i64>,
    /// Seconds to wait for news when there is none yet (at most 30). JSON answers only.
    pub wait: Option<u64>,
}

fn wants_stream(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/event-stream"))
}

fn last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}

pub async fn changes(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<ChangesQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = state.sessions.peek(&code).await?;
    let cursor = match q.after.or_else(|| last_event_id(&headers)) {
        Some(c) => c,
        None => state.changes.head(&session.code).await?,
    };
    if wants_stream(&headers) {
        return Ok(stream(state, session.code, cursor));
    }
    let wait = Duration::from_secs(q.wait.unwrap_or(0));
    let page = state.changes.wait(&session.code, cursor, wait).await?;
    let status = if page.reset {
        StatusCode::GONE
    } else {
        StatusCode::OK
    };
    Ok((status, Json(page)).into_response())
}

struct Feed {
    state: AppState,
    code: String,
    cursor: i64,
    queue: VecDeque<Event>,
    started: Instant,
    done: bool,
}

fn stream(state: AppState, code: String, cursor: i64) -> Response {
    let feed = Feed {
        state,
        code,
        cursor,
        queue: VecDeque::new(),
        started: Instant::now(),
        done: false,
    };
    let events = futures_util::stream::unfold(feed, |mut f| async move {
        loop {
            if let Some(event) = f.queue.pop_front() {
                return Some((Ok::<Event, Infallible>(event), f));
            }
            if f.done || f.state.changes.is_closing() || f.started.elapsed() > STREAM_MAX {
                return None;
            }
            match f.state.changes.wait(&f.code, f.cursor, STREAM_WAIT).await {
                Ok(page) => {
                    if page.reset {
                        f.queue.push_back(
                            Event::default()
                                .event("reset")
                                .id(page.seq.to_string())
                                .data(format!("{{\"seq\":{}}}", page.seq)),
                        );
                    }
                    for change in &page.events {
                        if let Ok(event) = Event::default()
                            .event("change")
                            .id(change.seq.to_string())
                            .json_data(change)
                        {
                            f.queue.push_back(event);
                        }
                    }
                    f.cursor = page.seq;
                }
                Err(ServiceError::NotFound) => {
                    f.queue.push_back(Event::default().event("gone").data("{}"));
                    f.done = true;
                }
                Err(e) => {
                    tracing::error!(error = %e, "change stream failed");
                    return None;
                }
            }
        }
    });
    (
        // Tells buffering proxies to pass the events on as they come.
        [("x-accel-buffering", "no")],
        Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))),
    )
        .into_response()
}
