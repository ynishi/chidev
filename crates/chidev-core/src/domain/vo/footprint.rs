//! The footprint: what one change set moved, as it is attributed to the change request it was applied for.

use std::collections::BTreeMap;

use super::clause::ClauseLabel;
use super::id::{CaseId, ClauseId, ItemId};
use super::item_key::ItemKey;
use super::target::Target;

/// A new name a Clause or an Item was given: the name it had and the name it has. A name is not part of what a
/// target promises, so a new name moves no revision (see ADR 0004), and it moves no container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rename {
    /// A Clause given another label.
    Label {
        /// The label it had.
        from: ClauseLabel,
        /// The label it has.
        to: ClauseLabel,
    },
    /// An Item given another key.
    Key {
        /// The key it had.
        from: ItemKey,
        /// The key it has.
        to: ItemKey,
    },
}

/// What a change set did to the target of a [`Move`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The target was made, its revision went up, or it is gone.
    Revised,
    /// The target was given a new name and no revision moved.
    Renamed(Rename),
    /// The target was given a new name and its revision went up, both in the one change set.
    RevisedAndRenamed(Rename),
}

/// A target a change set moved: it was made, its revision went up, or it went, or it is a Clause or an Item given a
/// new name. A new name moves no revision and no container. It keeps the containers it is in (the Project first,
/// then the Context, then the asset) as they were when it moved, so that whether it is inside a frame can be read
/// later, when the target is gone and the Project cannot say where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    target: Target,
    within: Vec<Target>,
    change: Change,
}

impl Move {
    /// A move of `target`, which is inside `within`, the outermost first (the Project is the first of them): it was
    /// made, its revision went up, or it is gone.
    pub fn new(target: Target, within: Vec<Target>) -> Move {
        Move {
            target,
            within,
            change: Change::Revised,
        }
    }

    /// The move of the Clause `clause`, inside `within`, from the label `from` to the label `to`. `revised` is
    /// whether its revision moved in the same change set.
    pub fn relabeled(
        clause: ClauseId,
        within: Vec<Target>,
        from: ClauseLabel,
        to: ClauseLabel,
        revised: bool,
    ) -> Move {
        Move::named(
            Target::Clause(clause),
            within,
            Rename::Label { from, to },
            revised,
        )
    }

    /// The move of the Item `item`, inside `within`, from the key `from` to the key `to`. `revised` is whether its
    /// revision moved in the same change set.
    pub fn rekeyed(
        item: ItemId,
        within: Vec<Target>,
        from: ItemKey,
        to: ItemKey,
        revised: bool,
    ) -> Move {
        Move::named(
            Target::Item(item),
            within,
            Rename::Key { from, to },
            revised,
        )
    }

    fn named(target: Target, within: Vec<Target>, rename: Rename, revised: bool) -> Move {
        let change = if revised {
            Change::RevisedAndRenamed(rename)
        } else {
            Change::Renamed(rename)
        };
        Move {
            target,
            within,
            change,
        }
    }

    /// The target that moved.
    pub fn target(&self) -> Target {
        self.target
    }

    /// What was done to it.
    pub fn change(&self) -> &Change {
        &self.change
    }

    /// Whether its revision moved: it was made, went up, or is gone.
    pub fn is_revised(&self) -> bool {
        matches!(self.change, Change::Revised | Change::RevisedAndRenamed(_))
    }

    /// The new name it was given, if it was given one.
    pub fn rename(&self) -> Option<&Rename> {
        match &self.change {
            Change::Revised => None,
            Change::Renamed(r) | Change::RevisedAndRenamed(r) => Some(r),
        }
    }

    /// The containers it is in, the outermost first.
    pub fn within(&self) -> &[Target] {
        &self.within
    }

    /// Whether the target is `container` or inside it.
    pub fn is_inside(&self, container: Target) -> bool {
        self.target == container || self.within.contains(&container)
    }
}

/// What a moved Case cites, as the model had it when the change set was applied: the Items and Clauses, and the
/// containers of those. The change set may have changed what the Case cites, so these are the citations before it
/// and after it together, so that a Case is not taken out of a frame by the change that moved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citations {
    cites: Vec<Target>,
    within: Vec<Target>,
}

impl Citations {
    /// The citations `cites` (Items and Clauses), the containers of which are `within` (the Contracts, the
    /// Interfaces, the Contexts and the Project they are in).
    pub fn new(cites: Vec<Target>, within: Vec<Target>) -> Citations {
        Citations { cites, within }
    }

    /// The Items and Clauses cited.
    pub fn cites(&self) -> &[Target] {
        &self.cites
    }

    /// The containers of what is cited, the Project first.
    pub fn within(&self) -> &[Target] {
        &self.within
    }

    /// Whether any one of the citations is `container` or inside it.
    pub fn is_inside(&self, container: Target) -> bool {
        self.cites.contains(&container) || self.within.contains(&container)
    }
}

/// What a change set moved, recorded for the change request it was applied for, at the revision of the store it
/// was kept at: the targets made, revised or gone, and the Clauses and Items given a new name. It is a fact about
/// the past: it never changes, and a container whose revision moved because a thing in it was added or removed is a
/// move of its own; a new name moves no container.
///
/// A Case that moved also keeps what it cites (see [`Citations`]), so that whether the Case is inside a frame
/// through a citation can be read later too. They are kept by the Case, so that they cannot be kept for anything
/// else; a footprint recorded without them is judged by what moved alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footprint {
    store_revision: u64,
    moves: Vec<Move>,
    citations: BTreeMap<CaseId, Citations>,
}

impl Footprint {
    /// The footprint of the change set kept at `store_revision`. It records no citations.
    pub fn new(store_revision: u64, moves: Vec<Move>) -> Footprint {
        Footprint {
            store_revision,
            moves,
            citations: BTreeMap::new(),
        }
    }

    /// This footprint with what the Cases it moved cite, each by its Case.
    pub fn with_citations(mut self, citations: BTreeMap<CaseId, Citations>) -> Footprint {
        self.citations = citations;
        self
    }

    /// The revision of the store the change set was kept at.
    pub fn store_revision(&self) -> u64 {
        self.store_revision
    }

    /// What moved, in the order of the targets.
    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// What the moved Cases cited, as recorded; none for a footprint kept before citations were recorded.
    pub fn citations(&self) -> &BTreeMap<CaseId, Citations> {
        &self.citations
    }

    /// The targets that moved.
    pub fn targets(&self) -> impl Iterator<Item = Target> + '_ {
        self.moves.iter().map(Move::target)
    }

    /// The targets given a new name, each with the name it had and the name it has, in the order of the targets.
    pub fn renames(&self) -> impl Iterator<Item = (Target, &Rename)> + '_ {
        self.moves
            .iter()
            .filter_map(|m| m.rename().map(|r| (m.target(), r)))
    }

    /// What `target` cited when it moved, as recorded: only a Case that moved has it.
    pub fn citations_of(&self, target: Target) -> Option<&Citations> {
        match target {
            Target::Case(case) => self.citations.get(&case),
            _ => None,
        }
    }

    /// Whether `target` is a Case with a citation that is `container` or inside it.
    pub fn cites_inside(&self, target: Target, container: Target) -> bool {
        self.citations_of(target)
            .is_some_and(|c| c.is_inside(container))
    }
}
