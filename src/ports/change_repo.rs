use async_trait::async_trait;

use crate::domain::change::ChangePage;
use crate::ports::session_repo::RepoError;

/// Read side of the per-session change log. The write side lives inside the repositories that
/// change data, in the same transaction as the change.
#[async_trait]
pub trait ChangeRepo: Send + Sync {
    /// Events after `after`, oldest first, at most `limit`. `NotFound` if the session is gone.
    async fn since(&self, session: &str, after: i64, limit: i64) -> Result<ChangePage, RepoError>;

    /// Sequence number of the newest event (0 for a session without any).
    async fn head(&self, session: &str) -> Result<i64, RepoError>;
}
