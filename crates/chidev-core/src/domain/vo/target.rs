//! What a citation or a change request points at, and what a change request asks of it (to revise or retire it, or
//! to keep it).

use std::fmt;

use super::id::{CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId};
use super::revision::Revision;

/// A thing a Case or a change request can point at. The targets form the tree of the assets:
///
/// ```text
/// Project
///  └ Context
///     ├ Contract  ─ Clause
///     ├ Interface ─ Item
///     └ Harness   ─ Case
/// ```
///
/// Each has a revision (see `service::cite::revision_of`). A container's revision moves when something directly
/// under it is added or removed, so that adding a thing is written as a change of what holds it. A target holds
/// the id alone; ids are given once in the whole Project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// The Project itself: its revision moves when a Context is added or removed.
    Project,
    /// A Context: its revision moves when one of its assets is added or removed, and when its uses or its basis
    /// change.
    Context(ContextId),
    /// A Contract.
    Contract(ContractId),
    /// An Interface.
    Interface(InterfaceId),
    /// A Harness.
    Harness(HarnessId),
    /// A Clause of a Contract.
    Clause(ClauseId),
    /// An Item of an Interface.
    Item(ItemId),
    /// A Case of a Harness.
    Case(CaseId),
}

impl Target {
    /// Its id; none for the Project, of which there is one.
    pub fn as_uuid(self) -> Option<uuid::Uuid> {
        match self {
            Target::Project => None,
            Target::Context(i) => Some(i.as_uuid()),
            Target::Contract(i) => Some(i.as_uuid()),
            Target::Interface(i) => Some(i.as_uuid()),
            Target::Harness(i) => Some(i.as_uuid()),
            Target::Clause(i) => Some(i.as_uuid()),
            Target::Item(i) => Some(i.as_uuid()),
            Target::Case(i) => Some(i.as_uuid()),
        }
    }

    /// The word for its kind.
    pub fn word(self) -> &'static str {
        match self {
            Target::Project => "project",
            Target::Context(_) => "context",
            Target::Contract(_) => "contract",
            Target::Interface(_) => "interface",
            Target::Harness(_) => "harness",
            Target::Clause(_) => "clause",
            Target::Item(_) => "item",
            Target::Case(_) => "case",
        }
    }
}

/// What a change request asks about a target. Both are the content of the request: neither is a condition of
/// resolving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExpectKind {
    /// The target is what the request asks to be revised or retired: its work is meant to move it (or take it out).
    /// Whether it did is read from the footprints of the work, not from the target's revision. Adding something is
    /// a move of what holds it.
    Target,
    /// The target is to stay as it is: its revision is still the one recorded.
    Keep,
}

impl ExpectKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            ExpectKind::Target => "target",
            ExpectKind::Keep => "keep",
        }
    }
}

impl fmt::Display for ExpectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// What a change request asks about one target, and the revision the target had when it was recorded (when the
/// change request was opened or, for an option of its Crux, when the option was chosen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expectation {
    /// What is asked.
    pub kind: ExpectKind,
    /// The revision the target had when this was recorded. A keep is judged against it; for a target it is the
    /// record of where the work started from.
    pub recorded: Revision,
}

impl Expectation {
    /// Whether this is a keep that `now`, the revision the target has now (`None`: it is gone), breaks. A target
    /// to revise or retire is never broken by a revision.
    pub fn keep_broken_by(&self, now: Option<Revision>) -> bool {
        self.kind == ExpectKind::Keep && now != Some(self.recorded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_keep_is_broken_by_the_revision() {
        let r1 = Revision::first();
        let r2 = r1.next();
        let at = |kind| Expectation { kind, recorded: r1 };
        assert!(!at(ExpectKind::Keep).keep_broken_by(Some(r1)));
        assert!(at(ExpectKind::Keep).keep_broken_by(Some(r2)));
        assert!(at(ExpectKind::Keep).keep_broken_by(None));
        for now in [Some(r1), Some(r2), None] {
            assert!(!at(ExpectKind::Target).keep_broken_by(now));
        }
    }
}
