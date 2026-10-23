//! The repositories: how each aggregate (the Project, a Pin, a Crux) is kept and found again. They are declared
//! here, beside the aggregates they keep, and implemented outside the domain; the domain never calls one.

use thiserror::Error;

use super::entity::crux::Crux;
use super::entity::pin::Pin;
use super::entity::project::Project;
use super::vo::id::{CruxId, InterfaceId};

/// Why a repository could not keep or give back an aggregate (the store is unreadable, a write failed). It holds
/// the store's own words: the domain does not interpret them.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("store: {0}")]
pub struct StoreError(pub String);

/// Keeps the one Project of a repository.
pub trait ProjectRepository {
    /// The Project, or `None` before one is kept.
    fn project(&self) -> Result<Option<Project>, StoreError>;

    /// Keeps `project`, replacing the one kept before.
    fn save_project(&mut self, project: &Project) -> Result<(), StoreError>;
}

/// Keeps the Pins, one per Interface.
pub trait PinRepository {
    /// The Pin of `interface`, if its code was pinned.
    fn pin_of(&self, interface: InterfaceId) -> Result<Option<Pin>, StoreError>;

    /// Every Pin, in the order of their Interfaces' ids.
    fn pins(&self) -> Result<Vec<Pin>, StoreError>;

    /// Keeps `pin`, replacing the Pin of the same Interface.
    fn save_pin(&mut self, pin: &Pin) -> Result<(), StoreError>;
}

/// Keeps the Cruxes.
pub trait CruxRepository {
    /// The Crux with this id.
    fn crux(&self, id: CruxId) -> Result<Option<Crux>, StoreError>;

    /// Every Crux, in the order of their ids.
    fn cruxes(&self) -> Result<Vec<Crux>, StoreError>;

    /// Keeps `crux`, replacing the one with the same id.
    fn save_crux(&mut self, crux: &Crux) -> Result<(), StoreError>;
}
