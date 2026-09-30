//! Endpoints for LLM agents: text indexes, the OpenAPI description and MCP.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use serde_json::{json, Value};

use super::error::ApiResult;
use super::mcp;
use super::AppState;
use crate::llms;
use crate::services::error::ServiceError;

/// Cap on nodes in a text index, so one huge session cannot produce a huge response.
const INDEX_LIMIT: usize = 1000;

const OPENAPI: &str = include_str!("../../web/openapi.json");

pub async fn render_index(state: &AppState, code: &str) -> Result<String, ServiceError> {
    let session = state.sessions.open(code).await?;
    let walk = state.tree.walk(&session.code, INDEX_LIMIT).await?;
    Ok(llms::session_index(
        &state.config.base_url,
        &session,
        state.config.session_idle_ttl_days,
        state.config.max_files_per_session,
        &walk,
    ))
}

fn text(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

pub async fn session_llms(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> ApiResult<Response> {
    Ok(text(render_index(&state, &code).await?))
}

pub async fn general_llms(State(state): State<AppState>) -> Response {
    text(llms::general_index(
        &state.config.base_url,
        state.config.code_length,
        state.config.session_idle_ttl_days,
        state.config.max_file_bytes,
        state.config.max_files_per_session,
    ))
}

pub async fn openapi(State(state): State<AppState>) -> Response {
    let body = OPENAPI
        .replace("{{BASE_URL}}", &state.config.base_url)
        .replace("{{VERSION}}", state.version);
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// `scheme://host[:port]` of a URL, the form a browser puts in the `Origin` header.
pub fn origin_of(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => format!(
            "{}://{}",
            scheme.to_ascii_lowercase(),
            rest.split('/').next().unwrap_or("").to_ascii_lowercase()
        ),
        None => url.to_ascii_lowercase(),
    }
}

pub async fn mcp_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    // A page on another site can send a POST with a simple content type and no preflight, and the
    // browser attaches cached basic-auth credentials. Requiring JSON makes it a preflighted request,
    // and a browser request from another origin is refused outright. Agents do not send `Origin`.
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/json")
        });
    if !is_json {
        let err = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": "content type must be application/json" } });
        return Ok((StatusCode::UNSUPPORTED_MEDIA_TYPE, Json(err)).into_response());
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if origin.to_ascii_lowercase() != origin_of(&state.config.base_url) {
            let err = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": "origin not allowed" } });
            return Ok((StatusCode::FORBIDDEN, Json(err)).into_response());
        }
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let err = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } });
            return Ok((StatusCode::BAD_REQUEST, Json(err)).into_response());
        }
    };
    match parsed {
        Value::Array(batch) => {
            let mut replies = Vec::new();
            for req in batch {
                if let Some(r) = mcp::handle(&state, req).await {
                    replies.push(r);
                }
            }
            if replies.is_empty() {
                Ok(StatusCode::ACCEPTED.into_response())
            } else {
                Ok(Json(Value::Array(replies)).into_response())
            }
        }
        single => match mcp::handle(&state, single).await {
            Some(reply) => Ok(Json(reply).into_response()),
            None => Ok(StatusCode::ACCEPTED.into_response()),
        },
    }
}
