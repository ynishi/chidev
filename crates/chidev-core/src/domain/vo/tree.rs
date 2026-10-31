//! The working tree's copy of an asset, as a value. The text of a placed asset is held twice: by the model (the code
//! a Pin accepted, the source of a Harness) and by the file the project's layout places it in. The model never reads
//! the file, so a copy that a person edited is a fact about the tree that nothing in the model records. A check reads
//! it as what the tree is *now* (as it reads [`SettingsNow`](super::measure::SettingsNow)), not as something kept:
//! the copy is a cache of the model's text, and a cache is looked at, not trusted.
//!
//! What a port read of a copy is [`CopyRead`]. What a copy is against the text the model holds is [`CodeNow`] for
//! the code of an Interface (against its Pin) and [`FileNow`] for the source of a Harness; both are made from a
//! [`CopyRead`] by one function each, the only places that compare digests. [`TreeNow`] is the copies of a whole
//! Project, and what it says of an asset it holds no copy of is said in one place.

use std::collections::BTreeMap;
use std::fmt;

use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{HarnessId, InterfaceId};

/// What was read of the file that holds the copy of one asset, as a port gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyRead {
    /// No file is laid out for the asset.
    Unplaced,
    /// A file is laid out for it, and it is not there.
    Missing,
    /// A file is laid out for it, and it cannot be read as text. The text says why, in the words of the reader.
    Unreadable(String),
    /// The file was read: the digest of its bytes, which are UTF-8 text. It is the digest a Pin makes of the same
    /// text, so the two are equal when the texts are.
    Read(Digest),
}

impl CopyRead {
    /// Whether the copy was read and its digest is not `held`. The one place that compares a copy with the text the
    /// model holds; a copy that was not read differs from nothing.
    fn differs_from(&self, held: Digest) -> bool {
        matches!(self, CopyRead::Read(read) if *read != held)
    }
}

/// What the copy of the code of an Interface is, against the code its Pin accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeNow {
    /// No file is laid out for the Interface: there is no copy to look at.
    Unplaced,
    /// A copy is there and the Interface has no Pin, so there is nothing accepted to compare it with (`pin.unpinned`
    /// says so).
    Unpinned,
    /// A file is laid out for the Interface and it is not there.
    Missing,
    /// A file is laid out for the Interface and it cannot be read as text. The text says why.
    Unreadable(String),
    /// The copy is the code the Pin accepted.
    Same,
    /// The copy is other text than the code the Pin accepted.
    Moved {
        /// What is known of the Items the copy declares.
        declared: DeclaredNow,
    },
}

/// What the Items of a copy that moved are known to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredNow {
    /// The project does not look at the Items of a copy: it is known that the copy moved, and not how.
    Off,
}

impl DeclaredNow {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            DeclaredNow::Off => "off",
        }
    }
}

/// The copy of the code of an Interface, observed: settled, or moved and still to be told what it declares. A moved
/// copy is made only by [`CodeNow::observe`], and becomes a [`CodeNow`] only by saying what is known of its Items, so
/// that a [`CodeNow::Moved`] always says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// The copy is, in every way that matters, known.
    Settled(CodeNow),
    /// The copy is other text than the code accepted.
    Moved(MovedCode),
}

/// A copy of the code of an Interface that is other text than the code its Pin accepted, not yet told what it
/// declares. It cannot be made outside [`CodeNow::observe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedCode(());

impl MovedCode {
    /// The copy as it stands when the project does not look at the Items of a copy.
    pub fn without_items(self) -> CodeNow {
        CodeNow::Moved {
            declared: DeclaredNow::Off,
        }
    }
}

impl CodeNow {
    /// The copy `copy` of the code of an Interface whose Pin accepted code of the digest `pin` (none: it has no
    /// Pin). What the port read decides first, so that a file that is missing or unreadable is told whether the
    /// Interface is pinned or not; the Pin is looked at only for a copy that was read.
    pub fn observe(pin: Option<Digest>, copy: CopyRead) -> Observed {
        match copy {
            CopyRead::Unplaced => Observed::Settled(CodeNow::Unplaced),
            CopyRead::Missing => Observed::Settled(CodeNow::Missing),
            CopyRead::Unreadable(why) => Observed::Settled(CodeNow::Unreadable(why)),
            read @ CopyRead::Read(_) => match pin {
                None => Observed::Settled(CodeNow::Unpinned),
                Some(held) if read.differs_from(held) => Observed::Moved(MovedCode(())),
                Some(_) => Observed::Settled(CodeNow::Same),
            },
        }
    }
}

/// What the copy of the source of a Harness is, against the source the model holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileNow {
    /// No file is laid out for the Harness.
    Unplaced,
    /// A file is laid out for the Harness and it is not there.
    Missing,
    /// A file is laid out for the Harness and it cannot be read as text. The text says why.
    Unreadable(String),
    /// The copy is the source the model holds.
    Same,
    /// The copy is other text than the source the model holds.
    Moved,
}

impl FileNow {
    /// The copy `copy` of the source of a Harness whose source in the model has the digest `source`.
    pub fn observe(source: Digest, copy: CopyRead) -> FileNow {
        match copy {
            CopyRead::Unplaced => FileNow::Unplaced,
            CopyRead::Missing => FileNow::Missing,
            CopyRead::Unreadable(why) => FileNow::Unreadable(why),
            read if read.differs_from(source) => FileNow::Moved,
            CopyRead::Read(_) => FileNow::Same,
        }
    }
}

/// The copies of the placed assets of a Project, as they are now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeNow {
    interfaces: BTreeMap<InterfaceId, CodeNow>,
    harnesses: BTreeMap<HarnessId, FileNow>,
}

impl TreeNow {
    /// No copy of anything: every asset is unplaced.
    pub const fn empty() -> TreeNow {
        TreeNow {
            interfaces: BTreeMap::new(),
            harnesses: BTreeMap::new(),
        }
    }

    /// The copies `interfaces` and `harnesses` are of.
    pub fn new(
        interfaces: BTreeMap<InterfaceId, CodeNow>,
        harnesses: BTreeMap<HarnessId, FileNow>,
    ) -> TreeNow {
        TreeNow {
            interfaces,
            harnesses,
        }
    }

    /// The copy of the code of `id`. An Interface that is not among the copies has none to look at: it is
    /// unplaced. This is the one place that says so.
    pub fn interface(&self, id: InterfaceId) -> CodeNow {
        self.interfaces
            .get(&id)
            .cloned()
            .unwrap_or(CodeNow::Unplaced)
    }

    /// The copy of the source of `id`; unplaced when it is not among the copies (see [`TreeNow::interface`]).
    pub fn harness(&self, id: HarnessId) -> FileNow {
        self.harnesses
            .get(&id)
            .cloned()
            .unwrap_or(FileNow::Unplaced)
    }
}

/// A file of the working tree, by its path relative to the project's root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TreePlace(String);

impl TreePlace {
    /// The file at `path`, a path that was checked to stay under the project's root where it was read.
    pub fn new(path: String) -> TreePlace {
        TreePlace(path)
    }
}

impl fmt::Display for TreePlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;

    fn d(word: &str) -> Digest {
        Digest::of(word.as_bytes())
    }

    fn interface(n: u128) -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(n))
    }

    fn harness(n: u128) -> HarnessId {
        HarnessId::from_uuid(test_uuid(n))
    }

    #[test]
    fn a_copy_the_pin_accepted_is_the_same_and_another_text_has_moved() {
        let settled = |pin, copy| CodeNow::observe(pin, copy);
        assert_eq!(
            settled(Some(d("a")), CopyRead::Read(d("a"))),
            Observed::Settled(CodeNow::Same)
        );
        assert!(matches!(
            settled(Some(d("a")), CopyRead::Read(d("b"))),
            Observed::Moved(_)
        ));
        // a copy with no Pin is compared with nothing
        assert_eq!(
            settled(None, CopyRead::Read(d("a"))),
            Observed::Settled(CodeNow::Unpinned)
        );
    }

    #[test]
    fn what_the_port_could_not_read_is_told_whether_or_not_the_interface_is_pinned() {
        for pin in [None, Some(d("a"))] {
            assert_eq!(
                CodeNow::observe(pin, CopyRead::Unplaced),
                Observed::Settled(CodeNow::Unplaced)
            );
            // a missing file is not hidden behind "unpinned"
            assert_eq!(
                CodeNow::observe(pin, CopyRead::Missing),
                Observed::Settled(CodeNow::Missing)
            );
            assert_eq!(
                CodeNow::observe(pin, CopyRead::Unreadable("not text".into())),
                Observed::Settled(CodeNow::Unreadable("not text".into()))
            );
        }
    }

    #[test]
    fn a_moved_copy_becomes_a_code_only_by_saying_what_is_known_of_its_items() {
        let Observed::Moved(moved) = CodeNow::observe(Some(d("a")), CopyRead::Read(d("b"))) else {
            panic!("the copy moved");
        };
        assert_eq!(
            moved.without_items(),
            CodeNow::Moved {
                declared: DeclaredNow::Off
            }
        );
        assert_eq!(DeclaredNow::Off.word(), "off");
    }

    #[test]
    fn the_source_of_a_harness_is_compared_with_the_source_the_model_holds() {
        assert_eq!(
            FileNow::observe(d("a"), CopyRead::Read(d("a"))),
            FileNow::Same
        );
        assert_eq!(
            FileNow::observe(d("a"), CopyRead::Read(d("b"))),
            FileNow::Moved
        );
        assert_eq!(
            FileNow::observe(d("a"), CopyRead::Unplaced),
            FileNow::Unplaced
        );
        assert_eq!(
            FileNow::observe(d("a"), CopyRead::Missing),
            FileNow::Missing
        );
        assert_eq!(
            FileNow::observe(d("a"), CopyRead::Unreadable("why".into())),
            FileNow::Unreadable("why".into())
        );
    }

    #[test]
    fn an_asset_that_is_not_among_the_copies_is_unplaced() {
        let tree = TreeNow::new(
            BTreeMap::from([(interface(1), CodeNow::Same)]),
            BTreeMap::from([(harness(2), FileNow::Moved)]),
        );
        assert_eq!(tree.interface(interface(1)), CodeNow::Same);
        assert_eq!(tree.harness(harness(2)), FileNow::Moved);
        assert_eq!(tree.interface(interface(9)), CodeNow::Unplaced);
        assert_eq!(tree.harness(harness(9)), FileNow::Unplaced);
        assert_eq!(TreeNow::empty().interface(interface(1)), CodeNow::Unplaced);
        assert_eq!(TreeNow::empty(), TreeNow::default());
    }

    #[test]
    fn a_place_is_ordered_and_written_as_its_path() {
        let (a, b) = (
            TreePlace::new("a/x.rs".into()),
            TreePlace::new("b.rs".into()),
        );
        assert!(a < b);
        assert_eq!(a.to_string(), "a/x.rs");
    }
}
