//! What the code of an Interface stood on when it was pinned, and what of it has moved since.

use std::collections::BTreeMap;

use super::id::{ContractId, HarnessId, InterfaceId};
use super::revision::Revision;

/// Which part of what a Pin stands on moved: what moved past the revision it recorded, or is gone, or is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleReason {
    /// The pinned Interface changed.
    Interface,
    /// This Harness changed, is gone, or newly checks the Interface.
    Harness(HarnessId),
    /// This Contract changed, is gone, or is newly cited.
    Contract(ContractId),
    /// This other Interface changed, is gone, or is newly cited.
    UsedInterface(InterfaceId),
}

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

    /// What of this basis has moved in `now`, in the order of [`StaleReason`] and then of the ids: the Interface,
    /// then each Harness, Contract and other Interface whose revision differs or that is in only one of the two.
    /// Empty when nothing it stood on has moved.
    pub fn moved(&self, now: &PinBasis) -> Vec<StaleReason> {
        let mut why = Vec::new();
        if self.interface != now.interface {
            why.push(StaleReason::Interface);
        }
        why.extend(moved_keys(&self.harnesses, &now.harnesses).map(StaleReason::Harness));
        why.extend(moved_keys(&self.contracts, &now.contracts).map(StaleReason::Contract));
        why.extend(
            moved_keys(&self.used_interfaces, &now.used_interfaces).map(StaleReason::UsedInterface),
        );
        why
    }
}

/// The keys whose value differs between `was` and `now`, or that are in only one of them, in order. The one place
/// that tells what moved between two maps of a basis, for a Pin's and for a simulation's.
pub(super) fn moved_keys<'a, K: Ord + Copy, V: PartialEq>(
    was: &'a BTreeMap<K, V>,
    now: &'a BTreeMap<K, V>,
) -> impl Iterator<Item = K> + 'a {
    let mut keys: Vec<K> = was.keys().chain(now.keys()).copied().collect();
    keys.sort();
    keys.dedup();
    keys.into_iter().filter(move |k| was.get(k) != now.get(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;

    fn harness(n: u128) -> HarnessId {
        HarnessId::from_uuid(test_uuid(n))
    }

    fn contract(n: u128) -> ContractId {
        ContractId::from_uuid(test_uuid(n))
    }

    fn interface(n: u128) -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(n))
    }

    /// A basis of the Interface at its first revision, with Harness 4, Contract 3 and Interface 2 at their first.
    fn basis() -> PinBasis {
        PinBasis::new(
            Revision::first(),
            BTreeMap::from([(harness(4), Revision::first())]),
            BTreeMap::from([(contract(3), Revision::first())]),
            BTreeMap::from([(interface(2), Revision::first())]),
        )
    }

    #[test]
    fn nothing_moved_is_empty() {
        assert_eq!(basis().moved(&basis()), []);
    }

    #[test]
    fn each_part_alone_moves_only_that_part() {
        let next = Revision::first().next();
        let was = basis();
        let moved = |now: PinBasis| was.moved(&now);
        assert_eq!(
            moved(PinBasis::new(
                next,
                was.harnesses().clone(),
                was.contracts().clone(),
                was.used_interfaces().clone()
            )),
            [StaleReason::Interface]
        );
        assert_eq!(
            moved(PinBasis::new(
                was.interface(),
                BTreeMap::from([(harness(4), next)]),
                was.contracts().clone(),
                was.used_interfaces().clone()
            )),
            [StaleReason::Harness(harness(4))]
        );
        assert_eq!(
            moved(PinBasis::new(
                was.interface(),
                was.harnesses().clone(),
                BTreeMap::from([(contract(3), next)]),
                was.used_interfaces().clone()
            )),
            [StaleReason::Contract(contract(3))]
        );
        assert_eq!(
            moved(PinBasis::new(
                was.interface(),
                was.harnesses().clone(),
                was.contracts().clone(),
                BTreeMap::from([(interface(2), next)])
            )),
            [StaleReason::UsedInterface(interface(2))]
        );
    }

    #[test]
    fn a_key_in_only_one_basis_has_moved() {
        let was = basis();
        // a Harness that is new, a Contract that is gone, an Interface that is new: all three moved, in order
        let now = PinBasis::new(
            was.interface(),
            BTreeMap::from([
                (harness(4), Revision::first()),
                (harness(5), Revision::first()),
            ]),
            BTreeMap::new(),
            BTreeMap::from([
                (interface(1), Revision::first()),
                (interface(2), Revision::first()),
            ]),
        );
        assert_eq!(
            was.moved(&now),
            [
                StaleReason::Harness(harness(5)),
                StaleReason::Contract(contract(3)),
                StaleReason::UsedInterface(interface(1)),
            ]
        );
        // and the other way round
        assert_eq!(
            now.moved(&was),
            [
                StaleReason::Harness(harness(5)),
                StaleReason::Contract(contract(3)),
                StaleReason::UsedInterface(interface(1)),
            ]
        );
    }
}
