use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Session {
    pub code: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
}

impl Session {
    /// When the session will be purged if nothing touches it.
    pub fn expires_at(&self, idle_ttl_days: u32) -> DateTime<Utc> {
        self.last_activity_at + Duration::days(i64::from(idle_ttl_days))
    }

    pub fn is_idle(&self, now: DateTime<Utc>, idle_ttl_days: u32) -> bool {
        self.expires_at(idle_ttl_days) <= now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn session(last: DateTime<Utc>) -> Session {
        Session {
            code: "k7m3q".into(),
            description: String::new(),
            created_at: last,
            last_activity_at: last,
        }
    }

    #[test]
    fn expires_after_ttl_days_of_inactivity() {
        let t0 = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
        let s = session(t0);
        assert_eq!(
            s.expires_at(7),
            Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0).unwrap()
        );
    }

    #[test]
    fn idle_only_once_ttl_has_passed() {
        let t0 = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
        let s = session(t0);
        assert!(!s.is_idle(t0 + Duration::days(7) - Duration::seconds(1), 7));
        assert!(s.is_idle(t0 + Duration::days(7), 7));
    }
}
