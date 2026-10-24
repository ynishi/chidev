//! The time, from the system clock; and a clock that stands still, for tests.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::ports::Clock;
use crate::domain::vo::timestamp::Timestamp;

/// The system clock: milliseconds since the Unix epoch. A system time before the epoch reads as the epoch.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        Timestamp::from_millis(millis)
    }
}

/// A clock that always says the same time: for tests, so that a record's time is known.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixed_clock_stands_still() {
        let clock = FixedClock(Timestamp::from_millis(7));
        assert_eq!(clock.now(), Timestamp::from_millis(7));
        assert_eq!(clock.now(), clock.now());
    }

    #[test]
    fn the_system_clock_is_after_the_year_2020_and_does_not_go_back() {
        let first = SystemClock.now();
        assert!(first.millis() > 1_577_836_800_000);
        assert!(SystemClock.now() >= first);
    }
}
