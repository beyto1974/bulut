use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::session::Session;

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("code already taken")]
    CodeTaken,
    #[error("not found")]
    NotFound,
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
        pin_hash: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Session, RepoError>;

    async fn get(&self, code: &str) -> Result<Option<Session>, RepoError>;

    async fn pin_hash(&self, code: &str) -> Result<Option<String>, RepoError>;

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
