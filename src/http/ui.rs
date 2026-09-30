//! The web UI (static files embedded in the binary) and the QR code of a session link.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use super::error::ApiResult;
use super::AppState;
use crate::domain::short_code::ShortCode;
use crate::services::error::ServiceError;

const INDEX: &str = include_str!("../../web/index.html");
const CSS: &str = include_str!("../../web/app.css");
const JS: &str = include_str!("../../web/app.js");

/// Scripts and styles only from this origin, no framing, nothing else loaded.
const PAGE_CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; \
    connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'";

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(state: &AppState) -> Response {
    let cfg = &state.config;
    let config = json!({
        "env": cfg.app_env.as_str(),
        "version": state.version,
        "codeLength": cfg.code_length,
    })
    .to_string();
    // Non-production environments are named in the title so tabs cannot be mixed up.
    let title = if cfg.app_env.is_production() {
        "Bulut".to_string()
    } else {
        format!("Bulut ({})", cfg.app_env.as_str())
    };
    let html = INDEX
        .replace("{{TITLE}}", &html_escape(&title))
        .replace("{{CONFIG}}", &html_escape(&config));
    let mut res = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PAGE_CSP),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

pub async fn home(State(state): State<AppState>) -> Response {
    page(&state)
}

/// Any single path segment that looks like a code gets the app, which loads the session itself.
/// Everything else (favicon.ico, robots.txt, ...) is a plain 404.
pub async fn session_page(State(state): State<AppState>, Path(code): Path<String>) -> Response {
    if ShortCode::parse(&code, state.config.code_length).is_err() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    page(&state)
}

pub async fn css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        CSS,
    )
        .into_response()
}

pub async fn js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        JS,
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct QrQuery {
    pub download: Option<String>,
}

/// QR code of `BASE_URL/<code>`.
pub async fn qr(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<QrQuery>,
) -> ApiResult<Response> {
    let session = state.sessions.open(&code).await?;
    let link = format!("{}/{}", state.config.base_url, session.code);
    let svg = crate::qr::svg(&link).map_err(|e| ServiceError::Storage(e.to_string()))?;
    let mut res = ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response();
    if q.download.is_some() {
        let value = format!("attachment; filename=\"bulut-{}.svg\"", session.code);
        if let Ok(v) = HeaderValue::from_str(&value) {
            res.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_special_characters() {
        assert_eq!(
            html_escape(r#"<a href="x">&'"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }
}
