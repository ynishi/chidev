//! A part of the model that a reader can be shown without the rest: the assets of one kind, or the change requests.

/// The sections, written once: each variant with its doc and its word. The enum, [`ModelSection::ALL`] and
/// [`ModelSection::word`] are all made from this one list, so a section cannot be in one and missing from another.
/// (The domain takes no serde or derive crate for this; a `macro_rules!` is enough.)
macro_rules! sections {
    ($($(#[doc = $doc:literal])* $variant:ident => $word:literal,)*) => {
        /// A part of the model. A rule that reads the model names the parts it reads, and an agent asks for the parts
        /// it judges from; the Contexts themselves are always there, so they are not a section.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ModelSection {
            $($(#[doc = $doc])* $variant,)*
        }

        impl ModelSection {
            /// Every section, in the order they are written.
            pub const ALL: &'static [ModelSection] = &[$(ModelSection::$variant,)*];

            /// The word for it, as a configuration and an operation write it. A saved external run stamps the words
            /// of the sections it read, so a word is never changed.
            pub fn word(self) -> &'static str {
                match self {
                    $(ModelSection::$variant => $word,)*
                }
            }
        }
    };
}

sections! {
    /// The Contracts of each Context, with their Clauses.
    Contracts => "contracts",
    /// The Interfaces of each Context, with their source and Items.
    Interfaces => "interfaces",
    /// The Harnesses of each Context, with their source and Cases.
    Harnesses => "harnesses",
    /// The change requests, which belong to the Project and not to a Context.
    ChangeRequests => "change_requests",
}

impl ModelSection {
    /// The section a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<ModelSection> {
        Self::ALL.iter().copied().find(|s| s.word() == word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_round_trip_and_are_the_ones_a_configuration_writes() {
        for s in ModelSection::ALL {
            assert_eq!(ModelSection::from_word(s.word()), Some(*s));
        }
        let words: Vec<_> = ModelSection::ALL.iter().map(|s| s.word()).collect();
        assert_eq!(
            words,
            ["contracts", "interfaces", "harnesses", "change_requests"]
        );
        assert_eq!(ModelSection::from_word("contract"), None);
    }
}
