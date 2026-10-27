//! The repositories: how each aggregate (the Project, a Pin, a change request, a Crux) is found again. They are declared
//! here, beside the aggregates they keep, and implemented outside the domain; the domain never calls one. Writing
//! is not here: a use case gathers what it changed and commits it at once (`app::ports::Store`), so that what one
//! use case writes is kept whole or not at all.

use std::future::Future;

use thiserror::Error;

use super::entity::change_request::ChangeRequest;
use super::entity::crux::Crux;
use super::entity::pin::Pin;
use super::entity::project::Project;
use super::vo::id::{ChangeRequestId, CruxId, InterfaceId};

/// Why a store could not give back or keep what was asked. The domain does not interpret the store's words.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StoreError {
    /// Someone else committed since this use case read: its reads may be out of date, so nothing was written.
    #[error("the store changed since it was read; nothing was written")]
    Conflict,
    /// What the store keeps cannot be read as the model (a broken or newer record).
    #[error("the store holds what cannot be read: {0}")]
    Corrupt(String),
    /// The store failed (unreadable, a write refused).
    #[error("store: {0}")]
    Failed(String),
}

/// Finds the one Project of a repository.
pub trait ProjectRepository {
    /// The Project, or `None` before one is kept.
    fn project(&self) -> impl Future<Output = Result<Option<Project>, StoreError>> + Send;
}

/// Finds the Pins, one per Interface.
pub trait PinRepository {
    /// The Pin of `interface`, if its code was pinned.
    fn pin_of(
        &self,
        interface: InterfaceId,
    ) -> impl Future<Output = Result<Option<Pin>, StoreError>> + Send;

    /// Every Pin, in the order of their Interfaces' ids.
    fn pins(&self) -> impl Future<Output = Result<Vec<Pin>, StoreError>> + Send;
}

/// Finds the change requests.
pub trait ChangeRequestRepository {
    /// The change request with this id.
    fn change_request(
        &self,
        id: ChangeRequestId,
    ) -> impl Future<Output = Result<Option<ChangeRequest>, StoreError>> + Send;

    /// Every change request, in the order of their ids.
    fn change_requests(
        &self,
    ) -> impl Future<Output = Result<Vec<ChangeRequest>, StoreError>> + Send;
}

/// Finds the Cruxes. Which Cruxes stop a change request is a query over `raised_in`: the change request does not
/// hold them.
pub trait CruxRepository {
    /// The Crux with this id.
    fn crux(&self, id: CruxId) -> impl Future<Output = Result<Option<Crux>, StoreError>> + Send;

    /// Every Crux, in the order of their ids.
    fn cruxes(&self) -> impl Future<Output = Result<Vec<Crux>, StoreError>> + Send;

    /// The Cruxes raised in the change request `raised_in`, in the order of their ids.
    fn cruxes_raised_in(
        &self,
        raised_in: ChangeRequestId,
    ) -> impl Future<Output = Result<Vec<Crux>, StoreError>> + Send;
}
