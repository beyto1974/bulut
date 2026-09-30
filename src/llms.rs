//! Plain-text indexes for LLM agents: one per session and one describing the service.

use std::fmt::Write;

use chrono::{DateTime, Utc};

use crate::domain::node::NodeKind;
use crate::domain::session::Session;
use crate::services::tree_service::Walk;

/// Percent-encodes everything except unreserved characters, for use inside query strings.
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

pub fn human_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = b;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn when(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M UTC").to_string()
}

pub fn session_index(
    base_url: &str,
    session: &Session,
    idle_ttl_days: u32,
    max_files: u32,
    walk: &Walk,
) -> String {
    let code = &session.code;
    let files: Vec<_> = walk
        .items
        .iter()
        .filter(|i| i.entry.node.kind == NodeKind::File)
        .collect();
    let total: i64 = files
        .iter()
        .filter_map(|i| i.entry.latest.as_ref())
        .map(|v| v.size)
        .sum();

    let mut o = String::new();
    let _ = writeln!(o, "# Session {code}\n");
    if session.description.is_empty() {
        let _ = writeln!(o, "> No description.\n");
    } else {
        for line in session.description.lines() {
            let _ = writeln!(o, "> {line}");
        }
        o.push('\n');
    }
    let _ = writeln!(o, "- Link: {base_url}/{code}");
    let _ = writeln!(
        o,
        "- Files: {} of {max_files} ({} in the newest versions)",
        files.len(),
        human_size(total)
    );
    let _ = writeln!(
        o,
        "- Expires: {}. The session is deleted after {idle_ttl_days} days without any activity.",
        when(session.expires_at(idle_ttl_days))
    );
    let _ = writeln!(o, "\n## Files\n");
    if walk.items.is_empty() {
        let _ = writeln!(o, "This session is empty.");
    }
    for item in &walk.items {
        let node = &item.entry.node;
        let depth = item.path.matches('/').count();
        let pad = "  ".repeat(depth);
        let note = if node.note.is_empty() {
            String::new()
        } else {
            format!(" Note: {}", node.note.replace('\n', " "))
        };
        match node.kind {
            NodeKind::Folder => {
                let _ = writeln!(
                    o,
                    "{pad}- {}/ (folder, {} items, id {}){note}",
                    node.name, item.entry.child_count, node.id
                );
            }
            NodeKind::File => {
                let Some(latest) = &item.entry.latest else {
                    continue;
                };
                let _ = writeln!(
                    o,
                    "{pad}- {} ({}, {} version(s), uploaded {}, created {}, tags: {}){note}",
                    node.name,
                    human_size(latest.size),
                    item.entry.version_count,
                    when(latest.uploaded_at),
                    latest
                        .client_created_at
                        .map(when)
                        .unwrap_or_else(|| "unknown".into()),
                    latest.tags.join(", "),
                );
                let parent = node
                    .parent_id
                    .map(|p| format!("&parent={p}"))
                    .unwrap_or_default();
                let _ = writeln!(
                    o,
                    "{pad}  newest: {base_url}/api/s/{code}/download?name={}{parent}",
                    url_encode(&node.name)
                );
                if item.versions.len() > 1 {
                    for v in &item.versions {
                        let _ = writeln!(
                            o,
                            "{pad}  - v{}: {}, uploaded {}, tags: {}, {base_url}/api/s/{code}/versions/{}/download",
                            v.version,
                            human_size(v.size),
                            when(v.uploaded_at),
                            v.tags.join(", "),
                            v.id
                        );
                    }
                }
            }
        }
    }
    if walk.truncated {
        let _ = writeln!(o, "\n(The list is cut off. Use GET {base_url}/api/s/{code}/files for the complete listing.)");
    }
    let _ = writeln!(o, "\n## More\n");
    let _ = writeln!(o, "- JSON listing: {base_url}/api/s/{code}/files");
    let _ = writeln!(
        o,
        "- Upload a file: curl -T file '{base_url}/api/s/{code}/upload?name=file&tag=v1'"
    );
    let _ = writeln!(o, "- API description: {base_url}/openapi.json");
    let _ = writeln!(o, "- General guide: {base_url}/llms.txt");
    o
}

pub fn general_index(
    base_url: &str,
    code_length: usize,
    idle_ttl_days: u32,
    max_file_bytes: u64,
    max_files: u32,
) -> String {
    format!(
        "# Bulut

> A small dropbox for sessions. A session has a short code, a description and any number of files.
> Files can carry a note. Uploading a file with an existing name adds a new version, and a version can be tagged.

## Basics

- A session code is {code_length} characters of lowercase letters and digits, without look-alikes (no 0, o, 1, l, i).
- A session is deleted after {idle_ttl_days} days without any activity. Viewing, downloading and uploading all count as activity.
- Files can be up to {max_size}. Larger files are sent in chunks, small ones with a single request.
- A session holds at most {max_files} files. A new version of an existing file does not count as another file.
- Access is controlled in front of the app, send the credentials your deployment gave you with every request.

## Common calls

- Create a session: POST {base_url}/api/s with JSON {{\"description\": \"...\"}}
- Read a session as text: GET {base_url}/<code>/llms.txt
- Upload one file: PUT {base_url}/api/s/<code>/upload?name=build.apk&tag=v1.4.2 with the file as the body
- Download the newest version: GET {base_url}/api/s/<code>/download?name=build.apk
- Download a tagged version: GET {base_url}/api/s/<code>/download?name=build.apk&tag=v1.4.1
- List files as JSON, with ids: GET {base_url}/api/s/<code>/files
- Set a note: PATCH {base_url}/api/s/<code>/nodes/<id> with JSON {{\"note\": \"...\"}}
- Chunked upload: POST /api/s/<code>/uploads, PUT each part, GET the upload to see which parts exist, then POST .../complete

## Reference

- OpenAPI description: {base_url}/openapi.json
- MCP (streamable HTTP, JSON-RPC over POST): {base_url}/mcp
",
        max_size = human_size(max_file_bytes.min(i64::MAX as u64) as i64),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::node::{FileVersion, Node, NodeEntry};
    use crate::services::tree_service::WalkItem;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 30, h, 0, 0).unwrap()
    }

    fn node(name: &str, kind: NodeKind, parent: Option<Uuid>, note: &str) -> Node {
        Node {
            id: Uuid::new_v4(),
            session_code: "k7m3q".into(),
            parent_id: parent,
            kind,
            name: name.into(),
            note: note.into(),
            created_at: t(8),
        }
    }

    fn version(n: &Node, v: i32, size: i64, tags: &[&str], latest: bool) -> FileVersion {
        FileVersion {
            id: Uuid::new_v4(),
            node_id: n.id,
            version: v,
            size,
            content_type: "x/y".into(),
            sha256: None,
            blob_key: "k".into(),
            thumb_key: None,
            has_thumbnail: false,
            client_created_at: Some(t(7)),
            uploaded_at: t(9),
            is_latest: latest,
            tags: tags.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn sample() -> (Session, Walk) {
        let folder = node("fixtures", NodeKind::Folder, None, "only the failing ones");
        let apk = node("build.apk", NodeKind::File, None, "");
        let inner = node(
            "a b.pdf",
            NodeKind::File,
            Some(folder.id),
            "total row is an image",
        );
        let (v1, v2) = (
            version(&apk, 1, 47_900_000, &["v1.4.1"], false),
            version(&apk, 2, 48_200_000, &["latest", "v1.4.2"], true),
        );
        let vi = version(&inner, 1, 212_000, &["latest"], true);
        let items = vec![
            WalkItem {
                path: "fixtures".into(),
                entry: NodeEntry {
                    node: folder,
                    latest: None,
                    version_count: 0,
                    child_count: 1,
                },
                versions: vec![],
            },
            WalkItem {
                path: "fixtures/a b.pdf".into(),
                entry: NodeEntry {
                    node: inner,
                    latest: Some(vi.clone()),
                    version_count: 1,
                    child_count: 0,
                },
                versions: vec![vi],
            },
            WalkItem {
                path: "build.apk".into(),
                entry: NodeEntry {
                    node: apk,
                    latest: Some(v2.clone()),
                    version_count: 2,
                    child_count: 0,
                },
                versions: vec![v2, v1],
            },
        ];
        let session = Session {
            code: "k7m3q".into(),
            description: "Run 14\nfor the reviewer".into(),
            created_at: t(6),
            last_activity_at: t(10),
        };
        (
            session,
            Walk {
                items,
                truncated: false,
            },
        )
    }

    #[test]
    fn session_index_lists_files_versions_tags_and_dates() {
        let (session, walk) = sample();
        let text = session_index("https://bulut.dev", &session, 7, 100, &walk);
        assert!(text.starts_with("# Session k7m3q\n\n> Run 14\n> for the reviewer\n"));
        assert!(text.contains("- Link: https://bulut.dev/k7m3q"));
        assert!(text.contains("- Files: 2 of 100 (48.4 MB in the newest versions)"));
        assert!(text.contains("- Expires: 2026-10-07 10:00 UTC."));
        assert!(text.contains("- fixtures/ (folder, 1 items, id "));
        assert!(text.contains("Note: only the failing ones"));
        // Nested file is indented one level and its name is encoded in the download URL.
        assert!(text.contains("  - a b.pdf (212.0 KB, 1 version(s), uploaded 2026-09-30 09:00 UTC, created 2026-09-30 07:00 UTC, tags: latest) Note: total row is an image"));
        assert!(text.contains("name=a%20b.pdf&parent="));
        // Only files with several versions list them.
        assert!(text.contains("- build.apk (48.2 MB, 2 version(s)"));
        assert!(text.contains("  - v2: 48.2 MB, uploaded 2026-09-30 09:00 UTC, tags: latest, v1.4.2, https://bulut.dev/api/s/k7m3q/versions/"));
        assert!(text.contains("  - v1: 47.9 MB"));
        assert!(text.contains("newest: https://bulut.dev/api/s/k7m3q/download?name=build.apk\n"));
        assert!(
            text.contains("curl -T file 'https://bulut.dev/api/s/k7m3q/upload?name=file&tag=v1'")
        );
    }

    #[test]
    fn empty_and_truncated_sessions_say_so() {
        let (session, _) = sample();
        let empty = Walk {
            items: vec![],
            truncated: false,
        };
        assert!(
            session_index("http://x", &session, 7, 100, &empty).contains("This session is empty.")
        );
        let cut = Walk {
            items: vec![],
            truncated: true,
        };
        assert!(session_index("http://x", &session, 7, 100, &cut).contains("The list is cut off"));
    }

    #[test]
    fn sizes_and_encoding() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1_000), "1.0 KB");
        assert_eq!(human_size(1_000_000_000), "1.0 GB");
        assert_eq!(url_encode("a b/é.txt"), "a%20b%2F%C3%A9.txt");
    }

    #[test]
    fn general_index_mentions_the_key_facts() {
        let text = general_index("https://bulut.dev", 5, 7, 1_073_741_824, 100);
        assert!(text.contains("5 characters of lowercase letters and digits"));
        assert!(text.contains("after 7 days without any activity"));
        assert!(text.contains("up to 1.1 GB"));
        assert!(text.contains("at most 100 files"));
        assert!(text.contains("https://bulut.dev/openapi.json"));
        assert!(text.contains("https://bulut.dev/mcp"));
    }
}
