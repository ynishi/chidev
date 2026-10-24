//! When something was decided, as a value the domain is given.

use std::fmt;

/// A point in time, in milliseconds since the Unix epoch. The domain never reads a clock: the application takes
/// the time from its clock port and passes it in, so that a record carries the time it was made at and a test can
/// fix it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The point `millis` milliseconds after the epoch.
    pub fn from_millis(millis: u64) -> Timestamp {
        Timestamp(millis)
    }

    /// Milliseconds since the epoch.
    pub fn millis(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_is_ordered_milliseconds() {
        let early = Timestamp::from_millis(1);
        let late = Timestamp::from_millis(2);
        assert!(early < late);
        assert_eq!(late.millis(), 2);
        assert_eq!(late.to_string(), "2ms");
    }
}
