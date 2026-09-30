//! llms.txt, openapi.json and MCP through the router.

use axum::http::{Method, StatusCode};
use axum::Router;
use serde_json::{json, Value};

use super::file_tests::{json_of, new_session, raw};
use super::tests::app;

async fn upload(app: &Router, code: &str, name: &str, tags: &str, body: &'static [u8]) {
    let url = format!("/api/s/{code}/upload?name={name}&tag={tags}");
    assert_eq!(
        raw(app, Method::PUT, &url, &[], body).await.0,
        StatusCode::CREATED
    );
}

async fn rpc(app: &Router, msg: Value) -> (StatusCode, Value) {
    let body: &'static [u8] = Box::leak(msg.to_string().into_bytes().into_boxed_slice());
    let (status, _, b) = raw(
        app,
        Method::POST,
        "/mcp",
        &[("content-type", "application/json")],
        body,
    )
    .await;
    (status, json_of(&b))
}

fn call(id: u32, tool: &str, args: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": args } })
}

fn tool_text(reply: &Value) -> (bool, String) {
    (
        reply["result"]["isError"].as_bool().unwrap(),
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string(),
    )
}

#[tokio::test]
async fn session_llms_txt_describes_files_versions_and_notes() {
    let (app, _f) = app().await;
    let code = new_session(&app).await;
    upload(&app, &code, "build.apk", "v1.4.0", b"one").await;
    upload(&app, &code, "build.apk", "v1.4.1", b"two!").await;

    let (status, headers, body) =
        raw(&app, Method::GET, &format!("/{code}/llms.txt"), &[], b"").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.starts_with(&format!("# Session {code}")), "{text}");
    assert!(text.contains("- build.apk (4 B, 2 version(s)"), "{text}");
    assert!(text.contains("tags: latest, v1.4.1"), "{text}");
    assert!(text.contains("  - v1: 3 B"), "{text}");
    assert!(
        text.contains(&format!("/api/s/{code}/download?name=build.apk")),
        "{text}"
    );

    let (status, _, _) = raw(&app, Method::GET, "/k7m3q/llms.txt", &[], b"").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    raw(&app, Method::DELETE, &format!("/api/s/{code}"), &[], b"").await;
}

#[tokio::test]
async fn general_llms_txt_states_the_rules() {
    let (app, _f) = app().await;
    let (status, _, body) = raw(&app, Method::GET, "/llms.txt", &[], b"").await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("after 7 days without any activity"));
    assert!(text.contains("/openapi.json") && text.contains("/mcp"));
    assert!(
        !text.to_lowercase().contains("folder"),
        "folders are not offered while disabled"
    );
}

#[tokio::test]
async fn openapi_is_valid_and_every_operation_is_routable() {
    let (app, _f) = app().await;
    let (status, headers, body) = raw(&app, Method::GET, "/openapi.json", &[], b"").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/json");
    let spec = json_of(&body);
    assert_eq!(spec["openapi"], "3.0.3");
    assert_eq!(spec["info"]["version"], "9.9.9");
    assert_eq!(spec["servers"][0]["url"], "http://localhost:8080");
    assert!(
        !String::from_utf8_lossy(&body).contains("{{"),
        "all placeholders are filled"
    );
    assert!(!String::from_utf8_lossy(&body)
        .to_lowercase()
        .contains("folder"));

    // Every documented path and method must reach a handler: not 405, and not an empty 404 from the router.
    let id = uuid::Uuid::new_v4().to_string();
    let mut checked = 0;
    for (path, item) in spec["paths"].as_object().unwrap() {
        let url = path
            .replace("{code}", "k7m3q")
            .replace("{id}", &id)
            .replace("{version}", &id)
            .replace("{number}", "1")
            .replace("{tag}", "x");
        for method in ["get", "put", "post", "patch", "delete"] {
            if item.get(method).is_none() {
                continue;
            }
            let m = Method::from_bytes(method.to_uppercase().as_bytes()).unwrap();
            let (status, _, body) = raw(
                &app,
                m,
                &format!("{url}?name=x"),
                &[("content-type", "application/json")],
                b"{}",
            )
            .await;
            assert_ne!(
                status,
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path} is documented but not routed"
            );
            assert!(
                !(status == StatusCode::NOT_FOUND && body.is_empty()),
                "{method} {path} is documented but no route matches"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "checked {checked} operations");
}

#[tokio::test]
async fn mcp_handshake_and_errors() {
    let (app, _f) = app().await;
    let (status, reply) = rpc(&app, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["serverInfo"]["name"], "bulut");
    assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
    assert!(reply["result"]["capabilities"]["tools"].is_object());

    // A notification gets no body.
    let (status, reply) = rpc(
        &app,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    assert_eq!((status, reply), (StatusCode::ACCEPTED, Value::Null));

    let (_, reply) = rpc(
        &app,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    let names: Vec<&str> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "create_session",
            "get_session",
            "upload_text",
            "read_text_file",
            "set_note",
            "add_tag",
            "set_description"
        ]
    );
    assert!(reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["inputSchema"]["type"] == "object"));

    let (_, reply) = rpc(&app, json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" })).await;
    assert_eq!(reply["error"]["code"], -32601);
    let (_, reply) = rpc(&app, json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" })).await;
    assert_eq!(reply["result"], json!({}));

    let (status, _, b) = raw(&app, Method::POST, "/mcp", &[], b"{not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json_of(&b)["error"]["code"], -32700);

    // A batch answers every request and skips notifications.
    let batch = json!([
        { "jsonrpc": "2.0", "id": 5, "method": "ping" },
        { "jsonrpc": "2.0", "method": "notifications/initialized" },
        { "jsonrpc": "2.0", "id": 6, "method": "ping" }
    ]);
    let (_, reply) = rpc(&app, batch).await;
    assert_eq!(reply.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn mcp_tools_work_end_to_end() {
    let (app, _f) = app().await;
    let (_, reply) = rpc(
        &app,
        call(
            1,
            "create_session",
            json!({ "description": "shared by an agent" }),
        ),
    )
    .await;
    let (is_err, text) = tool_text(&reply);
    assert!(!is_err, "{text}");
    let code = text
        .split_whitespace()
        .nth(2)
        .unwrap()
        .trim_end_matches('.')
        .to_string();
    assert_eq!(code.len(), 5, "{text}");
    assert!(text.contains("after 7 days without activity"));

    let args = |extra: Value| {
        let mut base = json!({ "code": code });
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        base
    };
    let (_, r) = rpc(&app, call(2, "upload_text", args(json!({ "name": "notes.md", "content": "héllo wörld, first", "tags": ["v1"], "note": "draft" })))).await;
    assert_eq!(
        tool_text(&r),
        (
            false,
            "Stored notes.md as version 1 (20 bytes).".to_string()
        )
    );
    let (_, r) = rpc(
        &app,
        call(
            3,
            "upload_text",
            args(json!({ "name": "notes.md", "content": "second", "tags": ["v2"] })),
        ),
    )
    .await;
    assert!(tool_text(&r).1.contains("version 2"));

    let (_, r) = rpc(&app, call(4, "get_session", args(json!({})))).await;
    let (is_err, index) = tool_text(&r);
    assert!(!is_err);
    assert!(index.contains("> shared by an agent"), "{index}");
    assert!(index.contains("- notes.md (6 B, 2 version(s)"), "{index}");
    assert!(index.contains("Note: draft"), "{index}");

    let (_, r) = rpc(
        &app,
        call(5, "read_text_file", args(json!({ "name": "notes.md" }))),
    )
    .await;
    assert_eq!(tool_text(&r), (false, "second".to_string()));
    let (_, r) = rpc(
        &app,
        call(
            6,
            "read_text_file",
            args(json!({ "name": "notes.md", "tag": "v1" })),
        ),
    )
    .await;
    assert_eq!(tool_text(&r).1, "héllo wörld, first");
    // A cut in the middle of a multi-byte character drops the partial character only.
    let (_, r) = rpc(
        &app,
        call(
            7,
            "read_text_file",
            args(json!({ "name": "notes.md", "tag": "v1", "max_bytes": 2 })),
        ),
    )
    .await;
    assert_eq!(tool_text(&r).1, "h\n[cut off after 2 of 20 bytes]");

    let (_, r) = rpc(
        &app,
        call(
            8,
            "add_tag",
            args(json!({ "name": "notes.md", "tag": "stable", "version": 1 })),
        ),
    )
    .await;
    assert_eq!(
        tool_text(&r),
        (
            false,
            "Version 1 of notes.md now has tags: stable, v1.".to_string()
        )
    );
    let (_, r) = rpc(
        &app,
        call(
            9,
            "add_tag",
            args(json!({ "name": "notes.md", "tag": "x", "version": 9 })),
        ),
    )
    .await;
    assert_eq!(
        (tool_text(&r).0, tool_text(&r).1.contains("no version 9")),
        (true, true)
    );
    let (_, r) = rpc(
        &app,
        call(
            10,
            "set_note",
            args(json!({ "name": "notes.md", "note": "final" })),
        ),
    )
    .await;
    assert!(!tool_text(&r).0);
    let (_, r) = rpc(
        &app,
        call(
            11,
            "set_description",
            args(json!({ "description": "new text" })),
        ),
    )
    .await;
    assert!(!tool_text(&r).0);

    // Failures come back as tool errors the agent can read, not as transport errors.
    let (status, r) = rpc(&app, call(12, "get_session", json!({ "code": "k7m3q" }))).await;
    assert_eq!(
        (status, tool_text(&r)),
        (StatusCode::OK, (true, "not found".to_string()))
    );
    let (_, r) = rpc(&app, call(13, "get_session", json!({}))).await;
    assert_eq!(tool_text(&r), (true, "missing argument: code".to_string()));
    let (_, r) = rpc(&app, call(14, "no_such_tool", json!({}))).await;
    assert!(tool_text(&r).1.contains("unknown tool"));
    let (_, r) = rpc(
        &app,
        call(15, "read_text_file", args(json!({ "name": "absent.txt" }))),
    )
    .await;
    assert_eq!(tool_text(&r), (true, "not found".to_string()));

    raw(&app, Method::DELETE, &format!("/api/s/{code}"), &[], b"").await;
}

#[tokio::test]
async fn mcp_refuses_binary_files() {
    let (app, _f) = app().await;
    let code = new_session(&app).await;
    let url = format!("/api/s/{code}/upload?name=blob.bin");
    raw(&app, Method::PUT, &url, &[], &[0xff, 0xfe, 0x00, 0x80]).await;
    let (_, r) = rpc(
        &app,
        call(
            1,
            "read_text_file",
            json!({ "code": code, "name": "blob.bin" }),
        ),
    )
    .await;
    let (is_err, text) = tool_text(&r);
    assert!(is_err && text.contains("not a text file"), "{text}");
    raw(&app, Method::DELETE, &format!("/api/s/{code}"), &[], b"").await;
}
