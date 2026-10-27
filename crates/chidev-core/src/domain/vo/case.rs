//! The content of a Case of a Harness that is not its code: its name and what it cites.

use std::collections::BTreeMap;
use std::fmt;

use thiserror::Error;

use super::id::{ClauseId, ItemId};
use super::revision::Revision;
use super::target::Target;

/// Why a name or a set of citations is not the content of a Case.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaseContentError {
    /// The name is empty or only white space.
    #[error("a case name is empty")]
    EmptyName,
    /// The Case cites no Item; a Case checks something the Interface shows.
    #[error("a case cites no item")]
    NoItem,
}

/// The name of a Case as the code writes it (the test function's name). It is content, not the identity (that is
/// the `CaseId`), and two Cases may have the same name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaseName(String);

impl CaseName {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for CaseName {
    type Error = CaseContentError;

    fn try_from(text: &str) -> Result<CaseName, CaseContentError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CaseContentError::EmptyName);
        }
        Ok(CaseName(text.to_string()))
    }
}

impl fmt::Display for CaseName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a Case cites, each with the revision it was written against: Items and Clauses. A Case of a Harness of
/// tests cites one Item or more (it checks something the Interface shows) and any number of Clauses (none when the
/// Context has no Contract); a Case of a sim Harness cites Clauses only (the program it drives is named by the
/// Harness, not cited). Which of the two a Case is allowed is a rule of its Harness, not of the citations. Each
/// target is there once, in the order of its id. A target whose revision has moved past the one recorded is one
/// the Case has not been brought up to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaseCites {
    items: BTreeMap<ItemId, Revision>,
    clauses: BTreeMap<ClauseId, Revision>,
}

impl CaseCites {
    /// The citations of a Case that checks code; refused when it cites no Item.
    pub fn new(
        items: BTreeMap<ItemId, Revision>,
        clauses: BTreeMap<ClauseId, Revision>,
    ) -> Result<CaseCites, CaseContentError> {
        if items.is_empty() {
            return Err(CaseContentError::NoItem);
        }
        Ok(CaseCites::of(items, clauses))
    }

    /// The citations as given, with no Item if there is none. Whether the Harness the Case is in takes a Case
    /// with or without Items is the Harness's rule.
    pub fn of(
        items: BTreeMap<ItemId, Revision>,
        clauses: BTreeMap<ClauseId, Revision>,
    ) -> CaseCites {
        CaseCites { items, clauses }
    }

    /// The Items it cites, with the revision each was cited at.
    pub fn items(&self) -> &BTreeMap<ItemId, Revision> {
        &self.items
    }

    /// The Clauses it cites, with the revision each was cited at.
    pub fn clauses(&self) -> &BTreeMap<ClauseId, Revision> {
        &self.clauses
    }

    /// What it cites as targets: the Items, then the Clauses, each in the order of its id.
    pub fn targets(&self) -> impl Iterator<Item = Target> + '_ {
        self.items
            .keys()
            .map(|i| Target::Item(*i))
            .chain(self.clauses.keys().map(|c| Target::Clause(*c)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;

    #[test]
    fn a_case_name_is_kept_without_the_space_around_it_and_not_blank() {
        assert_eq!(
            CaseName::try_from(" push_then_pop ").unwrap().as_str(),
            "push_then_pop"
        );
        assert_eq!(CaseName::try_from(" "), Err(CaseContentError::EmptyName));
    }

    #[test]
    fn citations_need_an_item_and_may_have_no_clause() {
        let item = ItemId::from_uuid(test_uuid(1));
        let cites =
            CaseCites::new(BTreeMap::from([(item, Revision::first())]), BTreeMap::new()).unwrap();
        assert_eq!(cites.items().get(&item), Some(&Revision::first()));
        assert!(cites.clauses().is_empty());
        let clause = ClauseId::from_uuid(test_uuid(2));
        assert_eq!(
            CaseCites::new(
                BTreeMap::new(),
                BTreeMap::from([(clause, Revision::first())])
            ),
            Err(CaseContentError::NoItem)
        );
    }
}
