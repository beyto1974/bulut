//! Finds objects in the bucket that nothing in the database points to, and deletes them.
//!
//! Orphans appear when a session is purged while an upload into it completes, or when the
//! process dies between the object store finishing a file and the database recording it. The
//! per-session purge cannot see them because it works from the database.

use std::sync::Arc;

use chrono::Duration;

use crate::domain::short_code::ShortCode;
use crate::ports::blob_store::BlobStore;
use crate::ports::clock::Clock;
use crate::ports::node_repo::NodeRepo;
use crate::ports::upload_repo::UploadRepo;
use crate::services::error::ServiceError;

/// Objects asked from the store per request (the S3 maximum).
const PAGE: usize = 1000;

pub struct OrphanSweeper {
    pub blobs: Arc<dyn BlobStore>,
    pub nodes: Arc<dyn NodeRepo>,
    pub uploads: Arc<dyn UploadRepo>,
    pub clock: Arc<dyn Clock>,
    /// Objects younger than this are never touched: a file can be in the store a moment before its
    /// row exists, and a retried `complete` can need it for as long as its upload is kept.
    pub grace: Duration,
    pub code_length: usize,
}

/// Only objects named like ours (`<session code>/<something>`) are candidates, so a bucket that
/// something else also writes to is left alone.
fn is_ours(key: &str, code_length: usize) -> bool {
    key.split_once('/').is_some_and(|(code, rest)| {
        !rest.is_empty()
            && ShortCode::parse(code, code_length).is_ok_and(|parsed| parsed.as_str() == code)
    })
}

impl OrphanSweeper {
    /// Deletes the orphans and returns how many there were.
    pub async fn sweep(&self) -> Result<usize, ServiceError> {
        let cutoff = self.clock.now() - self.grace;
        let mut deleted = 0;
        let mut after: Option<String> = None;
        loop {
            let page = self.blobs.list(after.as_deref(), PAGE).await?;
            let Some(last) = page.last() else { break };
            after = Some(last.key.clone());
            let full_page = page.len() >= PAGE;

            let candidates: Vec<String> = page
                .into_iter()
                .filter(|o| o.modified <= cutoff && is_ours(&o.key, self.code_length))
                .map(|o| o.key)
                .collect();
            let mut known = self.nodes.known_keys(&candidates).await?;
            known.extend(self.uploads.known_keys(&candidates).await?);
            let orphans: Vec<String> = candidates
                .into_iter()
                .filter(|k| !known.contains(k))
                .collect();
            if !orphans.is_empty() {
                self.blobs.delete_many(&orphans).await?;
                deleted += orphans.len();
            }
            if !full_page {
                break;
            }
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::is_ours;

    #[test]
    fn only_keys_shaped_like_ours_are_candidates() {
        assert!(is_ours("k7m3q/5f0c1c0e-0000-4000-8000-000000000000", 5));
        assert!(is_ours("k7m3q/thumb", 5));
        for foreign in [
            "",
            "notes.txt",
            "k7m3q",
            "k7m3q/",
            "K7M3Q/x",
            "k7m3o/x",
            "k7m3qq/x",
            "backups/2026/db.dump",
        ] {
            assert!(!is_ours(foreign, 5), "{foreign:?}");
        }
    }
}
