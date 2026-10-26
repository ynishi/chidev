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

/// The moment a check runs at: when a use case is about to keep a change (`Edit`, `Pin`, `Resolve`), or when a
/// person asks for every check on the state as it is (`Check`).
///
/// A moment is not a use case. The use cases that change the model are the triggers of
/// [`crate::app::commit::Trigger`], and each of them is held to exactly one moment, which its type gives. An edit
/// and a confirmation are held to different things. `Edit` cannot finish the model in one step: it reports, and a
/// mandatory check refuses, only what the edit introduced (the findings of the model after it that the model
/// before it did not have). A check that runs there is a change, one that does not is a state (see
/// [`Descriptor::kind`](super::descriptor::Descriptor::kind)). `Pin`, `Resolve` and `Check` confirm or ask: they
/// judge the whole model as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Moment {
    /// The model is edited: an asset, a Context, a change request or the Policy is made or revised. Reports only
    /// what the edit introduced.
    Edit,
    /// The code of an Interface is pinned.
    Pin,
    /// A change request is resolved.
    Resolve,
    /// On demand: nothing is kept.
    Check,
}

impl Moment {
    /// Every moment, in the order of the type.
    pub const ALL: [Moment; 4] = [Moment::Edit, Moment::Pin, Moment::Resolve, Moment::Check];

    /// The word for it, as a configuration file and the output write it.
    pub fn word(self) -> &'static str {
        match self {
            Moment::Edit => "edit",
            Moment::Pin => "pin",
            Moment::Resolve => "resolve",
            Moment::Check => "check",
        }
    }

    /// The word a moment is kept by: in a stored digest, and in the record of each check run. It does not change: a
    /// digest that changed would make every saved result of an external check stale at once, and a record a chidev
    /// cannot read would make the whole record unreadable to it. `Edit` is `put` here and for good, whatever
    /// [`Moment::word`] says; the word was `put` when the digests and records that exist were made.
    pub fn digest_key(self) -> &'static str {
        match self {
            Moment::Edit => "put",
            Moment::Pin => "pin",
            Moment::Resolve => "resolve",
            Moment::Check => "check",
        }
    }

    /// The moment a word names: the word of a moment, or why it is none. `put` was the word of `Edit` and is
    /// refused with the new word, so that a configuration written for the old one is told what to write.
    pub fn parse(word: &str) -> Result<Moment, MomentWordError> {
        if let Some(m) = Moment::ALL.into_iter().find(|m| m.word() == word) {
            return Ok(m);
        }
        if word == RENAMED_PUT {
            return Err(MomentWordError::Renamed {
                from: RENAMED_PUT,
                to: Moment::Edit,
            });
        }
        Err(MomentWordError::Unknown {
            word: word.to_string(),
        })
    }

    /// The moments, as a message lists them: `edit, pin, resolve or check`.
    pub fn listed() -> String {
        let words: Vec<&str> = Moment::ALL.iter().map(|m| m.word()).collect();
        match words.split_last() {
            Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
            None => String::new(),
        }
    }
}

/// The word `Edit` had before it was `edit`.
const RENAMED_PUT: &str = "put";

/// The key under which the moments of a check are counted in a stored digest. It was `hooks` when the first digests
/// were made, and a digest that changed would make every saved result of an external check stale at once, so it is
/// `hooks` still, wherever a digest is made of the moments of a check (see [`Moment::digest_key`]).
pub const DIGEST_MOMENTS_KEY: &str = "hooks";

/// The message that stops a file or a request that still uses the key `old`, which is `new` now. Silence would be
/// worse: a key that is no longer read is ignored by a reader that does not refuse unknown keys, and refused as
/// "unknown" by one that does, without saying what to write. Every reader of a renamed key says it with this.
pub fn renamed_key(old: &str, new: &str) -> String {
    format!("`{old}` is now `{new}`")
}

impl fmt::Display for Moment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// Why a word is no moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MomentWordError {
    /// The word was one of a moment once; it names that moment now.
    Renamed {
        /// The word that was used.
        from: &'static str,
        /// The moment it is now.
        to: Moment,
    },
    /// The word is none.
    Unknown {
        /// The word.
        word: String,
    },
}

impl fmt::Display for MomentWordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MomentWordError::Renamed { from, to } => {
                write!(
                    f,
                    "`{from}` is now `{to}`; the moments are {}",
                    Moment::listed()
                )
            }
            MomentWordError::Unknown { word } => {
                write!(f, "`{word}` is not a moment; they are {}", Moment::listed())
            }
        }
    }
}

impl std::error::Error for MomentWordError {}

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

    #[test]
    fn a_moment_is_read_back_from_its_word() {
        for m in Moment::ALL {
            assert_eq!(Moment::parse(m.word()), Ok(m));
        }
    }

    #[test]
    fn the_old_word_of_edit_is_refused_with_the_new_one() {
        let err = Moment::parse("put").unwrap_err();
        assert_eq!(
            err,
            MomentWordError::Renamed {
                from: "put",
                to: Moment::Edit
            }
        );
        let said = err.to_string();
        assert!(said.contains("`put` is now `edit`"), "{said}");
        for m in Moment::ALL {
            assert!(said.contains(m.word()), "{said}");
        }
    }

    #[test]
    fn an_unknown_word_lists_the_moments() {
        let err = Moment::parse("save").unwrap_err();
        assert_eq!(
            err,
            MomentWordError::Unknown {
                word: "save".into()
            }
        );
        assert!(err.to_string().contains("edit, pin, resolve or check"));
    }

    #[test]
    fn the_digest_key_of_edit_is_the_word_it_had_when_digests_were_made() {
        assert_eq!(Moment::Edit.digest_key(), "put");
        for m in [Moment::Pin, Moment::Resolve, Moment::Check] {
            assert_eq!(m.digest_key(), m.word());
        }
    }

    #[test]
    fn a_renamed_key_is_said_with_its_new_name() {
        assert_eq!(renamed_key("hooks", "moments"), "`hooks` is now `moments`");
    }
}
