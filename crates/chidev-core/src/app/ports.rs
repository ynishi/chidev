//! What the use cases need from outside the domain besides the repositories: fresh ids, and the names a person
//! gives the assets.

use uuid::Uuid;

use crate::domain::repository::StoreError;
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

/// The names a person addresses the assets by. A name is not part of the model (an asset is its id); it is how
/// the commands find the id, so it is kept beside the model, one name per asset within its Context and kind.
pub trait AssetNames {
    /// The id of the asset of `kind` named `name` in `context`.
    fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
    ) -> Result<Option<Uuid>, StoreError>;

    /// The name of the asset with this id.
    fn asset_name(&self, id: Uuid) -> Result<Option<String>, StoreError>;

    /// Names the asset `id`.
    fn name_asset(
        &mut self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
        id: Uuid,
    ) -> Result<(), StoreError>;
}
