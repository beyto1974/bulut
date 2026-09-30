//! HTTP layer. Handlers stay thin: parse, call a service, shape the response.

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::config::Config;
use crate::services::session_service::SessionService;
use crate::services::tree_service::TreeService;
use crate::services::upload_service::UploadService;

pub mod agent;
pub mod error;
pub mod files;
pub mod mcp;
pub mod sessions;
pub mod tree;
pub mod ui;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub version: &'static str,
    pub sessions: Arc<SessionService>,
    pub tree: Arc<TreeService>,
    pub uploads: Arc<UploadService>,
}

pub fn router(state: AppState) -> Router {
    // A part is at most one chunk; allow a little slack for clients that add framing.
    let part_limit = state.config.chunk_size + 1024;
    Router::new()
        .route("/", get(ui::home))
        .route("/assets/app.css", get(ui::css))
        .route("/assets/app.js", get(ui::js))
        .route("/{code}", get(ui::session_page))
        .route("/api/s/{code}/qr.svg", get(ui::qr))
        .route("/healthz", get(healthz))
        .route("/api/version", get(version))
        .route("/llms.txt", get(agent::general_llms))
        .route("/openapi.json", get(agent::openapi))
        .route("/mcp", post(agent::mcp_post))
        .route("/{code}/llms.txt", get(agent::session_llms))
        .route("/api/s", post(sessions::create))
        .route(
            "/api/s/{code}",
            get(sessions::get)
                .patch(sessions::patch)
                .delete(sessions::delete),
        )
        .route("/api/s/{code}/files", get(tree::list))
        .route("/api/s/{code}/folders", post(tree::create_folder))
        .route(
            "/api/s/{code}/nodes/{id}",
            patch(tree::patch_node).delete(tree::delete_node),
        )
        .route("/api/s/{code}/nodes/{id}/versions", get(tree::versions))
        .route(
            "/api/s/{code}/versions/{version}/tags/{tag}",
            put(tree::add_tag).delete(tree::remove_tag),
        )
        .route("/api/s/{code}/uploads", post(files::init_upload))
        .route(
            "/api/s/{code}/uploads/{id}",
            get(files::upload_status).delete(files::abort_upload),
        )
        .route(
            "/api/s/{code}/uploads/{id}/parts/{number}",
            put(files::put_part).layer(DefaultBodyLimit::max(part_limit)),
        )
        .route(
            "/api/s/{code}/uploads/{id}/complete",
            post(files::complete_upload),
        )
        // The body is streamed and its size is checked by the service (MAX_FILE_BYTES).
        .route(
            "/api/s/{code}/upload",
            put(files::simple_upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/s/{code}/download", get(files::download_named))
        .route(
            "/api/s/{code}/versions/{version}/download",
            get(files::download_version),
        )
        .with_state(state)
        // Files and pages are user content or carry user content: never sniff types, never leak the URL.
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn version(axum::extract::State(state): axum::extract::State<AppState>) -> Json<Value> {
    Json(json!({
        "version": state.version,
        "env": state.config.app_env.as_str(),
    }))
}

#[cfg(test)]
mod agent_tests;
#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod ui_tests;
