//! A store held in memory: every repository and the asset names, gone when the process ends.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::app::ports::{AssetKind, AssetNames};
use crate::domain::entity::crux::Crux;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{CruxRepository, PinRepository, ProjectRepository, StoreError};
use crate::domain::vo::id::{ContextId, CruxId, InterfaceId};

/// Keeps the Project, the Pins, the Cruxes and the asset names in memory. Nothing it does fails.
#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    project: Option<Project>,
    pins: BTreeMap<InterfaceId, Pin>,
    cruxes: BTreeMap<CruxId, Crux>,
    names: BTreeMap<(ContextId, AssetKind, String), Uuid>,
}

impl ProjectRepository for MemoryStore {
    fn project(&self) -> Result<Option<Project>, StoreError> {
        Ok(self.project.clone())
    }

    fn save_project(&mut self, project: &Project) -> Result<(), StoreError> {
        self.project = Some(project.clone());
        Ok(())
    }
}

impl PinRepository for MemoryStore {
    fn pin_of(&self, interface: InterfaceId) -> Result<Option<Pin>, StoreError> {
        Ok(self.pins.get(&interface).cloned())
    }

    fn pins(&self) -> Result<Vec<Pin>, StoreError> {
        Ok(self.pins.values().cloned().collect())
    }

    fn save_pin(&mut self, pin: &Pin) -> Result<(), StoreError> {
        self.pins.insert(pin.interface(), pin.clone());
        Ok(())
    }
}

impl CruxRepository for MemoryStore {
    fn crux(&self, id: CruxId) -> Result<Option<Crux>, StoreError> {
        Ok(self.cruxes.get(&id).cloned())
    }

    fn cruxes(&self) -> Result<Vec<Crux>, StoreError> {
        Ok(self.cruxes.values().cloned().collect())
    }

    fn save_crux(&mut self, crux: &Crux) -> Result<(), StoreError> {
        self.cruxes.insert(crux.id(), crux.clone());
        Ok(())
    }
}

impl AssetNames for MemoryStore {
    fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        Ok(self.names.get(&(context, kind, name.to_string())).copied())
    }

    fn asset_name(&self, id: Uuid) -> Result<Option<String>, StoreError> {
        Ok(self
            .names
            .iter()
            .find(|(_, v)| **v == id)
            .map(|((_, _, n), _)| n.clone()))
    }

    fn name_asset(
        &mut self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
        id: Uuid,
    ) -> Result<(), StoreError> {
        self.names.insert((context, kind, name.to_string()), id);
        Ok(())
    }
}
