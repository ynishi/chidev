//! How many times something kept over time has changed.

use std::fmt;

/// The revision of an asset or of a definition inside one: 1 when it is made, one more each time its content
/// changes, and unchanged when it is given the same content again. A Pin or a citation records it to see later
/// that what it stood on has changed. The old content is not kept here; the history is git's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Revision(u64);

impl Revision {
    /// The revision of something just made.
    pub fn first() -> Revision {
        Revision(1)
    }

    /// The revision after this one.
    pub fn next(self) -> Revision {
        Revision(self.0 + 1)
    }

    /// The number, from 1.
    pub fn number(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revision_starts_at_one_and_counts_up() {
        assert_eq!(Revision::first().number(), 1);
        assert_eq!(Revision::first().next().next().number(), 3);
        assert_eq!(Revision::first().next().to_string(), "r2");
    }
}
