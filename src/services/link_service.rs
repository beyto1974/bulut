//! Upload links: random URLs that let a client without the proxy's bearer token send a limited
//! number of files into one session for a short time. The link grants nothing else.

use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use futures_util::Stream;
use rand::Rng;

use crate::domain::node::{FileVersion, Node};
use crate::domain::upload_link::UploadLink;
use crate::ports::clock::Clock;
use crate::ports::link_repo::LinkRepo;
use crate::services::error::ServiceError;
use crate::services::session_service::SessionService;
use crate::services::upload_service::UploadService;

/// Expired links are kept this long before they are deleted.
const PURGE_AFTER: i64 = 24;

pub struct LinkService {
    pub links: Arc<dyn LinkRepo>,
    pub sessions: Arc<SessionService>,
    pub uploads: Arc<UploadService>,
    pub clock: Arc<dyn Clock>,
    /// Most files one link can take (`UPLOAD_LINK_MAX_FILES`).
    pub max_files: u32,
    pub ttl: Duration,
}

fn random_id() -> String {
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| char::from_digit(rng.gen_range(0..16), 16).unwrap_or('0'))
        .collect()
}

/// Gives a link slot back if the upload future is dropped before it finishes.
struct SlotGuard {
    links: Arc<dyn LinkRepo>,
    id: String,
    armed: bool,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (links, id) = (self.links.clone(), std::mem::take(&mut self.id));
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                if let Err(e) = links.release(&id).await {
                    tracing::warn!(error = %e, "cannot give an upload link slot back");
                }
            });
        }
    }
}

impl LinkService {
    pub async fn create(&self, code: &str, files: Option<u32>) -> Result<UploadLink, ServiceError> {
        let session = self.sessions.open(code).await?;
        let files = files.unwrap_or(1);
        if files == 0 || files > self.max_files {
            return Err(ServiceError::Invalid(format!(
                "files must be between 1 and {}",
                self.max_files
            )));
        }
        let now: DateTime<Utc> = self.clock.now();
        // Housekeeping must not stop a link from being made.
        if let Err(e) = self
            .links
            .purge_expired(now - Duration::hours(PURGE_AFTER))
            .await
        {
            tracing::warn!(error = %e, "cannot purge expired upload links");
        }
        let link = UploadLink {
            id: random_id(),
            session_code: session.code,
            files_total: files as i32,
            files_left: files as i32,
            expires_at: now + self.ttl,
            created_at: now,
        };
        self.links.create(&link).await?;
        Ok(link)
    }

    /// Store one file through a link. A slot is used before the bytes are read, so parallel
    /// requests cannot exceed the count; it is given back when nothing was stored.
    pub async fn upload<S, E>(
        &self,
        id: &str,
        name: &str,
        content_type: Option<String>,
        tags: &[String],
        body: S,
    ) -> Result<(Node, FileVersion), ServiceError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send,
        E: std::fmt::Display,
    {
        let link = self
            .links
            .consume(id, self.clock.now())
            .await?
            .ok_or(ServiceError::NotFound)?;
        // A dropped request (client gone, proxy timeout) never reaches the code below: the guard
        // gives the slot back then.
        let mut guard = SlotGuard {
            links: self.links.clone(),
            id: id.to_string(),
            armed: true,
        };
        let stored = self
            .uploads
            .upload_stream(
                &link.session_code,
                None,
                name,
                content_type,
                None,
                tags,
                body,
            )
            .await;
        guard.armed = false;
        if stored.is_err() {
            if let Err(e) = self.links.release(id).await {
                tracing::warn!(error = %e, "cannot give an upload link slot back");
            }
        }
        stored
    }
}
