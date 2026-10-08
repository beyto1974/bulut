use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::upload_link::UploadLink;
use crate::ports::session_repo::RepoError;

#[async_trait]
pub trait LinkRepo: Send + Sync {
    async fn create(&self, link: &UploadLink) -> Result<(), RepoError>;

    /// Use one slot of a link that has not expired. Atomic, so parallel requests cannot overshoot.
    async fn consume(&self, id: &str, now: DateTime<Utc>) -> Result<Option<UploadLink>, RepoError>;

    /// Give a slot back after an upload that failed before storing anything.
    async fn release(&self, id: &str) -> Result<(), RepoError>;

    /// Delete links that expired before `cutoff`.
    async fn purge_expired(&self, cutoff: DateTime<Utc>) -> Result<u64, RepoError>;
}
