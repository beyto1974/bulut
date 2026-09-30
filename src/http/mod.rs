//! HTTP layer. Handlers stay thin: parse, call a service, shape the response.

use std::sync::Arc;

use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::config::Config;
use crate::services::session_service::SessionService;
use crate::services::tree_service::TreeService;

pub mod error;
pub mod sessions;
pub mod tree;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub version: &'static str,
    pub sessions: Arc<SessionService>,
    pub tree: Arc<TreeService>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/version", get(version))
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
        .with_state(state)
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
mod tests;
