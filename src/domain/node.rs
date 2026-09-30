use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    File,
    Folder,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "file" => Some(Self::File),
            "folder" => Some(Self::Folder),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: Uuid,
    pub session_code: String,
    pub parent_id: Option<Uuid>,
    pub kind: NodeKind,
    pub name: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

/// One uploaded version of a file. `client_created_at` is the file's own timestamp from the
/// sender's machine, `uploaded_at` is when it reached the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileVersion {
    pub id: Uuid,
    pub node_id: Uuid,
    pub version: i32,
    pub size: i64,
    pub content_type: String,
    pub sha256: Option<String>,
    #[serde(skip)]
    pub blob_key: String,
    #[serde(skip)]
    pub thumb_key: Option<String>,
    pub has_thumbnail: bool,
    pub client_created_at: Option<DateTime<Utc>>,
    pub uploaded_at: DateTime<Utc>,
    pub is_latest: bool,
    pub tags: Vec<String>,
}

/// A row in a folder listing: the node plus a summary of its latest version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeEntry {
    #[serde(flatten)]
    pub node: Node,
    pub latest: Option<FileVersion>,
    pub version_count: i64,
    /// Number of direct children, folders only.
    pub child_count: i64,
}

/// Names must be non-empty, at most 255 bytes, and cannot contain `/` or control characters.
pub fn validate_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name cannot be empty");
    }
    if name.len() > 255 {
        return Err("name is longer than 255 bytes");
    }
    // Control characters and the Unicode line and paragraph separators, which readers of a text listing
    // may treat as a line break.
    let breaks = |c: char| c.is_control() || c == '\u{2028}' || c == '\u{2029}';
    if name.contains('/') || name.chars().any(breaks) || name == "." || name == ".." {
        return Err("name contains a character that is not allowed");
    }
    Ok(name)
}

/// Tags are short labels such as `v1.4.2` or `latest`.
pub fn validate_tag(tag: &str) -> Result<String, &'static str> {
    let tag = tag.trim();
    if tag.is_empty() || tag.len() > 64 {
        return Err("tag must be 1 to 64 characters");
    }
    if tag
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == ',' || c == '/')
    {
        return Err("tag cannot contain spaces, commas or slashes");
    }
    Ok(tag.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_checked() {
        assert_eq!(validate_name("  build.apk ").unwrap(), "build.apk");
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("bad\nname").is_err());
        assert!(validate_name("line\u{2028}break").is_err());
        assert!(validate_name("para\u{2029}graph").is_err());
        assert!(validate_name(&"x".repeat(256)).is_err());
    }

    #[test]
    fn tags_reject_separators() {
        assert_eq!(validate_tag(" v1.4.2 ").unwrap(), "v1.4.2");
        assert!(validate_tag("two words").is_err());
        assert!(validate_tag("a,b").is_err());
        assert!(validate_tag("").is_err());
        assert!(validate_tag(&"t".repeat(65)).is_err());
    }

    #[test]
    fn kind_roundtrip() {
        assert_eq!(
            NodeKind::parse(NodeKind::File.as_str()),
            Some(NodeKind::File)
        );
        assert_eq!(NodeKind::parse("folder"), Some(NodeKind::Folder));
        assert_eq!(NodeKind::parse("x"), None);
    }
}
