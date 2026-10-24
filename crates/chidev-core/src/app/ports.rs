//! What the use cases need from outside the domain: fresh ids, the names a person gives the assets, and a store
//! that keeps what one use case changed whole or not at all.

use std::future::Future;

use uuid::Uuid;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{
    ChangeRequestRepository, PinRepository, ProjectRepository, StoreError,
};
use crate::domain::vo::id::ContextId;

/// Gives a fresh id each time. The domain makes no ids; the use cases take them from here.
pub trait IdSource {
    /// An id not given before.
    fn next_uuid(&mut self) -> Uuid;
}

/// The kind of an asset, for its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    /// A Contract.
    Contract,
    /// An Interface.
    Interface,
    /// A Harness.
    Harness,
}

impl AssetKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            AssetKind::Contract => "contract",
            AssetKind::Interface => "interface",
            AssetKind::Harness => "harness",
        }
    }
}

/// The name a person gives an asset: one per asset within its Context and kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetName {
    /// The Context the asset is in.
    pub context: ContextId,
    /// What kind of asset it is.
    pub kind: AssetKind,
    /// The name.
    pub name: String,
    /// The asset's id.
    pub id: Uuid,
}

/// The names a person addresses the assets by. A name is not part of the model (an asset is its id); it is how
/// the commands find the id, so it is kept beside the model.
pub trait AssetNames {
    /// The id of the asset of `kind` named `name` in `context`.
    fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
    ) -> impl Future<Output = Result<Option<Uuid>, StoreError>> + Send;

    /// The name of the asset with this id.
    fn asset_name(
        &self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;
}

/// What one use case changed, to be kept at once.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    /// The Project, as it is after the use case.
    pub project: Option<Project>,
    /// Pins made (each replaces the Pin of its Interface).
    pub pins: Vec<Pin>,
    /// Change requests opened or changed.
    pub change_requests: Vec<ChangeRequest>,
    /// Asset names given.
    pub names: Vec<AssetName>,
}

/// Where the store stood when a use case began to read. A commit given an older one is refused
/// ([`StoreError::Conflict`]): someone else wrote in between, and the use case's reads may be out of date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreRevision(pub u64);

/// A store: the repositories and the names to read from, and one commit per use case to write. A use case reads
/// what it needs (each read on its own; no transaction is held across them), decides, and commits every change at
/// once; the store keeps them all or none.
pub trait Store: ProjectRepository + PinRepository + ChangeRequestRepository + AssetNames {
    /// Where the store stands now; read it before the use case reads anything else.
    fn revision(&self) -> impl Future<Output = Result<StoreRevision, StoreError>> + Send;

    /// Keeps `changes` whole, if the store still stands at `read_at`; otherwise keeps nothing and answers
    /// [`StoreError::Conflict`].
    fn commit(
        &self,
        read_at: StoreRevision,
        changes: Changes,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
