//! The footprint: what one change set moved, as it is attributed to the change request it was applied for.

use std::collections::BTreeMap;

use super::id::CaseId;
use super::target::Target;

/// A target a change set moved: it was made, its revision went up, or it went. It keeps the containers it is in
/// (the Project first, then the Context, then the asset) as they were when it moved, so that whether it is inside a
/// frame can be read later, when the target is gone and the Project cannot say where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    target: Target,
    within: Vec<Target>,
}

impl Move {
    /// A move of `target`, which is inside `within`, the outermost first (the Project is the first of them).
    pub fn new(target: Target, within: Vec<Target>) -> Move {
        Move { target, within }
    }

    /// The target that moved.
    pub fn target(&self) -> Target {
        self.target
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
/// was kept at. It is a fact about the past: it never changes, and a container whose revision moved because a
/// thing in it was added or removed is a move of its own.
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

    /// Whether `target` is a Case with a citation that is `container` or inside it.
    pub fn cites_inside(&self, target: Target, container: Target) -> bool {
        match target {
            Target::Case(case) => self
                .citations
                .get(&case)
                .is_some_and(|c| c.is_inside(container)),
            _ => false,
        }
    }
}
