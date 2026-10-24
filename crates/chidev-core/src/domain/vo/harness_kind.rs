//! What a Harness is for: the tests that check the code against the Interface, or the simulation that checks it
//! against the Contracts from a program driven as a black box.

/// The kind of a Harness. It is content: a Harness that changes kind has a new revision, and a record that stands
/// on the Harness (a Pin, a measurement) stands on the old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum HarnessKind {
    /// Tests, run by the project's own test command. This is what a Harness is unless it says otherwise.
    #[default]
    Tests,
    /// A simulation script: one `sweep` Case and any number of `regress-<name>` Cases, run by chidev's `simulate`
    /// measure under the protocol `chidev-sim 2`.
    Sim,
}

impl HarnessKind {
    /// Every kind.
    pub const ALL: [HarnessKind; 2] = [HarnessKind::Tests, HarnessKind::Sim];

    /// The word for it, as the operations write it.
    pub fn word(self) -> &'static str {
        match self {
            HarnessKind::Tests => "tests",
            HarnessKind::Sim => "sim",
        }
    }

    /// The kind a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<HarnessKind> {
        Self::ALL.into_iter().find(|k| k.word() == word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_round_trip_and_tests_is_the_default() {
        for k in HarnessKind::ALL {
            assert_eq!(HarnessKind::from_word(k.word()), Some(k));
        }
        assert_eq!(HarnessKind::from_word("simulation"), None);
        assert_eq!(HarnessKind::default(), HarnessKind::Tests);
    }
}
