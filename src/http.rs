//! HTTP layer. Handlers stay thin and call services (added in later phases).

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub version: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/version", get(version))
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
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::collections::HashMap;
    use tower::ServiceExt;

    fn app() -> Router {
        router(AppState {
            config: Config::from_map(&HashMap::new()).unwrap(),
            version: "9.9.9",
        })
    }

    async fn get_json(path: &str) -> (StatusCode, Value) {
        let res = app()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn healthz_is_ok() {
        let (status, body) = get_json("/healthz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn version_reports_version_and_env() {
        let (status, body) = get_json("/api/version").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["version"], "9.9.9");
        assert_eq!(body["env"], "production");
    }
}
