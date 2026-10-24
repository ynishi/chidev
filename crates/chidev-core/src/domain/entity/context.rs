//! The Context: a unit of the Project and the assets it owns (its Contracts, Interfaces and Harnesses).

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::contract::{Contract, ContractError};
use super::harness::{CaseFound, Harness, HarnessError};
use super::interface::{Interface, InterfaceError, ItemFound};
use crate::domain::vo::clause::ClauseContent;
use crate::domain::vo::id::{ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId};
use crate::domain::vo::name::ContextName;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    id: ContextId,
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
            name,
            basis: Basis::Interface,
            uses,
            contracts: BTreeMap::new(),
            interfaces: BTreeMap::new(),
            harnesses: BTreeMap::new(),
        })
    }

    /// A Context as a store kept it, with its basis and assets. It does not use itself, and a Context based on its
    /// Contracts has one at least.
    pub(crate) fn restore(
        id: ContextId,
        name: ContextName,
        basis: Basis,
        uses: BTreeSet<ContextId>,
        contracts: Vec<Contract>,
        interfaces: Vec<Interface>,
        harnesses: Vec<Harness>,
    ) -> Result<Context, ContextError> {
        if uses.contains(&id) {
            return Err(ContextError::UsesItself);
        }
        if basis == Basis::Contract && contracts.is_empty() {
            return Err(ContextError::NoContract);
        }
        Ok(Context {
            id,
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

    /// Sets the Contexts it uses.
    pub(super) fn set_uses(&mut self, uses: BTreeSet<ContextId>) -> Result<(), ContextError> {
        if uses.contains(&self.id) {
            return Err(ContextError::UsesItself);
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
        Ok(())
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
            }
        }
        Ok(())
    }

    /// Makes the Harness with this id, or revises it when the Context has it.
    pub(super) fn put_harness(
        &mut self,
        id: HarnessId,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), ContextError> {
        match self.harnesses.get_mut(&id) {
            Some(h) => h.revise(source, cases)?,
            None => {
                self.harnesses.insert(id, Harness::new(id, source, cases)?);
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
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::signature::Signature;

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
    fn the_interface_of_an_item_is_found() {
        let mut c = ctx();
        let id = InterfaceId::from_uuid(test_uuid(200));
        let item = ItemId::from_uuid(test_uuid(20));
        c.put_interface(
            id,
            Source::new("fn f()\n".to_string()),
            vec![(
                item,
                Signature::try_from("fn f()").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        assert_eq!(c.interface_of(item).unwrap().id(), id);
        assert!(c.interface_of(ItemId::from_uuid(test_uuid(99))).is_none());
    }
}
