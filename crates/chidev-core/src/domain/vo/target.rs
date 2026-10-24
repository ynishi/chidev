//! What a citation or a change request points at, and what a change request expects of it.

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

/// What a change request expects of a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExpectKind {
    /// The target is to change: its revision goes past the one recorded (and it is still there). Adding something
    /// is a change of what holds it.
    Change,
    /// The target is to go: it is no longer there.
    Remove,
    /// The target is to stay as it is: its revision is still the one recorded.
    Keep,
}

impl ExpectKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            ExpectKind::Change => "change",
            ExpectKind::Remove => "remove",
            ExpectKind::Keep => "keep",
        }
    }
}

impl fmt::Display for ExpectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// What a change request expects of one target, and the revision the target had when the expectation was recorded
/// (when the change request was opened or, for an option of its Crux, when the option was chosen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expectation {
    /// What is expected.
    pub kind: ExpectKind,
    /// The revision the target had when this was recorded.
    pub recorded: Revision,
}

impl Expectation {
    /// Whether `now`, the revision the target has now (`None`: it is gone), meets the expectation.
    pub fn is_met_by(&self, now: Option<Revision>) -> bool {
        match (self.kind, now) {
            (ExpectKind::Change, Some(now)) => now > self.recorded,
            (ExpectKind::Change, None) => false,
            (ExpectKind::Remove, now) => now.is_none(),
            (ExpectKind::Keep, now) => now == Some(self.recorded),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_is_met_by_its_own_state_of_the_target() {
        let r1 = Revision::first();
        let r2 = r1.next();
        let at = |kind| Expectation { kind, recorded: r1 };
        assert!(!at(ExpectKind::Change).is_met_by(Some(r1)));
        assert!(at(ExpectKind::Change).is_met_by(Some(r2)));
        assert!(!at(ExpectKind::Change).is_met_by(None));
        assert!(!at(ExpectKind::Remove).is_met_by(Some(r2)));
        assert!(at(ExpectKind::Remove).is_met_by(None));
        assert!(at(ExpectKind::Keep).is_met_by(Some(r1)));
        assert!(!at(ExpectKind::Keep).is_met_by(Some(r2)));
        assert!(!at(ExpectKind::Keep).is_met_by(None));
    }
}
