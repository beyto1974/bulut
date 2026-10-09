use chrono::{DateTime, Utc};

/// A random URL that accepts a limited number of files into one session until it expires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadLink {
    pub id: String,
    pub session_code: String,
    pub files_total: i32,
    pub files_left: i32,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
