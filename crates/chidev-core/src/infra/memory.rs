//! A store held in memory: every repository and the asset names, gone when the process ends.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::app::ports::{AssetKind, AssetNames};
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{
    ChangeRequestRepository, PinRepository, ProjectRepository, StoreError,
};
use crate::domain::vo::id::{ChangeRequestId, ContextId, InterfaceId};

/// Keeps the Project, the Pins, the change requests and the asset names in memory. Nothing it does fails.
#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    project: Option<Project>,
    pins: BTreeMap<InterfaceId, Pin>,
    crs: BTreeMap<ChangeRequestId, ChangeRequest>,
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

impl ChangeRequestRepository for MemoryStore {
    fn change_request(&self, id: ChangeRequestId) -> Result<Option<ChangeRequest>, StoreError> {
        Ok(self.crs.get(&id).cloned())
    }

    fn change_requests(&self) -> Result<Vec<ChangeRequest>, StoreError> {
        Ok(self.crs.values().cloned().collect())
    }

    fn save_change_request(&mut self, cr: &ChangeRequest) -> Result<(), StoreError> {
        self.crs.insert(cr.id(), cr.clone());
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
