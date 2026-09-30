use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::ApiResult;
use super::AppState;
use crate::domain::session::Session;

#[derive(Serialize)]
pub struct SessionDto {
    pub code: String,
    pub url: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    /// When the session is deleted if nobody touches it.
    pub expires_at: DateTime<Utc>,
    pub idle_ttl_days: u32,
    /// Most files this session can hold.
    pub max_files: u32,
}

impl SessionDto {
    pub fn new(s: &Session, state: &AppState) -> Self {
        let ttl = state.config.session_idle_ttl_days;
        Self {
            url: format!("{}/{}", state.config.base_url, s.code),
            code: s.code.clone(),
            description: s.description.clone(),
            created_at: s.created_at,
            last_activity_at: s.last_activity_at,
            expires_at: s.expires_at(ttl),
            idle_ttl_days: ttl,
            max_files: state.config.max_files_per_session,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct CreateSession {
    #[serde(default)]
    pub description: String,
}

#[derive(Deserialize)]
pub struct PatchSession {
    pub description: String,
}

pub async fn create(
    State(state): State<AppState>,
    body: Option<Json<CreateSession>>,
) -> ApiResult<(StatusCode, Json<SessionDto>)> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let session = state.sessions.create(&body.description).await?;
    tracing::info!(session = %session.code, "session created");
    Ok((StatusCode::CREATED, Json(SessionDto::new(&session, &state))))
}

pub async fn get(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> ApiResult<Json<SessionDto>> {
    let session = state.sessions.open(&code).await?;
    Ok(Json(SessionDto::new(&session, &state)))
}

pub async fn patch(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Json(body): Json<PatchSession>,
) -> ApiResult<Json<SessionDto>> {
    let session = state
        .sessions
        .set_description(&code, &body.description)
        .await?;
    Ok(Json(SessionDto::new(&session, &state)))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> ApiResult<StatusCode> {
    state.sessions.delete(&code).await?;
    tracing::info!(session = %code, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}
