//! Shared subject input and resolution for findings and change-request addresses.

use super::ports::external::ExternalExecutor;
use super::ports::{AssetKind, IdSource, Store};
use super::{App, AppError, ClauseRef, ItemRef, context_id};
use crate::domain::entity::project::Project;
use crate::domain::vo::finding::Subject;
use crate::domain::vo::id::{CaseId, HarnessId, InterfaceId};

/// What a finding is about, addressed by the names the model view shows.
/// Shared by change-request addresses and judged reports; `None` at their call sites means the whole Project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectInput {
    /// A Context, by name.
    Context(String),
    /// An Interface of a Context.
    Interface {
        /// The Context.
        context: String,
        /// The Interface.
        interface: String,
    },
    /// An Item, by the Interface's name and the key.
    Item(ItemRef),
    /// A Clause, by the Contract's name and the label.
    Clause(ClauseRef),
    /// A Case of a Harness.
    Case {
        /// The Context.
        context: String,
        /// The Harness.
        harness: String,
        /// The Case's name.
        name: String,
    },
    /// A change request, by title.
    ChangeRequest(String),
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Resolves a shared subject input to its stable domain identity.
    pub(super) async fn subject_of(
        &self,
        p: &Project,
        s: &SubjectInput,
    ) -> Result<Subject, AppError> {
        Ok(match s {
            SubjectInput::Context(name) => Subject::Context(context_id(p, name)?),
            SubjectInput::Interface { context, interface } => {
                let ctx = context_id(p, context)?;
                let id = self
                    .asset(ctx, context, AssetKind::Interface, interface)
                    .await?;
                let id = InterfaceId::from_uuid(id);
                if p.context(ctx).and_then(|c| c.interface(id)).is_none() {
                    return Err(AppError::UnknownAsset {
                        context: context.clone(),
                        kind: "interface",
                        name: interface.clone(),
                    });
                }
                Subject::Interface(id)
            }
            SubjectInput::Item(r) => Subject::Item(self.item(p, None, r).await?.0),
            SubjectInput::Clause(r) => Subject::Clause(self.clause(p, None, r).await?.0),
            SubjectInput::Case {
                context,
                harness,
                name,
            } => Subject::Case(self.case_named(p, context, harness, name).await?),
            SubjectInput::ChangeRequest(title) => {
                Subject::ChangeRequest(self.change_request_titled(title).await?.id())
            }
        })
    }

    /// The Case named `name` in the Harness `harness` of the Context `context`. A name that two Cases of the
    /// Harness share does not say which.
    pub(super) async fn case_named(
        &self,
        p: &Project,
        context: &str,
        harness: &str,
        name: &str,
    ) -> Result<CaseId, AppError> {
        let ctx = context_id(p, context)?;
        let id = HarnessId::from_uuid(
            self.asset(ctx, context, AssetKind::Harness, harness)
                .await?,
        );
        let cases: Vec<_> = p
            .context(ctx)
            .and_then(|c| c.harness(id))
            .map(|h| {
                h.cases()
                    .iter()
                    .filter(|k| k.name().as_str() == name)
                    .collect()
            })
            .unwrap_or_default();
        match cases.as_slice() {
            [one] => Ok(one.id()),
            [] => Err(AppError::UnknownCase {
                context: context.to_string(),
                harness: harness.to_string(),
                name: name.to_string(),
            }),
            _ => Err(AppError::Input(format!(
                "the harness `{harness}` has several cases named `{name}`; \
                 a reference cannot say which"
            ))),
        }
    }
}
