//! How serious a finding is, and the moments a check can run at.

use std::fmt;

/// How a check is held to account. The order is the order of seriousness, so a policy can ask whether a level is
/// below another (`Level::Warning < Level::Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    /// The check does not run.
    Off,
    /// Worth knowing; nothing is asked of anyone.
    Note,
    /// Something to deal with; reported, never refuses.
    Warning,
    /// Something wrong; a mandatory check at this level refuses the use case that found it.
    Error,
}

impl Level {
    /// The word for it, as a policy file and the output write it.
    pub fn word(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Note => "note",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }

    /// The level a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<Level> {
        [Level::Off, Level::Note, Level::Warning, Level::Error]
            .into_iter()
            .find(|l| l.word() == word)
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// The moment a check runs at: when a use case is about to keep a change (`Put`, `Pin`, `Resolve`), or when a
/// person asks for every check on the state as it is (`Check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hook {
    /// An asset is made or revised.
    Put,
    /// The code of an Interface is pinned.
    Pin,
    /// A change request is resolved.
    Resolve,
    /// On demand: nothing is kept.
    Check,
}

impl Hook {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Hook::Put => "put",
            Hook::Pin => "pin",
            Hook::Resolve => "resolve",
            Hook::Check => "check",
        }
    }
}

impl Hook {
    /// The hook a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<Hook> {
        [Hook::Put, Hook::Pin, Hook::Resolve, Hook::Check]
            .into_iter()
            .find(|h| h.word() == word)
    }
}

impl fmt::Display for Hook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_ordered_by_seriousness() {
        assert!(Level::Off < Level::Note && Level::Note < Level::Warning);
        assert!(Level::Warning < Level::Error);
    }

    #[test]
    fn a_level_is_read_back_from_its_word() {
        for l in [Level::Off, Level::Note, Level::Warning, Level::Error] {
            assert_eq!(Level::from_word(l.word()), Some(l));
        }
        assert_eq!(Level::from_word("fatal"), None);
    }
}
