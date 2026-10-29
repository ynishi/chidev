//! What a Harness is for: the tests that check the code against the Interface, the simulation that checks it
//! against the Contracts from a program driven as a black box, or the examples a program is judged by.

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
    /// Examples: each Case is one example of what a program answers (its arguments, its input and what it is
    /// expected to give), held in the model as data. A Harness of examples has no code.
    Examples,
}

impl HarnessKind {
    /// Every kind.
    pub const ALL: [HarnessKind; 3] = [HarnessKind::Tests, HarnessKind::Sim, HarnessKind::Examples];

    /// The word for it, as the operations write it.
    pub fn word(self) -> &'static str {
        match self {
            HarnessKind::Tests => "tests",
            HarnessKind::Sim => "sim",
            HarnessKind::Examples => "examples",
        }
    }

    /// The kind a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<HarnessKind> {
        Self::ALL.into_iter().find(|k| k.word() == word)
    }

    /// The kind of code this is, when the Harness is code: `None` for examples, which have none.
    pub fn code(self) -> Option<CodeKind> {
        match self {
            HarnessKind::Tests => Some(CodeKind::Tests),
            HarnessKind::Sim => Some(CodeKind::Sim),
            HarnessKind::Examples => None,
        }
    }
}

/// The kinds of a Harness that is code. A Harness holds its content as code or as examples, never both, so the kind
/// of a Harness of code is a value of its own: a Harness that is code cannot be said to be a Harness of examples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum CodeKind {
    /// Tests, run by the project's own test command.
    #[default]
    Tests,
    /// A simulation script.
    Sim,
}

impl From<CodeKind> for HarnessKind {
    fn from(kind: CodeKind) -> HarnessKind {
        match kind {
            CodeKind::Tests => HarnessKind::Tests,
            CodeKind::Sim => HarnessKind::Sim,
        }
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
        assert_eq!(HarnessKind::ALL.len(), 3);
        assert_eq!(
            HarnessKind::from_word("examples"),
            Some(HarnessKind::Examples)
        );
    }

    #[test]
    fn a_code_kind_is_the_kind_it_is_and_examples_are_no_code() {
        for k in [CodeKind::Tests, CodeKind::Sim] {
            assert_eq!(HarnessKind::from(k).code(), Some(k));
        }
        assert_eq!(HarnessKind::Examples.code(), None);
    }
}
