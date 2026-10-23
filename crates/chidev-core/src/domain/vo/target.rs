//! What a citation points at.

use super::id::{ClauseId, ItemId};

/// A definition a Case or a change request can point at: an Item of an Interface or a Clause of a Contract. It holds the id
/// alone; ids are given once in the whole Project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// An Item of an Interface.
    Item(ItemId),
    /// A Clause of a Contract.
    Clause(ClauseId),
}
