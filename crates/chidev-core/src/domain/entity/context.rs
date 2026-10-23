//! The Context: a unit of the Project and the assets it owns (its Contract, Interface and Harness).

use std::collections::BTreeSet;

use thiserror::Error;

use super::contract::{Contract, ContractError};
use super::harness::{CaseFound, Harness, HarnessError};
use super::interface::{Interface, InterfaceError, ItemFound};
use crate::domain::vo::clause::ClauseContent;
use crate::domain::vo::id::{ClauseId, ContextId, ContractId, HarnessId, InterfaceId};
use crate::domain::vo::name::ContextName;
use crate::domain::vo::source::Source;
use crate::domain::vo::title::Title;

/// What a Context is checked against. It only goes up: a Context once based on its Contract cannot quietly stop
/// owing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Basis {
    /// The Interface and the Harness are enough; a Contract is optional.
    Interface,
    /// The Contract is the basis and is required.
    Contract,
}

/// Which rule of a Context a change would break. The change is not made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContextError {
    /// A Context uses itself.
    #[error("a context cannot use itself")]
    UsesItself,
    /// The Context is already based on its Contract.
    #[error("the context is already based on its contract")]
    AlreadyOnContract,
    /// The Context is to be based on a Contract it does not have.
    #[error("the context has no contract to be based on")]
    NoContract,
    /// The Context has a Contract with another id; an asset is revised, not swapped for another.
    #[error("the context has the contract {has}, not {given}")]
    OtherContract {
        /// The id it has.
        has: ContractId,
        /// The id given.
        given: ContractId,
    },
    /// The Context has an Interface with another id.
    #[error("the context has the interface {has}, not {given}")]
    OtherInterface {
        /// The id it has.
        has: InterfaceId,
        /// The id given.
        given: InterfaceId,
    },
    /// The Context has a Harness with another id.
    #[error("the context has the harness {has}, not {given}")]
    OtherHarness {
        /// The id it has.
        has: HarnessId,
        /// The id given.
        given: HarnessId,
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

/// A unit of the Project and what it owns. The assets are optional because a Context is made before they are
/// written. Its changes go through the Project, which checks the rules between Contexts (what a Harness may
/// cite, what a Context may use) before it keeps one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    id: ContextId,
    name: ContextName,
    basis: Basis,
    uses: BTreeSet<ContextId>,
    contract: Option<Contract>,
    interface: Option<Interface>,
    harness: Option<Harness>,
}

impl Context {
    /// A Context based on its Interface, with no assets yet.
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
            contract: None,
            interface: None,
            harness: None,
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

    /// What it promises.
    pub fn contract(&self) -> Option<&Contract> {
        self.contract.as_ref()
    }

    /// What it shows.
    pub fn interface(&self) -> Option<&Interface> {
        self.interface.as_ref()
    }

    /// The cases that check it.
    pub fn harness(&self) -> Option<&Harness> {
        self.harness.as_ref()
    }

    /// Sets the Contexts it uses.
    pub(super) fn set_uses(&mut self, uses: BTreeSet<ContextId>) -> Result<(), ContextError> {
        if uses.contains(&self.id) {
            return Err(ContextError::UsesItself);
        }
        self.uses = uses;
        Ok(())
    }

    /// Bases the Context on its Contract, which it must have. There is no way back.
    pub(super) fn base_on_contract(&mut self) -> Result<(), ContextError> {
        if self.basis == Basis::Contract {
            return Err(ContextError::AlreadyOnContract);
        }
        if self.contract.is_none() {
            return Err(ContextError::NoContract);
        }
        self.basis = Basis::Contract;
        Ok(())
    }

    /// Makes the Contract with this id, or revises the one it has; a Contract with another id is refused.
    pub(super) fn put_contract(
        &mut self,
        id: ContractId,
        title: Title,
        clauses: Vec<(ClauseId, ClauseContent)>,
    ) -> Result<(), ContextError> {
        match &mut self.contract {
            None => self.contract = Some(Contract::new(id, title, clauses)?),
            Some(c) if c.id() == id => c.revise(title, clauses)?,
            Some(c) => {
                return Err(ContextError::OtherContract {
                    has: c.id(),
                    given: id,
                });
            }
        }
        Ok(())
    }

    /// Makes the Interface with this id, or revises the one it has; an Interface with another id is refused.
    pub(super) fn put_interface(
        &mut self,
        id: InterfaceId,
        source: Source,
        items: Vec<ItemFound>,
    ) -> Result<(), ContextError> {
        match &mut self.interface {
            None => self.interface = Some(Interface::new(id, source, items)?),
            Some(i) if i.id() == id => i.revise(source, items)?,
            Some(i) => {
                return Err(ContextError::OtherInterface {
                    has: i.id(),
                    given: id,
                });
            }
        }
        Ok(())
    }

    /// Makes the Harness with this id, or revises the one it has; a Harness with another id is refused.
    pub(super) fn put_harness(
        &mut self,
        id: HarnessId,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), ContextError> {
        match &mut self.harness {
            None => self.harness = Some(Harness::new(id, source, cases)?),
            Some(h) if h.id() == id => h.revise(source, cases)?,
            Some(h) => {
                return Err(ContextError::OtherHarness {
                    has: h.id(),
                    given: id,
                });
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

    fn ctx() -> Context {
        Context::new(
            ContextId::from_uuid(test_uuid(1)),
            ContextName::try_from("core").unwrap(),
            BTreeSet::new(),
        )
        .unwrap()
    }

    fn clauses() -> Vec<(ClauseId, ClauseContent)> {
        vec![(
            ClauseId::from_uuid(test_uuid(10)),
            ClauseContent::new(
                ClauseLabel::try_from("C1").unwrap(),
                ClauseText::try_from("a").unwrap(),
            ),
        )]
    }

    #[test]
    fn a_new_context_is_based_on_its_interface_and_has_no_assets() {
        let c = ctx();
        assert_eq!(c.basis(), Basis::Interface);
        assert!(c.contract().is_none() && c.interface().is_none() && c.harness().is_none());
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
    fn a_context_is_based_on_its_contract_only_with_one_and_only_once() {
        let mut c = ctx();
        assert_eq!(c.base_on_contract(), Err(ContextError::NoContract));
        let id = ContractId::from_uuid(test_uuid(100));
        c.put_contract(id, Title::try_from("T").unwrap(), clauses())
            .unwrap();
        c.base_on_contract().unwrap();
        assert_eq!(c.basis(), Basis::Contract);
        assert_eq!(c.base_on_contract(), Err(ContextError::AlreadyOnContract));
    }

    #[test]
    fn putting_an_asset_again_revises_it_and_another_id_is_refused() {
        let mut c = ctx();
        let id = ContractId::from_uuid(test_uuid(100));
        c.put_contract(id, Title::try_from("T").unwrap(), clauses())
            .unwrap();
        c.put_contract(id, Title::try_from("T2").unwrap(), clauses())
            .unwrap();
        assert_eq!(c.contract().unwrap().revision().number(), 2);
        let other = ContractId::from_uuid(test_uuid(101));
        assert_eq!(
            c.put_contract(other, Title::try_from("T").unwrap(), clauses()),
            Err(ContextError::OtherContract {
                has: id,
                given: other
            })
        );
    }
}
