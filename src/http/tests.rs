//! API tests through the real router, Postgres repositories and an in-memory blob store.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::{router, AppState};
use crate::config::Config;
use crate::services::tests::{fixture, Fixture};

pub(super) async fn app() -> (Router, Fixture) {
    let f = fixture().await;
    let config = Config::from_map(&HashMap::new()).unwrap();
    let state = AppState {
        config: Arc::new(config),
        version: "9.9.9",
        sessions: f.sessions.clone(),
        tree: f.tree.clone(),
        uploads: f.uploads.clone(),
        orphans: None,
    };
    (router(state), f)
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    // Extractor rejections (for example a malformed UUID) are plain text, everything else is JSON.
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, json)
}

#[tokio::test]
async fn health_and_version() {
    let (app, _f) = app().await;
    let (status, body) = call(&app, Method::GET, "/healthz", None).await;
    assert_eq!(
        (status, body["status"].as_str()),
        (StatusCode::OK, Some("ok"))
    );
    let (_, body) = call(&app, Method::GET, "/api/version", None).await;
    assert_eq!(body["version"], "9.9.9");
    assert_eq!(body["env"], "production");
}

#[tokio::test]
async fn session_lifecycle() {
    let (app, _f) = app().await;
    let (status, created) = call(
        &app,
        Method::POST,
        "/api/s",
        Some(json!({"description": "run 14"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let code = created["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 5);
    assert_eq!(created["url"], format!("http://localhost:8080/{code}"));
    assert_eq!(created["idle_ttl_days"], 7);
    assert_eq!(created["max_files"], 100);
    assert!(created["expires_at"].is_string());

    let (status, got) = call(&app, Method::GET, &format!("/api/s/{code}"), None).await;
    assert_eq!(
        (status, got["description"].as_str()),
        (StatusCode::OK, Some("run 14"))
    );

    let (_, patched) = call(
        &app,
        Method::PATCH,
        &format!("/api/s/{code}"),
        Some(json!({"description": "new text"})),
    )
    .await;
    assert_eq!(patched["description"], "new text");

    let (status, _) = call(&app, Method::DELETE, &format!("/api/s/{code}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, err) = call(&app, Method::GET, &format!("/api/s/{code}"), None).await;
    assert_eq!(
        (status, err["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("not found"))
    );
}

#[tokio::test]
async fn create_without_body_and_unknown_code() {
    let (app, _f) = app().await;
    let (status, created) = call(&app, Method::POST, "/api/s", None).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["description"], "");
    let code = created["code"].as_str().unwrap();
    call(&app, Method::DELETE, &format!("/api/s/{code}"), None).await;
    // Look-alike characters can never be part of a code, so this is simply not found.
    let (status, _) = call(&app, Method::GET, "/api/s/k7m3o", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn folders_notes_and_listing() {
    let (app, _f) = app().await;
    let (_, s) = call(&app, Method::POST, "/api/s", None).await;
    let code = s["code"].as_str().unwrap();

    let (status, folder) = call(
        &app,
        Method::POST,
        &format!("/api/s/{code}/folders"),
        Some(json!({"name": "fixtures"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = folder["id"].as_str().unwrap();

    let (status, dup) = call(
        &app,
        Method::POST,
        &format!("/api/s/{code}/folders"),
        Some(json!({"name": "fixtures"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(dup["error"].as_str().unwrap().contains("already exists"));

    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/s/{code}/folders"),
        Some(json!({"name": "a/b"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, patched) = call(
        &app,
        Method::PATCH,
        &format!("/api/s/{code}/nodes/{id}"),
        Some(json!({"note": "only failing ones"})),
    )
    .await;
    assert_eq!(patched["note"], "only failing ones");

    let (_, listing) = call(&app, Method::GET, &format!("/api/s/{code}/files"), None).await;
    assert_eq!(listing["items"][0]["name"], "fixtures");
    assert_eq!(listing["items"][0]["kind"], "folder");
    assert_eq!(listing["path"], json!([]));

    let (_, inner) = call(
        &app,
        Method::GET,
        &format!("/api/s/{code}/files?parent={id}"),
        None,
    )
    .await;
    assert_eq!(inner["path"][0]["name"], "fixtures");
    assert_eq!(inner["items"], json!([]));

    let (status, _) = call(
        &app,
        Method::GET,
        &format!("/api/s/{code}/files?parent=not-a-uuid"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/s/{code}/nodes/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/s/{code}/nodes/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    call(&app, Method::DELETE, &format!("/api/s/{code}"), None).await;
}

#[tokio::test]
async fn sessions_cannot_see_each_others_nodes() {
    let (app, _f) = app().await;
    let (_, a) = call(&app, Method::POST, "/api/s", None).await;
    let (_, b) = call(&app, Method::POST, "/api/s", None).await;
    let (ca, cb) = (a["code"].as_str().unwrap(), b["code"].as_str().unwrap());
    let (_, folder) = call(
        &app,
        Method::POST,
        &format!("/api/s/{ca}/folders"),
        Some(json!({"name": "secret"})),
    )
    .await;
    let id = folder["id"].as_str().unwrap();
    let (status, _) = call(
        &app,
        Method::PATCH,
        &format!("/api/s/{cb}/nodes/{id}"),
        Some(json!({"note": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/s/{cb}/nodes/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    call(&app, Method::DELETE, &format!("/api/s/{ca}"), None).await;
    call(&app, Method::DELETE, &format!("/api/s/{cb}"), None).await;
}
