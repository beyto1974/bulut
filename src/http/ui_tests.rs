//! The UI shell, its assets and the QR endpoint through the router.

use axum::http::{Method, StatusCode};

use super::file_tests::{new_session, raw};
use super::tests::app;

#[tokio::test]
async fn home_and_session_pages_serve_the_app_with_config_and_csp() {
    let (app, _f) = app().await;
    for path in ["/", "/k7m3q"] {
        let (status, h, body) = raw(&app, Method::GET, path, &[], b"").await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(h["content-type"], "text/html; charset=utf-8");
        let csp = h["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains("unsafe-inline"));
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            html.contains("<title>Bulut</title>"),
            "production title has no environment"
        );
        assert!(html.contains("data-config=\"{&quot;codeLength&quot;:5,&quot;env&quot;:&quot;production&quot;,&quot;version&quot;:&quot;9.9.9&quot;}\""), "{html}");
        assert!(!html.contains("{{"), "all placeholders are filled");
        assert!(html.contains("/assets/app.js") && html.contains("/assets/app.css"));
    }
}

#[tokio::test]
async fn things_that_are_not_codes_are_plain_404s() {
    let (app, _f) = app().await;
    for path in [
        "/favicon.ico",
        "/robots.txt",
        "/k7m3o",
        "/toolongcode",
        "/a",
    ] {
        let (status, h, _) = raw(&app, Method::GET, path, &[], b"").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            h.get("content-security-policy").is_none(),
            "{path} is not the app"
        );
    }
}

#[tokio::test]
async fn assets_are_served_with_the_right_types() {
    let (app, _f) = app().await;
    let (status, h, body) = raw(&app, Method::GET, "/assets/app.js", &[], b"").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h["content-type"], "text/javascript; charset=utf-8");
    assert!(String::from_utf8_lossy(&body).contains("sessionPage"));
    let (status, h, body) = raw(&app, Method::GET, "/assets/app.css", &[], b"").await;
    assert_eq!(
        (status, h["content-type"].to_str().unwrap()),
        (StatusCode::OK, "text/css; charset=utf-8")
    );
    assert!(String::from_utf8_lossy(&body).contains("--accent"));
}

#[tokio::test]
async fn every_response_carries_the_hardening_headers() {
    let (app, _f) = app().await;
    for path in [
        "/",
        "/healthz",
        "/assets/app.js",
        "/llms.txt",
        "/nothing/here",
    ] {
        let (_, h, _) = raw(&app, Method::GET, path, &[], b"").await;
        assert_eq!(h["x-content-type-options"], "nosniff", "{path}");
        assert_eq!(h["referrer-policy"], "no-referrer", "{path}");
    }
}

#[tokio::test]
async fn qr_code_encodes_the_session_link() {
    let (app, _f) = app().await;
    let code = new_session(&app).await;
    let (status, h, body) = raw(
        &app,
        Method::GET,
        &format!("/api/s/{code}/qr.svg"),
        &[],
        b"",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h["content-type"], "image/svg+xml");
    assert!(h.get("content-disposition").is_none());
    let expected = crate::qr::svg(&format!("http://localhost:8080/{code}")).unwrap();
    assert_eq!(
        String::from_utf8(body.to_vec()).unwrap(),
        expected,
        "it is the link, not something else"
    );

    let (_, h, _) = raw(
        &app,
        Method::GET,
        &format!("/api/s/{code}/qr.svg?download=1"),
        &[],
        b"",
    )
    .await;
    assert_eq!(
        h["content-disposition"],
        format!("attachment; filename=\"bulut-{code}.svg\"")
    );

    let (status, _, _) = raw(&app, Method::GET, "/api/s/k7m3q/qr.svg", &[], b"").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    raw(&app, Method::DELETE, &format!("/api/s/{code}"), &[], b"").await;
}
