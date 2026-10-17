use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    VersionAdded,
    FileUpdated,
    FileDeleted,
    FolderDeleted,
    TagAdded,
    TagRemoved,
    SessionUpdated,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VersionAdded => "version_added",
            Self::FileUpdated => "file_updated",
            Self::FileDeleted => "file_deleted",
            Self::FolderDeleted => "folder_deleted",
            Self::TagAdded => "tag_added",
            Self::TagRemoved => "tag_removed",
            Self::SessionUpdated => "session_updated",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "version_added" => Self::VersionAdded,
            "file_updated" => Self::FileUpdated,
            "file_deleted" => Self::FileDeleted,
            "folder_deleted" => Self::FolderDeleted,
            "tag_added" => Self::TagAdded,
            "tag_removed" => Self::TagRemoved,
            "session_updated" => Self::SessionUpdated,
            _ => return None,
        })
    }
}

/// One entry of a session's change log. A deleted file is gone from the tree, so the entry
/// carries its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub seq: i64,
    pub kind: ChangeKind,
    pub node_id: Option<Uuid>,
    pub version_id: Option<Uuid>,
    pub name: Option<String>,
    pub version: Option<i32>,
    pub tag: Option<String>,
    pub at: DateTime<Utc>,
}

/// What a client gets for a cursor. `seq` is the cursor for the next call. `reset` means the
/// cursor is older than the kept log (or from the future): list the session again and continue
/// from `seq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangePage {
    pub seq: i64,
    pub reset: bool,
    pub events: Vec<Change>,
}
