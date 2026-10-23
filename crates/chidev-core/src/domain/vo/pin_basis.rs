//! What the code of an Interface stood on when it was pinned.

use std::collections::BTreeMap;

use super::id::{ContractId, HarnessId, InterfaceId};
use super::revision::Revision;

/// The revisions the code of one Interface was accepted against: the Interface itself, every Harness with a Case
/// that cites one of its Items, every Contract with a Clause those Cases cite, and every other Interface with an
/// Item those Cases cite (the code calls what its tests call). Only what the citations reach is in it, so a change
/// elsewhere in the Project stales nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinBasis {
    interface: Revision,
    harnesses: BTreeMap<HarnessId, Revision>,
    contracts: BTreeMap<ContractId, Revision>,
    used_interfaces: BTreeMap<InterfaceId, Revision>,
}

impl PinBasis {
    /// The revisions, as gathered from the citations.
    pub fn new(
        interface: Revision,
        harnesses: BTreeMap<HarnessId, Revision>,
        contracts: BTreeMap<ContractId, Revision>,
        used_interfaces: BTreeMap<InterfaceId, Revision>,
    ) -> PinBasis {
        PinBasis {
            interface,
            harnesses,
            contracts,
            used_interfaces,
        }
    }

    /// The revision of the pinned Interface.
    pub fn interface(&self) -> Revision {
        self.interface
    }

    /// The Harnesses that check it, with their revisions.
    pub fn harnesses(&self) -> &BTreeMap<HarnessId, Revision> {
        &self.harnesses
    }

    /// The Contracts its checks cite, with their revisions.
    pub fn contracts(&self) -> &BTreeMap<ContractId, Revision> {
        &self.contracts
    }

    /// The other Interfaces its checks cite, with their revisions.
    pub fn used_interfaces(&self) -> &BTreeMap<InterfaceId, Revision> {
        &self.used_interfaces
    }
}
