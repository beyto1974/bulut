//! The change feed through the real router.

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::tests::app;
use crate::ports::clock::Clock;
use crate::ports::node_repo::NodeRepo;
use crate::services::tests::nv;

async fn get(app: &Router, uri: &str, accept: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().uri(uri);
    if let Some(a) = accept {
        req = req.header(header::ACCEPT, a);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn events_come_back_after_the_cursor_and_the_listing_carries_one() {
    let (app, f) = app().await;
    let s = f.sessions.create("").await.unwrap();
    let (_, listing) = get(&app, &format!("/api/s/{}/files", s.code), None).await;
    assert_eq!(listing["seq"], 0);

    f.nodes
        .add_version(&s.code, None, "a.txt", nv("h1/a"), f.clock.now())
        .await
        .unwrap();
    let (status, page) = get(&app, &format!("/api/s/{}/changes?after=0", s.code), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["seq"], 1);
    assert_eq!(page["reset"], false);
    assert_eq!(page["events"][0]["kind"], "version_added");
    assert_eq!(page["events"][0]["name"], "a.txt");
    assert_eq!(page["events"][0]["version"], 1);

    let (_, listing) = get(&app, &format!("/api/s/{}/files", s.code), None).await;
    assert_eq!(listing["seq"], 1);
    let (_, quiet) = get(&app, &format!("/api/s/{}/changes?after=1", s.code), None).await;
    assert_eq!(
        (quiet["seq"].clone(), quiet["events"].clone()),
        (json!(1), json!([]))
    );
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn without_a_cursor_the_answer_is_the_current_one() {
    let (app, f) = app().await;
    let s = f.sessions.create("").await.unwrap();
    f.nodes
        .add_version(&s.code, None, "a.txt", nv("h2/a"), f.clock.now())
        .await
        .unwrap();
    let (status, page) = get(&app, &format!("/api/s/{}/changes", s.code), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (page["seq"].clone(), page["events"].clone()),
        (json!(1), json!([]))
    );
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_cursor_from_the_future_gets_410_and_the_current_one() {
    let (app, f) = app().await;
    let s = f.sessions.create("").await.unwrap();
    let (status, page) = get(&app, &format!("/api/s/{}/changes?after=99", s.code), None).await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(
        (page["reset"].clone(), page["seq"].clone()),
        (json!(true), json!(0))
    );
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn unknown_sessions_are_404_for_both_kinds_of_client() {
    let (app, _f) = app().await;
    for accept in [None, Some("text/event-stream")] {
        let (status, _) = get(&app, "/api/s/zzzzz/changes?after=0", accept).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn a_long_poll_returns_when_something_is_uploaded() {
    let (app, f) = app().await;
    let s = f.sessions.create("").await.unwrap();
    let poll = tokio::spawn({
        let app = app.clone();
        let uri = format!("/api/s/{}/changes?after=0&wait=20", s.code);
        async move { get(&app, &uri, None).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    f.nodes
        .add_version(&s.code, None, "late.txt", nv("h3/late"), f.clock.now())
        .await
        .unwrap();
    let (status, page) = tokio::time::timeout(Duration::from_secs(5), poll)
        .await
        .expect("answers soon")
        .unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["events"][0]["name"], "late.txt");
    f.sessions.delete(&s.code).await.unwrap();
}

async fn first_frame(app: &Router, uri: &str, headers: &[(&str, &str)]) -> String {
    let mut req = Request::builder()
        .uri(uri)
        .header(header::ACCEPT, "text/event-stream");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    assert_eq!(res.headers()["x-accel-buffering"], "no");
    let mut body = res.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .expect("an event arrives")
        .unwrap()
        .unwrap();
    String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap()
}

#[tokio::test]
async fn the_event_stream_sends_what_is_new_with_ids() {
    let (app, f) = app().await;
    let s = f.sessions.create("").await.unwrap();
    for (name, key) in [("a.txt", "h4/a"), ("b.txt", "h4/b")] {
        f.nodes
            .add_version(&s.code, None, name, nv(key), f.clock.now())
            .await
            .unwrap();
    }
    let text = first_frame(&app, &format!("/api/s/{}/changes?after=0", s.code), &[]).await;
    assert!(text.contains("event: change"), "{text}");
    assert!(text.contains("id: 1"), "{text}");
    assert!(text.contains("\"name\":\"a.txt\""), "{text}");

    // A reconnecting browser sends the last id it saw instead of a query.
    let text = first_frame(
        &app,
        &format!("/api/s/{}/changes", s.code),
        &[("last-event-id", "1")],
    )
    .await;
    assert!(text.contains("id: 2"), "{text}");
    assert!(text.contains("\"name\":\"b.txt\""), "{text}");
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_stream_with_a_cursor_from_the_future_starts_with_a_reset() {
    let (app, f) = app().await;
    let s = f.sessions.create("").await.unwrap();
    let text = first_frame(&app, &format!("/api/s/{}/changes?after=50", s.code), &[]).await;
    assert!(text.contains("event: reset"), "{text}");
    assert!(text.contains("{\"seq\":0}"), "{text}");
    f.sessions.delete(&s.code).await.unwrap();
}
