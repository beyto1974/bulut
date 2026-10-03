//! Uploads and downloads.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::{ApiError, ApiResult};
use super::AppState;
use crate::domain::node::{FileVersion, Node};
use crate::domain::upload::UploadStatus;
use crate::services::upload_service::NewUpload;

#[derive(Serialize)]
pub struct Stored {
    pub node: Node,
    pub version: FileVersion,
}

#[derive(Deserialize)]
pub struct InitUpload {
    pub parent: Option<Uuid>,
    pub name: String,
    pub size: i64,
    pub content_type: Option<String>,
    /// The file's own timestamp on the sender's machine.
    pub created_at: Option<DateTime<Utc>>,
}

pub async fn init_upload(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Json(body): Json<InitUpload>,
) -> ApiResult<(StatusCode, Json<UploadStatus>)> {
    let session = state.sessions.open(&code).await?;
    let status = state
        .uploads
        .init(
            &session.code,
            NewUpload {
                parent: body.parent,
                name: body.name,
                size: body.size,
                content_type: body.content_type,
                client_created_at: body.created_at,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(status)))
}

pub async fn upload_status(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<UploadStatus>> {
    let session = state.sessions.open(&code).await?;
    Ok(Json(state.uploads.status_of(&session.code, id).await?))
}

pub async fn put_part(
    State(state): State<AppState>,
    Path((code, id, number)): Path<(String, Uuid, i32)>,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let session = state.sessions.open(&code).await?;
    state
        .uploads
        .put_part(&session.code, id, number, body)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn complete_upload(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
) -> ApiResult<(StatusCode, Json<Stored>)> {
    let session = state.sessions.open(&code).await?;
    let (node, version) = state.uploads.complete(&session.code, id).await?;
    tracing::info!(session = %session.code, file = %node.name, version = version.version, size = version.size, "file stored");
    Ok((StatusCode::CREATED, Json(Stored { node, version })))
}

pub async fn abort_upload(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let session = state.sessions.open(&code).await?;
    state.uploads.abort(&session.code, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SimpleUploadQuery {
    pub name: String,
    pub parent: Option<Uuid>,
    /// Comma separated tags to put on the new version.
    pub tag: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// `PUT /api/s/{code}/upload?name=build.apk&tag=v1.4.2` with the raw file as the body.
pub async fn simple_upload(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<SimpleUploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<(StatusCode, Json<Stored>)> {
    let session = state.sessions.open(&code).await?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let tags: Vec<String> = q
        .tag
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    let (node, version) = state
        .uploads
        .upload_stream(
            &session.code,
            q.parent,
            &q.name,
            content_type,
            q.created_at,
            &tags,
            body.into_data_stream(),
        )
        .await?;
    tracing::info!(session = %session.code, file = %node.name, version = version.version, size = version.size, "file stored");
    Ok((StatusCode::CREATED, Json(Stored { node, version })))
}

#[derive(Deserialize)]
pub struct NamedQuery {
    pub name: String,
    pub parent: Option<Uuid>,
    pub tag: Option<String>,
    /// Ask for `Content-Disposition: inline`. Honoured for raster images only, see `inline_image`.
    pub inline: Option<String>,
}

#[derive(Deserialize)]
pub struct InlineQuery {
    pub inline: Option<String>,
}

pub async fn download_version(
    State(state): State<AppState>,
    Path((code, version)): Path<(String, Uuid)>,
    Query(q): Query<InlineQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = state.sessions.open(&code).await?;
    let (node, v) = state.tree.resolve_version(&session.code, version).await?;
    serve(&state, &node, &v, &headers, q.inline.is_some()).await
}

/// Download by name, optionally a tag: `?name=build.apk&tag=v1.4.1`. No tag means the latest version.
pub async fn download_named(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<NamedQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = state.sessions.open(&code).await?;
    let (node, v) = state
        .tree
        .resolve_named(&session.code, q.parent, &q.name, q.tag.as_deref())
        .await?;
    serve(&state, &node, &v, &headers, q.inline.is_some()).await
}

async fn serve(
    state: &AppState,
    node: &Node,
    v: &FileVersion,
    headers: &HeaderMap,
    want_inline: bool,
) -> ApiResult<Response> {
    let inline = want_inline && inline_image(state, v).await;
    let size = v.size as u64;
    let range = match headers.get(header::RANGE).and_then(|h| h.to_str().ok()) {
        Some(h) => match parse_range(h, size) {
            RangeRequest::Whole => None,
            RangeRequest::Partial(r) => Some(r),
            RangeRequest::Unsatisfiable => {
                return Ok((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [(header::CONTENT_RANGE, format!("bytes */{size}"))],
                )
                    .into_response())
            }
        },
        None => None,
    };
    let read = state.tree.open(v, range).await.map_err(ApiError::from)?;
    let (status, length) = match read.range {
        Some((s, e)) => (StatusCode::PARTIAL_CONTENT, e - s + 1),
        None => (StatusCode::OK, read.total),
    };
    let mut res = Response::new(Body::from_stream(read.stream));
    *res.status_mut() = status;
    let h = res.headers_mut();
    set(h, header::CONTENT_TYPE, &v.content_type);
    set(h, header::CONTENT_LENGTH, &length.to_string());
    set(h, header::ACCEPT_RANGES, "bytes");
    set(h, header::ETAG, &format!("\"{}\"", v.id));
    set(
        h,
        header::CONTENT_DISPOSITION,
        &content_disposition(&node.name, inline),
    );
    // Files are user content: never let the browser run or sniff them on this origin.
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(
        h,
        header::CONTENT_SECURITY_POLICY,
        "sandbox; default-src 'none'",
    );
    if let Some((s, e)) = read.range {
        set(
            h,
            header::CONTENT_RANGE,
            &format!("bytes {s}-{e}/{}", read.total),
        );
    }
    Ok(res)
}

fn set(h: &mut HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        h.insert(name, v);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RangeRequest {
    Whole,
    Partial((u64, u64)),
    Unsatisfiable,
}

/// Parses a single `Range: bytes=...` header against the file size. Anything that is not one
/// simple byte range is ignored and the whole file is served, as the HTTP spec allows.
pub fn parse_range(header: &str, size: u64) -> RangeRequest {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return RangeRequest::Whole;
    };
    if spec.contains(',') || size == 0 {
        return RangeRequest::Whole;
    }
    let Some((start, end)) = spec.split_once('-') else {
        return RangeRequest::Whole;
    };
    let (start, end) = match (start.trim(), end.trim()) {
        ("", n) => match n.parse::<u64>() {
            // Last n bytes.
            Ok(n) if n > 0 => (size.saturating_sub(n), size - 1),
            _ => return RangeRequest::Unsatisfiable,
        },
        (s, "") => match s.parse::<u64>() {
            Ok(s) => (s, size - 1),
            Err(_) => return RangeRequest::Whole,
        },
        (s, e) => match (s.parse::<u64>(), e.parse::<u64>()) {
            (Ok(s), Ok(e)) => (s, e.min(size - 1)),
            _ => return RangeRequest::Whole,
        },
    };
    if start >= size || start > end {
        return RangeRequest::Unsatisfiable;
    }
    RangeRequest::Partial((start, end))
}

/// Content types that may be shown inline. Raster images only: no SVG, no HTML, no PDF.
const INLINE_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];

/// The image type the first bytes of a file really have. The stored content type comes from the
/// uploader, so it is never trusted on its own.
pub fn sniff_image(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else if head.len() >= 12
        && &head[4..8] == b"ftyp"
        && matches!(&head[8..12], b"avif" | b"avis")
    {
        Some("image/avif")
    } else {
        None
    }
}

/// True when the stored type is an allowed raster image and the bytes agree with it.
async fn inline_image(state: &AppState, v: &FileVersion) -> bool {
    let declared = v
        .content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !INLINE_TYPES.contains(&declared.as_str()) || v.size == 0 {
        return false;
    }
    let Ok(read) = state.tree.open(v, Some((0, 15))).await else {
        return false;
    };
    let mut head = Vec::with_capacity(16);
    let mut stream = read.stream;
    while let Some(Ok(chunk)) = futures_util::StreamExt::next(&mut stream).await {
        head.extend_from_slice(&chunk);
        if head.len() >= 16 {
            break;
        }
    }
    sniff_image(&head) == Some(declared.as_str())
}

/// `attachment` (or `inline`) with an ASCII fallback and the exact UTF-8 name (RFC 6266 and 5987).
pub fn content_disposition(name: &str, inline: bool) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' && c != '%' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    let kind = if inline { "inline" } else { "attachment" };
    format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_ranges() {
        assert_eq!(parse_range("bytes=0-4", 10), RangeRequest::Partial((0, 4)));
        assert_eq!(parse_range("bytes=5-", 10), RangeRequest::Partial((5, 9)));
        assert_eq!(parse_range("bytes=-3", 10), RangeRequest::Partial((7, 9)));
        assert_eq!(parse_range("bytes=8-99", 10), RangeRequest::Partial((8, 9)));
        assert_eq!(parse_range("bytes=-99", 10), RangeRequest::Partial((0, 9)));
    }

    #[test]
    fn unsatisfiable_and_ignored_ranges() {
        assert_eq!(parse_range("bytes=10-12", 10), RangeRequest::Unsatisfiable);
        assert_eq!(parse_range("bytes=5-2", 10), RangeRequest::Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 10), RangeRequest::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-1,4-5", 10), RangeRequest::Whole);
        assert_eq!(parse_range("items=0-1", 10), RangeRequest::Whole);
        assert_eq!(parse_range("bytes=abc", 10), RangeRequest::Whole);
        assert_eq!(parse_range("bytes=0-4", 0), RangeRequest::Whole);
    }

    #[test]
    fn only_real_raster_images_are_sniffed() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n\0\0"), Some("image/png"));
        assert_eq!(
            sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]),
            Some("image/jpeg")
        );
        assert_eq!(sniff_image(b"GIF89a\x01\0"), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"\0\0\0\x1cftypavif\0\0"), Some("image/avif"));
        assert_eq!(
            sniff_image(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            None
        );
        assert_eq!(sniff_image(b"<html><script>"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn disposition_can_be_inline() {
        assert!(content_disposition("a.png", true).starts_with("inline; "));
    }

    #[test]
    fn disposition_keeps_ascii_and_encodes_the_rest() {
        assert_eq!(
            content_disposition("build.apk", false),
            "attachment; filename=\"build.apk\"; filename*=UTF-8''build.apk"
        );
        let d = content_disposition("café \"x\".txt", false);
        assert!(d.contains("filename=\"caf_ _x_.txt\""), "{d}");
        assert!(
            d.contains("filename*=UTF-8''caf%C3%A9%20%22x%22.txt"),
            "{d}"
        );
    }
}
