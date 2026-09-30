use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::session::Session;

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("code already taken")]
    CodeTaken,
    #[error("name already exists in this folder")]
    NameTaken,
    #[error("limit reached: {0}")]
    LimitReached(String),
    #[error("not found")]
    NotFound,
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("storage error: {0}")]
    Storage(String),
}

/// Storage of sessions. Every method takes `now` explicitly so time is testable.
#[async_trait]
pub trait SessionRepo: Send + Sync {
    async fn create(
        &self,
        code: &str,
        description: &str,
        now: DateTime<Utc>,
    ) -> Result<Session, RepoError>;

    async fn get(&self, code: &str) -> Result<Option<Session>, RepoError>;

    async fn set_description(&self, code: &str, description: &str) -> Result<(), RepoError>;

    /// Records activity. Skips the write if the last touch is newer than `throttle_secs`.
    /// Returns whether a write happened.
    async fn touch(
        &self,
        code: &str,
        now: DateTime<Utc>,
        throttle_secs: i64,
    ) -> Result<bool, RepoError>;

    async fn delete(&self, code: &str) -> Result<(), RepoError>;

    /// Codes of sessions whose last activity is at or before `cutoff`.
    async fn idle_before(&self, cutoff: DateTime<Utc>) -> Result<Vec<String>, RepoError>;
}

impl RepoError {
    pub fn files_limit(max: i64) -> Self {
        Self::LimitReached(format!(
            "this session already holds the maximum of {max} files, delete one or start another session"
        ))
    }

    pub fn versions_limit(max: i64) -> Self {
        Self::LimitReached(format!(
            "this file already has the maximum of {max} versions, delete it or use another name"
        ))
    }

    pub fn bytes_limit(max: i64) -> Self {
        Self::LimitReached(format!(
            "this upload would take the session past its storage limit of {}, delete files first",
            crate::llms::human_size(max)
        ))
    }
}
