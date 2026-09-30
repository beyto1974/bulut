//! Endpoints for LLM agents: text indexes, the OpenAPI description and MCP.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
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
    ))
}

pub async fn openapi(State(state): State<AppState>) -> Response {
    let body = OPENAPI
        .replace("{{BASE_URL}}", &state.config.base_url)
        .replace("{{VERSION}}", state.version);
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

pub async fn mcp_post(State(state): State<AppState>, body: Bytes) -> ApiResult<Response> {
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
