//! Wake-ups inside one process.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::ports::change_bus::ChangeBus;

#[derive(Default)]
pub struct InProcessBus {
    waiters: Mutex<HashMap<String, Arc<Notify>>>,
}

impl InProcessBus {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ChangeBus for InProcessBus {
    fn publish(&self, session: &str) {
        let map = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = map.get(session) {
            n.notify_waiters();
        }
    }

    fn subscribe(&self, session: &str) -> Arc<Notify> {
        let mut map = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
        // Nobody else holds an entry's Notify: drop it, so the map follows the live sessions.
        map.retain(|_, n| Arc::strong_count(n) > 1);
        map.entry(session.to_string()).or_default().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn publish_wakes_a_subscriber_of_that_session_only() {
        let bus = InProcessBus::new();
        let a = bus.subscribe("aaaaa");
        let b = bus.subscribe("bbbbb");
        let waiting = a.notified();
        tokio::pin!(waiting);
        waiting.as_mut().enable();
        bus.publish("aaaaa");
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("woken");
        let other = b.notified();
        tokio::pin!(other);
        other.as_mut().enable();
        bus.publish("aaaaa");
        assert!(tokio::time::timeout(Duration::from_millis(50), other)
            .await
            .is_err());
    }

    #[test]
    fn unused_entries_are_dropped() {
        let bus = InProcessBus::new();
        drop(bus.subscribe("aaaaa"));
        let _keep = bus.subscribe("bbbbb");
        assert_eq!(bus.waiters.lock().unwrap().len(), 1);
    }
}
