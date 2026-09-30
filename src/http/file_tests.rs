//! Upload and download through the router. The fixture uses 10 byte parts and a 1000 byte limit.

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::tests::app;

pub(super) async fn raw(
    app: &Router,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: &'static [u8],
) -> (StatusCode, HeaderMap, Bytes) {
    let mut req = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (
        status,
        headers,
        res.into_body().collect().await.unwrap().to_bytes(),
    )
}

pub(super) fn json_of(b: &Bytes) -> Value {
    serde_json::from_slice(b).unwrap_or(Value::Null)
}

pub(super) async fn new_session(app: &Router) -> String {
    let (_, _, b) = raw(app, Method::POST, "/api/s", &[], b"").await;
    json_of(&b)["code"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn chunked_upload_then_full_and_ranged_download() {
    let (app, _f) = app().await;
    let code = new_session(&app).await;

    let (status, _, b) = raw(
        &app,
        Method::POST,
        &format!("/api/s/{code}/uploads"),
        &[("content-type", "application/json")],
        br#"{"name":"notes.txt","size":25,"content_type":"text/plain","created_at":"2026-09-01T08:30:00Z"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let up = json_of(&b);
    let id = up["upload_id"].as_str().unwrap().to_string();
    assert_eq!(
        (up["part_size"].as_i64(), up["parts_total"].as_i64()),
        (Some(10), Some(3))
    );

    let part = |n: u32| format!("/api/s/{code}/uploads/{id}/parts/{n}");
    assert_eq!(
        raw(&app, Method::PUT, &part(1), &[], b"0123456789").await.0,
        StatusCode::NO_CONTENT
    );
    // Wrong size for a full part is refused.
    assert_eq!(
        raw(&app, Method::PUT, &part(2), &[], b"short").await.0,
        StatusCode::BAD_REQUEST
    );
    // Not finished yet: the client can ask what is missing.
    let (_, _, b) = raw(
        &app,
        Method::GET,
        &format!("/api/s/{code}/uploads/{id}"),
        &[],
        b"",
    )
    .await;
    assert_eq!(json_of(&b)["parts_received"], json!([1]));
    assert_eq!(
        raw(
            &app,
            Method::POST,
            &format!("/api/s/{code}/uploads/{id}/complete"),
            &[],
            b""
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        raw(&app, Method::PUT, &part(2), &[], b"ABCDEFGHIJ").await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        raw(&app, Method::PUT, &part(3), &[], b"abcde").await.0,
        StatusCode::NO_CONTENT
    );

    let (status, _, b) = raw(
        &app,
        Method::POST,
        &format!("/api/s/{code}/uploads/{id}/complete"),
        &[],
        b"",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let stored = json_of(&b);
    assert_eq!(stored["node"]["name"], "notes.txt");
    assert_eq!(stored["version"]["size"], 25);
    assert_eq!(
        stored["version"]["client_created_at"],
        "2026-09-01T08:30:00Z"
    );
    assert!(stored["version"]["uploaded_at"].is_string());
    let vid = stored["version"]["id"].as_str().unwrap();

    // Full download.
    let url = format!("/api/s/{code}/versions/{vid}/download");
    let (status, h, body) = raw(&app, Method::GET, &url, &[], b"").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"0123456789ABCDEFGHIJabcde");
    assert_eq!(h["content-type"], "text/plain");
    assert_eq!(h["content-length"], "25");
    assert_eq!(h["accept-ranges"], "bytes");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(h["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("attachment; filename=\"notes.txt\""));

    // Range download.
    let (status, h, body) = raw(&app, Method::GET, &url, &[("range", "bytes=8-12")], b"").await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&body[..], b"89ABC");
    assert_eq!(h["content-range"], "bytes 8-12/25");
    assert_eq!(h["content-length"], "5");
    let (status, h, _) = raw(&app, Method::GET, &url, &[("range", "bytes=100-")], b"").await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(h["content-range"], "bytes */25");

    raw(&app, Method::DELETE, &format!("/api/s/{code}"), &[], b"").await;
}

#[tokio::test]
async fn simple_upload_versions_tags_and_download_by_name() {
    let (app, _f) = app().await;
    let code = new_session(&app).await;

    let (status, _, b) = raw(
        &app,
        Method::PUT,
        &format!("/api/s/{code}/upload?name=build.apk&tag=v1.4.0,stable"),
        &[("content-type", "application/vnd.android.package-archive")],
        b"first build",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let first = json_of(&b);
    assert_eq!(first["version"]["version"], 1);
    assert_eq!(
        first["version"]["tags"],
        json!(["latest", "stable", "v1.4.0"])
    );

    let (_, _, b) = raw(
        &app,
        Method::PUT,
        &format!("/api/s/{code}/upload?name=build.apk&tag=v1.4.1"),
        &[],
        b"second build!",
    )
    .await;
    let second = json_of(&b);
    assert_eq!(second["version"]["version"], 2);
    assert_eq!(
        second["node"]["id"], first["node"]["id"],
        "same file, new version"
    );

    let get = |q: &str| {
        let app = app.clone();
        let url = format!("/api/s/{code}/download?{q}");
        async move { raw(&app, Method::GET, &url, &[], b"").await }
    };
    assert_eq!(&get("name=build.apk").await.2[..], b"second build!");
    assert_eq!(
        &get("name=build.apk&tag=latest").await.2[..],
        b"second build!"
    );
    assert_eq!(
        &get("name=build.apk&tag=v1.4.0").await.2[..],
        b"first build"
    );
    assert_eq!(
        &get("name=build.apk&tag=stable").await.2[..],
        b"first build"
    );
    assert_eq!(
        get("name=build.apk&tag=nope").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(get("name=other.apk").await.0, StatusCode::NOT_FOUND);

    let node_id = first["node"]["id"].as_str().unwrap();
    let (_, _, b) = raw(
        &app,
        Method::GET,
        &format!("/api/s/{code}/nodes/{node_id}/versions"),
        &[],
        b"",
    )
    .await;
    let versions = json_of(&b);
    assert_eq!(versions.as_array().unwrap().len(), 2);
    assert_eq!(versions[0]["version"], 2, "newest first");

    // Moving a tag: stable now points at version 2.
    let vid2 = second["version"]["id"].as_str().unwrap();
    let (status, _, b) = raw(
        &app,
        Method::PUT,
        &format!("/api/s/{code}/versions/{vid2}/tags/stable"),
        &[],
        b"",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json_of(&b)["tags"]
        .as_array()
        .unwrap()
        .contains(&json!("stable")));
    assert_eq!(
        &get("name=build.apk&tag=stable").await.2[..],
        b"second build!"
    );

    raw(&app, Method::DELETE, &format!("/api/s/{code}"), &[], b"").await;
}

#[tokio::test]
async fn upload_limits_and_bad_input() {
    let (app, _f) = app().await;
    let code = new_session(&app).await;
    let big: &'static [u8] = &[7u8; 1001];
    let (status, _, b) = raw(
        &app,
        Method::PUT,
        &format!("/api/s/{code}/upload?name=big"),
        &[],
        big,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json_of(&b)["error"].as_str().unwrap().contains("limit"));
    let (status, _, _) = raw(
        &app,
        Method::PUT,
        &format!("/api/s/{code}/upload?name=a%2Fb"),
        &[],
        b"x",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = raw(
        &app,
        Method::PUT,
        &format!("/api/s/{code}/upload?name=x&tag=two%20words"),
        &[],
        b"x",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = raw(
        &app,
        Method::PUT,
        &format!("/api/s/{code}/upload"),
        &[],
        b"x",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "name is required");
    let (status, _, _) = raw(
        &app,
        Method::GET,
        &format!("/api/s/{code}/versions/{}/download", uuid::Uuid::new_v4()),
        &[],
        b"",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = raw(&app, Method::PUT, "/api/s/k7m3q/upload?name=x", &[], b"x").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown session");
    raw(&app, Method::DELETE, &format!("/api/s/{code}"), &[], b"").await;
}
