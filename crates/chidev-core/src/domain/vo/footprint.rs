//! The footprint: what one change set moved, as it is attributed to the change request it was applied for.

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
