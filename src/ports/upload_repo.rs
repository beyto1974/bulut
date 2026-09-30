use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::upload::Upload;
use crate::ports::session_repo::RepoError;

#[async_trait]
pub trait UploadRepo: Send + Sync {
    async fn create(&self, upload: &Upload) -> Result<(), RepoError>;

    async fn get(&self, session: &str, id: Uuid) -> Result<Option<Upload>, RepoError>;

    async fn delete(&self, id: Uuid) -> Result<(), RepoError>;

    /// Uploads of one session, used to abort them when the session is purged.
    async fn for_session(&self, session: &str) -> Result<Vec<Upload>, RepoError>;

    /// Uploads started at or before `cutoff` that never finished.
    async fn started_before(&self, cutoff: DateTime<Utc>) -> Result<Vec<Upload>, RepoError>;
}
