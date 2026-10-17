use std::sync::Arc;

use tokio::sync::Notify;

/// Wakes readers that wait for news. A wake-up is a hint: readers check the log again, so a
/// missed one only delays them.
pub trait ChangeBus: Send + Sync {
    /// Called after a change committed.
    fn publish(&self, session: &str);

    /// Register before reading the log, then wait on the result, so no wake-up slips in between.
    fn subscribe(&self, session: &str) -> Arc<Notify>;
}
