//! `view`: the part of the model an agent asks to read, by the sections it names and the Contexts it names.
//!
//! The part is cut from the same whole view a judged rule is given (see [`ModelView`]), so what an agent reads here
//! is, entry for entry, what a rule would be judged on, and a reader that is to be kept from some part of the model
//! (a reviewer that must not see the Harnesses) is given the sections that leave it out.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::model_view::{ModelSlice, ModelView, ViewSelection};
use super::ports::external::ExternalExecutor;
use super::ports::{IdSource, NamedAsset, Store, StoreRevision};
use super::{App, AppError, context_ids};
use crate::domain::entity::context::Context;
use crate::domain::vo::model_section::ModelSection;

/// What a `view` asks for, in the names a person writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewInput {
    /// The Contexts to read, by name; none: every Context. An empty list is no Context at all, not every one.
    pub contexts: Option<Vec<String>>,
    /// The sections to read. At least one: nothing is read by default, so a reader gets only what it was given.
    pub sections: BTreeSet<ModelSection>,
}

/// What a `view` gives: the part of the model asked for, and where the store stood when it was read.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewRead {
    /// The revision of the store the view was read at. The slice has no revision of its own, because an external
    /// check is sent the same slice and keeps its own `read_at`.
    pub store_revision: StoreRevision,
    /// The part of the model asked for.
    pub view: ModelSlice,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// The sections of the model `input` names, of the Contexts it names (all when none). Refused when no section
    /// is named, and when a Context is not in the Project.
    pub async fn view(&self, input: &ViewInput) -> Result<ViewRead, AppError> {
        if input.sections.is_empty() {
            return Err(AppError::ViewSectionsEmpty);
        }
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let contexts = input
            .contexts
            .as_deref()
            .map(|names| context_ids(&p, names))
            .transpose()?;
        let (crs, cruxes) = self.requests_and_cruxes().await?;
        // the names are those of the assets of the Contexts read, and only an asset section shows them
        let names = if input
            .sections
            .iter()
            .any(|s| !matches!(s, ModelSection::ChangeRequests | ModelSection::Cruxes))
        {
            let read = |c: &&Context| contexts.as_ref().is_none_or(|ids| ids.contains(&c.id()));
            self.names_in_model(p.contexts().filter(read), &[]).await?
        } else {
            BTreeMap::new()
        };
        let selection = ViewSelection {
            contexts,
            sections: input.sections.clone(),
        };
        Ok(ViewRead {
            store_revision: at,
            view: ModelView::build(at.0, &p, &crs, &cruxes, &names).select(&selection),
        })
    }

    /// The names a person gave the Contracts, Interfaces and Harnesses of `contexts`, by id. `pending` are the names
    /// of assets a change is about to make, which the store does not have yet; they stand before the store's.
    pub(super) async fn names_in_model<'a>(
        &self,
        contexts: impl Iterator<Item = &'a Context>,
        pending: &[NamedAsset],
    ) -> Result<BTreeMap<Uuid, String>, AppError> {
        let mut names = BTreeMap::new();
        for c in contexts {
            let ids = c
                .contracts()
                .map(|x| x.id().as_uuid())
                .chain(c.interfaces().map(|x| x.id().as_uuid()))
                .chain(c.harnesses().map(|x| x.id().as_uuid()));
            for id in ids {
                names.insert(id, self.name_in(id, pending).await?.to_string());
            }
        }
        Ok(names)
    }
}
