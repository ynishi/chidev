//! The record of a thing the Project retired: it is out of the live model, and its id stays reserved.

use thiserror::Error;
use uuid::Uuid;

use super::name::ContextName;
use super::revision::Revision;
use super::target::Target;

/// Why a stored record of a retired thing is not one.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RetiredError {
    /// The Project itself cannot be retired.
    #[error("the project cannot be retired")]
    Project,
    /// A Clause, an Item or a Case is named by the asset that held it and by its own label, key or name.
    #[error("a retired {0} is named by the asset that held it and by its own text")]
    LeafNeedsHolder(&'static str),
    /// A Context or an asset is named by itself alone (the store names the assets).
    #[error("a retired {0} has no holder and no text of its own")]
    WholeHasNoHolder(&'static str),
}

/// What the Project keeps of a thing it retired: which thing, the words that name it later, the revision it had
/// when it went, and the revision of what held it at that moment. The text of the thing (the source, the sentence of
/// a Clause) is not kept: a retired thing is named, not read.
///
/// The names an asset has live beside the model (see `AssetNames`), so the record holds only what the model can
/// say: the name of the Context (its own, or the one the thing was in), and for a Clause, an Item or a Case the id of
/// the asset that held it with its label, key or name. Ids are reserved for good: a retired id is never
/// given again, and a thing is not brought back (make it again, with a new id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retired {
    target: Target,
    context: ContextName,
    holder: Option<Uuid>,
    label: Option<String>,
    last_revision: Revision,
    at: Revision,
}

impl Retired {
    /// The record of a Context or an asset (a Contract, an Interface or a Harness). `context` is the name of the
    /// Context itself, or of the one that held the asset.
    pub(crate) fn whole(
        target: Target,
        context: ContextName,
        last_revision: Revision,
        at: Revision,
    ) -> Retired {
        Retired {
            target,
            context,
            holder: None,
            label: None,
            last_revision,
            at,
        }
    }

    /// The record of a Clause, an Item or a Case: `holder` is the id of the Contract, Interface or Harness that
    /// held it, `label` its label, key or name.
    pub(crate) fn leaf(
        target: Target,
        context: ContextName,
        holder: Uuid,
        label: String,
        last_revision: Revision,
        at: Revision,
    ) -> Retired {
        Retired {
            target,
            context,
            holder: Some(holder),
            label: Some(label),
            last_revision,
            at,
        }
    }

    /// A record as a store kept it. It is a record of a thing the Project can retire, named the way its kind is.
    pub(crate) fn restore(
        target: Target,
        context: ContextName,
        holder: Option<Uuid>,
        label: Option<String>,
        last_revision: Revision,
        at: Revision,
    ) -> Result<Retired, RetiredError> {
        let leaf = match target {
            Target::Project => return Err(RetiredError::Project),
            Target::Context(_)
            | Target::Contract(_)
            | Target::Interface(_)
            | Target::Harness(_) => false,
            Target::Clause(_) | Target::Item(_) | Target::Case(_) => true,
        };
        match (leaf, holder.is_some() && label.is_some()) {
            (true, false) => return Err(RetiredError::LeafNeedsHolder(target.word())),
            (false, _) if holder.is_some() || label.is_some() => {
                return Err(RetiredError::WholeHasNoHolder(target.word()));
            }
            _ => {}
        }
        Ok(Retired {
            target,
            context,
            holder,
            label,
            last_revision,
            at,
        })
    }

    /// What was retired.
    pub fn target(&self) -> Target {
        self.target
    }

    /// The name of the Context: its own for a retired Context, else the one that held the thing.
    pub fn context(&self) -> &ContextName {
        &self.context
    }

    /// For a Clause, an Item or a Case, the id of the Contract, Interface or Harness that held it.
    pub fn holder(&self) -> Option<Uuid> {
        self.holder
    }

    /// For a Clause, an Item or a Case, its label, key or name.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// The revision the thing had when it was retired.
    pub fn last_revision(&self) -> Revision {
        self.last_revision
    }

    /// The revision of what held the thing once it was retired: the Project's for a Context, the Context's for an
    /// asset, the asset's for a Clause, an Item or a Case.
    pub fn at(&self) -> Revision {
        self.at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::{ClauseId, ContextId, test_uuid};

    fn name() -> ContextName {
        ContextName::try_from("core").unwrap()
    }

    #[test]
    fn a_record_is_named_by_its_kind_and_the_project_has_none() {
        let r = Revision::first();
        assert_eq!(
            Retired::restore(Target::Project, name(), None, None, r, r),
            Err(RetiredError::Project)
        );
        let clause = Target::Clause(ClauseId::from_uuid(test_uuid(1)));
        assert_eq!(
            Retired::restore(clause, name(), None, None, r, r),
            Err(RetiredError::LeafNeedsHolder("clause"))
        );
        assert!(
            Retired::restore(clause, name(), Some(test_uuid(2)), Some("C1".into()), r, r).is_ok()
        );
        let context = Target::Context(ContextId::from_uuid(test_uuid(3)));
        assert_eq!(
            Retired::restore(context, name(), Some(test_uuid(2)), None, r, r),
            Err(RetiredError::WholeHasNoHolder("context"))
        );
        assert!(Retired::restore(context, name(), None, None, r, r).is_ok());
    }
}
