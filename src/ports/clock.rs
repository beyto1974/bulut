use chrono::{DateTime, Utc};
use std::sync::Mutex;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock tests can move by hand.
pub struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        Self(Mutex::new(start))
    }

    pub fn advance(&self, by: chrono::Duration) {
        let mut t = self.0.lock().unwrap();
        *t += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    #[test]
    fn manual_clock_advances() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let c = ManualClock::new(t0);
        assert_eq!(c.now(), t0);
        c.advance(Duration::days(8));
        assert_eq!(c.now(), t0 + Duration::days(8));
    }
}
