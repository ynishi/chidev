//! The footprint: what one change set moved, as it is attributed to the change request it was applied for.

use super::target::Target;

/// A target a change set moved: it was made, its revision went up, or it went. It keeps the containers it is in
/// (the Project first, then the Context, then the asset) as they were when it moved, so that whether it is inside a
/// frame can be read later, when the target is gone and the Project cannot say where it was.
///
/// A moved Case also keeps what it cites (see [`Move::with_cites`]), as the model had it when the change set was
/// applied, so that whether the Case is inside a frame through a citation can be read later too. A move of anything
/// else cites nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    target: Target,
    within: Vec<Target>,
    cites: Vec<Target>,
    cites_within: Vec<Target>,
}

impl Move {
    /// A move of `target`, which is inside `within`, the outermost first (the Project is the first of them). It
    /// cites nothing.
    pub fn new(target: Target, within: Vec<Target>) -> Move {
        Move {
            target,
            within,
            cites: Vec::new(),
            cites_within: Vec::new(),
        }
    }

    /// This move of a Case, which cites `cites` (Items and Clauses), the containers of which are `cites_within`
    /// (the Contracts, the Interfaces, the Contexts and the Project they are in). The change set may have changed
    /// what the Case cites: they are the citations before it and after it together, so that a Case is not taken
    /// out of a frame by the change that moved it.
    pub fn with_cites(mut self, cites: Vec<Target>, cites_within: Vec<Target>) -> Move {
        self.cites = cites;
        self.cites_within = cites_within;
        self
    }

    /// The target that moved.
    pub fn target(&self) -> Target {
        self.target
    }

    /// The containers it is in, the outermost first.
    pub fn within(&self) -> &[Target] {
        &self.within
    }

    /// The Items and Clauses a moved Case cites, as recorded; none for a move of anything else, and none for a
    /// footprint kept before citations were recorded.
    pub fn cites(&self) -> &[Target] {
        &self.cites
    }

    /// The containers of what it cites, the Project first, as recorded.
    pub fn cites_within(&self) -> &[Target] {
        &self.cites_within
    }

    /// Whether the target is `container` or inside it.
    pub fn is_inside(&self, container: Target) -> bool {
        self.target == container || self.within.contains(&container)
    }

    /// Whether any one of the citations is `container` or inside it.
    pub fn cites_inside(&self, container: Target) -> bool {
        self.cites.contains(&container) || self.cites_within.contains(&container)
    }
}

/// What a change set moved, recorded for the change request it was applied for, at the revision of the store it
/// was kept at. It is a fact about the past: it never changes, and a container whose revision moved because a
/// thing in it was added or removed is a move of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footprint {
    store_revision: u64,
    moves: Vec<Move>,
}

impl Footprint {
    /// The footprint of the change set kept at `store_revision`.
    pub fn new(store_revision: u64, moves: Vec<Move>) -> Footprint {
        Footprint {
            store_revision,
            moves,
        }
    }

    /// The revision of the store the change set was kept at.
    pub fn store_revision(&self) -> u64 {
        self.store_revision
    }

    /// What moved, in the order of the targets.
    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// The targets that moved.
    pub fn targets(&self) -> impl Iterator<Item = Target> + '_ {
        self.moves.iter().map(Move::target)
    }
}
