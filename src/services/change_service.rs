use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use crate::domain::change::ChangePage;
use crate::ports::change_bus::ChangeBus;
use crate::ports::change_repo::ChangeRepo;
use crate::services::error::ServiceError;
use crate::services::session_service::SessionService;

/// Events per answer. A client that is further behind asks again with the new cursor.
const PAGE: i64 = 500;

/// Longest a single wait may last, whatever the client asks for.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

pub struct ChangeService {
    pub sessions: Arc<SessionService>,
    pub changes: Arc<dyn ChangeRepo>,
    pub bus: Arc<dyn ChangeBus>,
    /// How often a waiting reader looks at the log even without a wake-up. This is what carries
    /// news from another process and what notices a deleted session.
    pub poll: Duration,
}

impl ChangeService {
    /// The cursor to start following from. The caller has opened the session already.
    pub async fn head(&self, code: &str) -> Result<i64, ServiceError> {
        Ok(self.changes.head(code).await?)
    }

    /// Events after `after`. If there are none, waits up to `wait` (at most [`MAX_WAIT`]) for
    /// some. Only an answer with news counts as activity of the session: a reader that waits
    /// on a quiet session does not keep it alive.
    pub async fn wait(
        &self,
        raw_code: &str,
        after: i64,
        wait: Duration,
    ) -> Result<ChangePage, ServiceError> {
        let code = self.sessions.peek(raw_code).await?.code;
        let deadline = Instant::now() + wait.min(MAX_WAIT);
        loop {
            // Register before looking, so news that lands in between still wakes this reader.
            let woken = self.bus.subscribe(&code);
            let notified = woken.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let page = self.changes.since(&code, after.max(0), PAGE).await?;
            if page.reset || !page.events.is_empty() {
                self.sessions.open(&code).await?;
                return Ok(page);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(page);
            }
            let _ = tokio::time::timeout(left.min(self.poll), notified).await;
        }
    }
}
