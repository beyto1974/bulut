use std::sync::Arc;

use chrono::Duration;

use crate::domain::session::Session;
use crate::domain::short_code::ShortCode;
use crate::ports::blob_store::BlobStore;
use crate::ports::clock::Clock;
use crate::ports::code_generator::CodeGenerator;
use crate::ports::node_repo::NodeRepo;
use crate::ports::session_repo::{RepoError, SessionRepo};
use crate::services::error::ServiceError;
use crate::services::upload_service::UploadService;

/// One activity write per session per minute is plenty for a 7 day window.
const TOUCH_THROTTLE_SECS: i64 = 60;
const MAX_DESCRIPTION: usize = 2000;

pub struct SessionService {
    pub sessions: Arc<dyn SessionRepo>,
    pub nodes: Arc<dyn NodeRepo>,
    pub blobs: Arc<dyn BlobStore>,
    pub uploads: Arc<UploadService>,
    pub codes: Arc<dyn CodeGenerator>,
    pub clock: Arc<dyn Clock>,
    pub code_length: usize,
    pub idle_ttl_days: u32,
}

impl SessionService {
    pub async fn create(&self, description: &str) -> Result<Session, ServiceError> {
        let description = description.trim();
        if description.chars().count() > MAX_DESCRIPTION {
            return Err(ServiceError::Invalid(format!(
                "description is longer than {MAX_DESCRIPTION} characters"
            )));
        }
        // Codes are short, so collisions are possible: retry with a new code.
        for _ in 0..10 {
            let code = self.codes.generate();
            match self
                .sessions
                .create(code.as_str(), description, self.clock.now())
                .await
            {
                Ok(s) => return Ok(s),
                Err(RepoError::CodeTaken) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(ServiceError::Conflict(
            "could not find a free code, try again".into(),
        ))
    }

    /// Looks a session up by code and records the activity. Every read path goes through here,
    /// so any view, download or API call keeps the session alive.
    pub async fn open(&self, raw_code: &str) -> Result<Session, ServiceError> {
        let code =
            ShortCode::parse(raw_code, self.code_length).map_err(|_| ServiceError::NotFound)?;
        let session = self
            .sessions
            .get(code.as_str())
            .await?
            .ok_or(ServiceError::NotFound)?;
        let now = self.clock.now();
        self.sessions
            .touch(code.as_str(), now, TOUCH_THROTTLE_SECS)
            .await?;
        Ok(session)
    }

    pub async fn set_description(
        &self,
        code: &str,
        description: &str,
    ) -> Result<Session, ServiceError> {
        let description = description.trim();
        if description.chars().count() > MAX_DESCRIPTION {
            return Err(ServiceError::Invalid(format!(
                "description is longer than {MAX_DESCRIPTION} characters"
            )));
        }
        let session = self.open(code).await?;
        self.sessions
            .set_description(&session.code, description)
            .await?;
        self.open(code).await
    }

    /// Removes the session, its rows and its stored objects.
    pub async fn purge(&self, code: &str) -> Result<(), ServiceError> {
        // Unfinished multipart uploads hold space in the store until they are aborted.
        self.uploads.discard_session(code).await?;
        let keys = self.nodes.all_keys(code).await?;
        self.blobs.delete_many(&keys).await?;
        self.sessions.delete(code).await?;
        Ok(())
    }

    pub async fn delete(&self, code: &str) -> Result<(), ServiceError> {
        let session = self.open(code).await?;
        self.purge(&session.code).await
    }

    /// Deletes every session idle for longer than the configured window. Returns the purged codes.
    pub async fn sweep_idle(&self) -> Result<Vec<String>, ServiceError> {
        let cutoff = self.clock.now() - Duration::days(i64::from(self.idle_ttl_days));
        let idle = self.sessions.idle_before(cutoff).await?;
        let mut purged = Vec::new();
        for code in idle {
            match self.purge(&code).await {
                Ok(()) => {
                    tracing::info!(session = %code, idle_days = self.idle_ttl_days, "purged idle session");
                    purged.push(code);
                }
                Err(e) => {
                    tracing::error!(session = %code, error = %e, "failed to purge idle session")
                }
            }
        }
        Ok(purged)
    }
}
