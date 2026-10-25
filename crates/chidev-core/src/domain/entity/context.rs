//! The Context: a unit of the Project and the assets it owns (its Contracts, Interfaces and Harnesses).

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::contract::{Contract, ContractError};
use super::harness::{CaseFound, Harness, HarnessError, SimShapeProblem};
use super::interface::{Interface, InterfaceError, ItemFound};
use crate::domain::vo::clause::ClauseContent;
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{
    CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
};
use crate::domain::vo::name::ContextName;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::sim::role_of;
use crate::domain::vo::source::Source;
use crate::domain::vo::title::Title;

/// What a Context is checked against. It only goes up: a Context once based on its Contracts cannot quietly stop
/// owing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Basis {
    /// The Interfaces and the Harnesses are enough; a Contract is optional.
    Interface,
    /// The Contracts are the basis; the Context has at least one.
    Contract,
}

impl Basis {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Basis::Interface => "interface",
            Basis::Contract => "contract",
        }
    }
}

/// What a store keeps of a Context besides its assets.
#[derive(Debug, Clone)]
pub(crate) struct ContextHead {
    /// Its id.
    pub(crate) id: ContextId,
    /// Its revision.
    pub(crate) revision: Revision,
    /// Its name.
    pub(crate) name: ContextName,
    /// What it is checked against.
    pub(crate) basis: Basis,
    /// The Contexts it uses.
    pub(crate) uses: BTreeSet<ContextId>,
}

/// Which rule of a Context a change would break. The change is not made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContextError {
    /// A Context uses itself.
    #[error("a context cannot use itself")]
    UsesItself,
    /// The Context is already based on its Contracts.
    #[error("the context is already based on its contracts")]
    AlreadyOnContract,
    /// The Context is to be based on Contracts and has none.
    #[error("the context has no contract to be based on")]
    NoContract,
    /// The Context is based on its Contracts and the one to retire is its last.
    #[error("the context is based on its contracts, and this is its last one")]
    LastContract,
    /// The Context has no asset of this kind with this id.
    #[error("the context has no {kind} {id}")]
    NoSuchAsset {
        /// The kind of asset.
        kind: &'static str,
        /// The id looked for.
        id: uuid::Uuid,
    },
    /// The Contract content breaks a rule of the Contract.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// The Interface content breaks a rule of the Interface.
    #[error(transparent)]
    Interface(#[from] InterfaceError),
    /// The Harness content breaks a rule of the Harness.
    #[error(transparent)]
    Harness(#[from] HarnessError),
}

/// A unit of the Project and what it owns. A Context holds any number of each asset, each known by its id and
/// changed on its own; the assets are tied to each other only by what the Cases of a Harness cite. Its changes go
/// through the Project, which checks the rules between Contexts (what a Case may cite, what a Context may use)
/// before it keeps one.
///
/// Its revision is 1 when it is made and goes one up each time an asset is added to it or removed from it, and
/// each time its uses or its basis change. It does not move when the content of an asset changes: the asset has a
/// revision of its own. So a change request that adds a Contract expects the Context to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    id: ContextId,
    revision: Revision,
    name: ContextName,
    basis: Basis,
    uses: BTreeSet<ContextId>,
    contracts: BTreeMap<ContractId, Contract>,
    interfaces: BTreeMap<InterfaceId, Interface>,
    harnesses: BTreeMap<HarnessId, Harness>,
}

impl Context {
    /// A Context based on its Interfaces, with no assets yet.
    pub fn new(
        id: ContextId,
        name: ContextName,
        uses: BTreeSet<ContextId>,
    ) -> Result<Context, ContextError> {
        if uses.contains(&id) {
            return Err(ContextError::UsesItself);
        }
        Ok(Context {
            id,
            revision: Revision::first(),
            name,
            basis: Basis::Interface,
            uses,
            contracts: BTreeMap::new(),
            interfaces: BTreeMap::new(),
            harnesses: BTreeMap::new(),
        })
    }

    /// A Context as a store kept it, with its revision, basis and assets. It does not use itself, and a Context
    /// based on its Contracts has one at least.
    pub(crate) fn restore(
        head: ContextHead,
        contracts: Vec<Contract>,
        interfaces: Vec<Interface>,
        harnesses: Vec<Harness>,
    ) -> Result<Context, ContextError> {
        let ContextHead {
            id,
            revision,
            name,
            basis,
            uses,
        } = head;
        if uses.contains(&id) {
            return Err(ContextError::UsesItself);
        }
        if basis == Basis::Contract && contracts.is_empty() {
            return Err(ContextError::NoContract);
        }
        Ok(Context {
            id,
            revision,
            name,
            basis,
            uses,
            contracts: contracts.into_iter().map(|c| (c.id(), c)).collect(),
            interfaces: interfaces.into_iter().map(|i| (i.id(), i)).collect(),
            harnesses: harnesses.into_iter().map(|h| (h.id(), h)).collect(),
        })
    }

    /// Its id.
    pub fn id(&self) -> ContextId {
        self.id
    }

    /// How many times its set of assets, its uses or its basis has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Its name.
    pub fn name(&self) -> &ContextName {
        &self.name
    }

    /// What it is checked against.
    pub fn basis(&self) -> Basis {
        self.basis
    }

    /// The Contexts it uses.
    pub fn uses(&self) -> &BTreeSet<ContextId> {
        &self.uses
    }

    /// Its Contracts, in the order of their ids.
    pub fn contracts(&self) -> impl Iterator<Item = &Contract> {
        self.contracts.values()
    }

    /// Its Interfaces, in the order of their ids.
    pub fn interfaces(&self) -> impl Iterator<Item = &Interface> {
        self.interfaces.values()
    }

    /// Its Harnesses, in the order of their ids.
    pub fn harnesses(&self) -> impl Iterator<Item = &Harness> {
        self.harnesses.values()
    }

    /// The Contract with this id.
    pub fn contract(&self, id: ContractId) -> Option<&Contract> {
        self.contracts.get(&id)
    }

    /// The Interface with this id.
    pub fn interface(&self, id: InterfaceId) -> Option<&Interface> {
        self.interfaces.get(&id)
    }

    /// The Harness with this id.
    pub fn harness(&self, id: HarnessId) -> Option<&Harness> {
        self.harnesses.get(&id)
    }

    /// The Contract that has the Clause with this id.
    pub fn contract_of(&self, clause: ClauseId) -> Option<&Contract> {
        self.contracts().find(|c| c.clause(clause).is_some())
    }

    /// The Interface that has the Item with this id.
    pub fn interface_of(&self, item: ItemId) -> Option<&Interface> {
        self.interfaces().find(|i| i.item(item).is_some())
    }

    /// The Harness that has the Case with this id.
    pub fn harness_of(&self, case: CaseId) -> Option<&Harness> {
        self.harnesses().find(|h| h.case(case).is_some())
    }

    /// The ways the sim Harness `harness` of this Context is not the shape the protocol runs: those the Harness
    /// decides alone ([`Harness::sim_problems`]), more than one sim Harness in the Context, and a Clause a Case
    /// cites that no Contract of this Context has (one in a Context it uses is reachable, but a script is run
    /// against the Contracts of its own Context). None for a Harness of tests.
    ///
    /// `harness` is one of this Context's. A problem about the Context as a whole comes with each of its sim
    /// Harnesses, so a caller that asks for several keeps each problem once.
    pub fn sim_problems(&self, harness: &Harness) -> Vec<SimShapeProblem> {
        if harness.kind() != HarnessKind::Sim {
            return Vec::new();
        }
        let sims: Vec<HarnessId> = self
            .harnesses()
            .filter(|h| h.kind() == HarnessKind::Sim)
            .map(Harness::id)
            .collect();
        let mut out = Vec::new();
        if sims.len() > 1 {
            out.push(SimShapeProblem::OnePerContext {
                context: self.id,
                harnesses: sims,
            });
        }
        out.extend(harness.sim_problems());
        let own: BTreeSet<ClauseId> = self
            .contracts()
            .flat_map(|k| k.clauses().iter().map(|c| c.id()))
            .collect();
        for case in harness
            .cases()
            .iter()
            .filter(|c| role_of(c.name().as_str()).is_some())
        {
            for clause in case.cites().clauses().keys().filter(|k| !own.contains(k)) {
                out.push(SimShapeProblem::ClauseForeign {
                    harness: harness.id(),
                    case: case.id(),
                    clause: *clause,
                });
            }
        }
        out
    }

    /// Sets the Contexts it uses. Its revision goes up when they differ.
    pub(super) fn set_uses(&mut self, uses: BTreeSet<ContextId>) -> Result<(), ContextError> {
        if uses.contains(&self.id) {
            return Err(ContextError::UsesItself);
        }
        if uses != self.uses {
            self.revision = self.revision.next();
        }
        self.uses = uses;
        Ok(())
    }

    /// Bases the Context on its Contracts, of which it must have one. There is no way back.
    pub(super) fn base_on_contract(&mut self) -> Result<(), ContextError> {
        if self.basis == Basis::Contract {
            return Err(ContextError::AlreadyOnContract);
        }
        if self.contracts.is_empty() {
            return Err(ContextError::NoContract);
        }
        self.basis = Basis::Contract;
        self.revision = self.revision.next();
        Ok(())
    }

    /// Takes the Contract with this id out of the Context and gives it back. The Context goes to its next revision.
    /// A Context based on its Contracts keeps one at least.
    pub(super) fn retire_contract(&mut self, id: ContractId) -> Result<Contract, ContextError> {
        if self.basis == Basis::Contract
            && self.contracts.len() == 1
            && self.contracts.contains_key(&id)
        {
            return Err(ContextError::LastContract);
        }
        let contract = self
            .contracts
            .remove(&id)
            .ok_or(ContextError::NoSuchAsset {
                kind: "contract",
                id: id.as_uuid(),
            })?;
        self.revision = self.revision.next();
        Ok(contract)
    }

    /// Takes the Interface with this id out of the Context and gives it back. The Context goes to its next revision.
    pub(super) fn retire_interface(&mut self, id: InterfaceId) -> Result<Interface, ContextError> {
        let interface = self
            .interfaces
            .remove(&id)
            .ok_or(ContextError::NoSuchAsset {
                kind: "interface",
                id: id.as_uuid(),
            })?;
        self.revision = self.revision.next();
        Ok(interface)
    }

    /// Takes the Harness with this id out of the Context and gives it back. The Context goes to its next revision.
    pub(super) fn retire_harness(&mut self, id: HarnessId) -> Result<Harness, ContextError> {
        let harness = self
            .harnesses
            .remove(&id)
            .ok_or(ContextError::NoSuchAsset {
                kind: "harness",
                id: id.as_uuid(),
            })?;
        self.revision = self.revision.next();
        Ok(harness)
    }

    /// Makes the Contract with this id, or revises it when the Context has it.
    pub(super) fn put_contract(
        &mut self,
        id: ContractId,
        title: Title,
        clauses: Vec<(ClauseId, ClauseContent)>,
    ) -> Result<(), ContextError> {
        match self.contracts.get_mut(&id) {
            Some(c) => c.revise(title, clauses)?,
            None => {
                self.contracts
                    .insert(id, Contract::new(id, title, clauses)?);
                self.revision = self.revision.next();
            }
        }
        Ok(())
    }

    /// Makes the Interface with this id, or revises it when the Context has it.
    pub(super) fn put_interface(
        &mut self,
        id: InterfaceId,
        source: Source,
        items: Vec<ItemFound>,
    ) -> Result<(), ContextError> {
        match self.interfaces.get_mut(&id) {
            Some(i) => i.revise(source, items)?,
            None => {
                self.interfaces
                    .insert(id, Interface::new(id, source, items)?);
                self.revision = self.revision.next();
            }
        }
        Ok(())
    }

    /// Makes the Harness with this id, or revises it when the Context has it. `kind` is the kind it is to have;
    /// without one a new Harness is of tests and an existing one keeps its kind. `builds` is the whole set of
    /// Interfaces it builds (a put states the Harness as a whole, as it states its code and its Cases).
    pub(super) fn put_harness(
        &mut self,
        id: HarnessId,
        kind: Option<HarnessKind>,
        builds: BTreeSet<InterfaceId>,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), ContextError> {
        match self.harnesses.get_mut(&id) {
            Some(h) => {
                let kind = kind.unwrap_or(h.kind());
                h.revise_as(kind, builds, source, cases)?
            }
            None => {
                let kind = kind.unwrap_or_default();
                self.harnesses
                    .insert(id, Harness::of_kind(kind, id, builds, source, cases)?);
                self.revision = self.revision.next();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::clause::{ClauseLabel, ClauseText};
    use crate::domain::vo::id::test_uuid;
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;

    fn ctx() -> Context {
        Context::new(
            ContextId::from_uuid(test_uuid(1)),
            ContextName::try_from("core").unwrap(),
            BTreeSet::new(),
        )
        .unwrap()
    }

    fn clauses(n: u128) -> Vec<(ClauseId, ClauseContent)> {
        vec![(
            ClauseId::from_uuid(test_uuid(n)),
            ClauseContent::new(
                ClauseLabel::try_from("C1").unwrap(),
                ClauseText::try_from("a").unwrap(),
            ),
        )]
    }

    fn title(t: &str) -> Title {
        Title::try_from(t).unwrap()
    }

    #[test]
    fn a_new_context_is_based_on_its_interfaces_and_has_no_assets() {
        let c = ctx();
        assert_eq!(c.basis(), Basis::Interface);
        assert_eq!(
            (
                c.contracts().count(),
                c.interfaces().count(),
                c.harnesses().count()
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn a_context_cannot_use_itself() {
        let id = ContextId::from_uuid(test_uuid(1));
        let name = ContextName::try_from("core").unwrap();
        assert_eq!(
            Context::new(id, name, BTreeSet::from([id])),
            Err(ContextError::UsesItself)
        );
        let mut c = ctx();
        assert_eq!(
            c.set_uses(BTreeSet::from([id])),
            Err(ContextError::UsesItself)
        );
    }

    #[test]
    fn a_context_is_based_on_its_contracts_only_with_one_and_only_once() {
        let mut c = ctx();
        assert_eq!(c.base_on_contract(), Err(ContextError::NoContract));
        c.put_contract(
            ContractId::from_uuid(test_uuid(100)),
            title("T"),
            clauses(10),
        )
        .unwrap();
        c.base_on_contract().unwrap();
        assert_eq!(c.basis(), Basis::Contract);
        assert_eq!(c.base_on_contract(), Err(ContextError::AlreadyOnContract));
    }

    #[test]
    fn a_context_holds_several_of_an_asset_and_putting_one_again_revises_it() {
        let mut c = ctx();
        let a = ContractId::from_uuid(test_uuid(100));
        let b = ContractId::from_uuid(test_uuid(101));
        c.put_contract(a, title("A"), clauses(10)).unwrap();
        c.put_contract(b, title("B"), clauses(11)).unwrap();
        c.put_contract(a, title("A2"), clauses(10)).unwrap();
        assert_eq!(c.contracts().count(), 2);
        assert_eq!(c.contract(a).unwrap().revision().number(), 2);
        assert_eq!(c.contract(b).unwrap().revision().number(), 1);
        assert_eq!(
            c.contract_of(ClauseId::from_uuid(test_uuid(11)))
                .unwrap()
                .id(),
            b
        );
    }

    #[test]
    fn retiring_an_asset_hands_it_back_and_moves_the_context_and_a_missing_one_moves_nothing() {
        let mut c = ctx();
        let id = ContractId::from_uuid(test_uuid(100));
        c.put_contract(id, title("A"), clauses(10)).unwrap();
        let rev = c.revision();
        let taken = c.retire_contract(id).unwrap();
        assert_eq!(taken.id(), id);
        assert!(c.contract(id).is_none());
        assert_eq!(c.revision(), rev.next());
        assert_eq!(
            c.retire_contract(id),
            Err(ContextError::NoSuchAsset {
                kind: "contract",
                id: id.as_uuid()
            })
        );
        assert!(matches!(
            c.retire_interface(InterfaceId::from_uuid(test_uuid(1))),
            Err(ContextError::NoSuchAsset { .. })
        ));
        assert!(matches!(
            c.retire_harness(HarnessId::from_uuid(test_uuid(1))),
            Err(ContextError::NoSuchAsset { .. })
        ));
        assert_eq!(c.revision(), rev.next());
    }

    #[test]
    fn the_interface_of_an_item_is_found() {
        let mut c = ctx();
        let id = InterfaceId::from_uuid(test_uuid(200));
        let item = ItemId::from_uuid(test_uuid(20));
        c.put_interface(
            id,
            Source::new("fn f()\n".to_string()),
            vec![(
                item,
                ItemKey::try_from("f").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        assert_eq!(c.interface_of(item).unwrap().id(), id);
        assert!(c.interface_of(ItemId::from_uuid(test_uuid(99))).is_none());
    }
}
