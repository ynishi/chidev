//! How sure a recommendation is.

use std::fmt;

use thiserror::Error;

/// Why a number is not a confidence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfidenceError {
    /// It is above 100.
    #[error("a confidence is from 0 to 100; {0} is above it")]
    TooHigh(u64),
}

/// How sure a recommendation is, as a percentage from 0 to 100. A project's decision Policy can be written against
/// it ("take the recommendation from 80 up"), so it is a number and not a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Confidence(u8);

impl Confidence {
    /// The percentage, from 0 to 100.
    pub fn percent(self) -> u8 {
        self.0
    }
}

impl TryFrom<u64> for Confidence {
    type Error = ConfidenceError;

    fn try_from(percent: u64) -> Result<Confidence, ConfidenceError> {
        match u8::try_from(percent) {
            Ok(p) if p <= 100 => Ok(Confidence(p)),
            _ => Err(ConfidenceError::TooHigh(percent)),
        }
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_confidence_is_a_percentage_from_0_to_100() {
        assert_eq!(Confidence::try_from(0).unwrap().percent(), 0);
        assert_eq!(Confidence::try_from(100).unwrap().percent(), 100);
        assert_eq!(Confidence::try_from(85).unwrap().to_string(), "85%");
        assert_eq!(
            Confidence::try_from(101),
            Err(ConfidenceError::TooHigh(101))
        );
        assert_eq!(
            Confidence::try_from(u64::MAX),
            Err(ConfidenceError::TooHigh(u64::MAX))
        );
    }
}
