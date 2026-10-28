//! What a change request points at, in the names a person writes: the input that names a target of any kind, its
//! resolution to the id the domain works with, and the way back to names for what a use case reports.

use serde::Serialize;

use super::ports::external::ExternalExecutor;
use super::ports::{AssetKind, AssetName, IdSource, Store};
use super::{App, AppError, ClauseRef, ItemRef, context_id};
use crate::domain::entity::project::Project;
use crate::domain::vo::id::{ContractId, HarnessId, InterfaceId};
use crate::domain::vo::retired::Retired;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::{ExpectKind, Target};

/// A target of any of the eight kinds, by the names a person writes: the tree of the assets (Project, Context,
/// Contract and its Clauses, Interface and its Items, Harness and its Cases).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetInput {
    /// The Project.
    Project,
    /// A Context, by name.
    Context(String),
    /// A Contract of a Context.
    Contract {
        /// The Context.
        context: String,
        /// The Contract.
        contract: String,
    },
    /// An Interface of a Context.
    Interface {
        /// The Context.
        context: String,
        /// The Interface.
        interface: String,
    },
    /// A Harness of a Context.
    Harness {
        /// The Context.
        context: String,
        /// The Harness.
        harness: String,
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
}

/// A target by name, in the one form a reference to it is written in on the wire: `{"project": true}`,
/// `{"context": "core"}`, `{"clause": {"context": ..., "contract": ..., "label": ...}}`, and so on. What a use case
/// reports and what a suggestion proposes are written with it, so that they read back as the references an operation
/// takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRef {
    /// The Project, as `{"project": true}`.
    Project(bool),
    /// A Context, by name.
    Context(String),
    /// A Contract of a Context.
    Contract {
        /// The Context.
        context: String,
        /// The Contract.
        contract: String,
    },
    /// An Interface of a Context.
    Interface {
        /// The Context.
        context: String,
        /// The Interface.
        interface: String,
    },
    /// A Harness of a Context.
    Harness {
        /// The Context.
        context: String,
        /// The Harness.
        harness: String,
    },
    /// An Item, by the Interface's name and the key.
    Item {
        /// The Context; none within a Harness.
        context: Option<String>,
        /// The Interface.
        interface: String,
        /// The key of the Item.
        key: String,
    },
    /// A Clause, by the Contract's name and the label.
    Clause {
        /// The Context; none within a Harness.
        context: Option<String>,
        /// The Contract.
        contract: String,
        /// The label of the Clause.
        label: String,
    },
    /// A Case of a Harness.
    Case {
        /// The Context.
        context: String,
        /// The Harness.
        harness: String,
        /// The Case's name.
        name: String,
    },
}

impl From<TargetInput> for TargetRef {
    fn from(t: TargetInput) -> TargetRef {
        match t {
            TargetInput::Project => TargetRef::Project(true),
            TargetInput::Context(name) => TargetRef::Context(name),
            TargetInput::Contract { context, contract } => {
                TargetRef::Contract { context, contract }
            }
            TargetInput::Interface { context, interface } => {
                TargetRef::Interface { context, interface }
            }
            TargetInput::Harness { context, harness } => TargetRef::Harness { context, harness },
            TargetInput::Item(r) => TargetRef::Item {
                context: r.context,
                interface: r.interface,
                key: r.key,
            },
            TargetInput::Clause(r) => TargetRef::Clause {
                context: r.context,
                contract: r.contract,
                label: r.label,
            },
            TargetInput::Case {
                context,
                harness,
                name,
            } => TargetRef::Case {
                context,
                harness,
                name,
            },
        }
    }
}

/// A target (or keep) a change request recorded when it was opened, in the names a person wrote it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedExpect {
    /// Whether the target is to be revised or retired (`target`), or kept as it is (`keep`).
    pub kind: ExpectKind,
    /// The target, by name.
    pub target: TargetInput,
    /// The revision the target had when the change request was opened.
    pub recorded: Revision,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// The target `t` names, which must be in `p`.
    pub(super) async fn target_of(&self, p: &Project, t: &TargetInput) -> Result<Target, AppError> {
        Ok(match t {
            TargetInput::Project => Target::Project,
            TargetInput::Context(name) => Target::Context(context_id(p, name)?),
            TargetInput::Contract { context, contract } => {
                let ctx = context_id(p, context)?;
                let id = ContractId::from_uuid(
                    self.asset(ctx, context, AssetKind::Contract, contract)
                        .await?,
                );
                if p.context(ctx).and_then(|c| c.contract(id)).is_none() {
                    return Err(missing(
                        p,
                        Target::Contract(id),
                        context,
                        AssetKind::Contract,
                        contract,
                    ));
                }
                Target::Contract(id)
            }
            TargetInput::Interface { context, interface } => {
                let ctx = context_id(p, context)?;
                let id = InterfaceId::from_uuid(
                    self.asset(ctx, context, AssetKind::Interface, interface)
                        .await?,
                );
                if p.context(ctx).and_then(|c| c.interface(id)).is_none() {
                    return Err(missing(
                        p,
                        Target::Interface(id),
                        context,
                        AssetKind::Interface,
                        interface,
                    ));
                }
                Target::Interface(id)
            }
            TargetInput::Harness { context, harness } => {
                let ctx = context_id(p, context)?;
                let id = HarnessId::from_uuid(
                    self.asset(ctx, context, AssetKind::Harness, harness)
                        .await?,
                );
                if p.context(ctx).and_then(|c| c.harness(id)).is_none() {
                    return Err(missing(
                        p,
                        Target::Harness(id),
                        context,
                        AssetKind::Harness,
                        harness,
                    ));
                }
                Target::Harness(id)
            }
            TargetInput::Item(r) => Target::Item(self.item(p, None, r).await?.0),
            TargetInput::Clause(r) => Target::Clause(self.clause(p, None, r).await?.0),
            TargetInput::Case {
                context,
                harness,
                name,
            } => Target::Case(self.case_named(p, context, harness, name).await?),
        })
    }

    /// The target `target` of `p` by name, the way a person would write it; `None` when `p` does not have it.
    pub(super) async fn name_target(
        &self,
        p: &Project,
        target: Target,
    ) -> Result<Option<TargetInput>, AppError> {
        let asset = |id: uuid::Uuid| self.name_of(id);
        Ok(match target {
            Target::Project => Some(TargetInput::Project),
            Target::Context(id) => p
                .context(id)
                .map(|c| TargetInput::Context(c.name().to_string())),
            Target::Contract(id) => match p.contexts().find(|c| c.contract(id).is_some()) {
                Some(c) => Some(TargetInput::Contract {
                    context: c.name().to_string(),
                    contract: asset(id.as_uuid()).await?,
                }),
                None => None,
            },
            Target::Interface(id) => match p.contexts().find(|c| c.interface(id).is_some()) {
                Some(c) => Some(TargetInput::Interface {
                    context: c.name().to_string(),
                    interface: asset(id.as_uuid()).await?,
                }),
                None => None,
            },
            Target::Harness(id) => match p.contexts().find(|c| c.harness(id).is_some()) {
                Some(c) => Some(TargetInput::Harness {
                    context: c.name().to_string(),
                    harness: asset(id.as_uuid()).await?,
                }),
                None => None,
            },
            Target::Item(id) => {
                let found = p.contexts().find_map(|c| {
                    let i = c.interface_of(id)?;
                    let it = i.item(id)?;
                    Some((c.name().to_string(), i.id(), it.key().to_string()))
                });
                match found {
                    Some((context, interface, key)) => Some(TargetInput::Item(ItemRef {
                        context: Some(context),
                        interface: asset(interface.as_uuid()).await?,
                        key,
                    })),
                    None => None,
                }
            }
            Target::Clause(id) => {
                let found = p.contexts().find_map(|c| {
                    let k = c.contract_of(id)?;
                    let cl = k.clause(id)?;
                    Some((c.name().to_string(), k.id(), cl.label().to_string()))
                });
                match found {
                    Some((context, contract, label)) => Some(TargetInput::Clause(ClauseRef {
                        context: Some(context),
                        contract: asset(contract.as_uuid()).await?,
                        label,
                    })),
                    None => None,
                }
            }
            Target::Case(id) => {
                let found = p.contexts().find_map(|c| {
                    let h = c.harness_of(id)?;
                    let k = h.case(id)?;
                    Some((c.name().to_string(), h.id(), k.name().to_string()))
                });
                match found {
                    Some((context, harness, name)) => Some(TargetInput::Case {
                        context,
                        harness: asset(harness.as_uuid()).await?,
                        name,
                    }),
                    None => None,
                }
            }
        })
    }

    /// A target, in words, with the names a use case is about to give: for an Item or a Clause the asset's name and
    /// the key or the label, for any other the kind and the name. A target the Project does not have says so.
    pub(super) async fn describe_in(
        &self,
        p: &Project,
        target: Target,
        pending: &[AssetName],
    ) -> Result<String, AppError> {
        if let Some(r) = p.retired_of(target) {
            return self.retired_words(r, pending).await;
        }
        Ok(match target {
            Target::Project => "project".to_string(),
            Target::Context(id) => match p.context(id) {
                Some(c) => format!("context {}", c.name()),
                None => format!("context {id} (gone)"),
            },
            Target::Contract(id) => {
                let here = p.contexts().any(|c| c.contract(id).is_some());
                self.asset_words("contract", id.as_uuid(), here, pending)
                    .await?
            }
            Target::Interface(id) => {
                let here = p.contexts().any(|c| c.interface(id).is_some());
                self.asset_words("interface", id.as_uuid(), here, pending)
                    .await?
            }
            Target::Harness(id) => {
                let here = p.contexts().any(|c| c.harness(id).is_some());
                self.asset_words("harness", id.as_uuid(), here, pending)
                    .await?
            }
            Target::Item(id) => {
                let found = p.contexts().find_map(|c| {
                    let i = c.interface_of(id)?;
                    let it = i.item(id)?;
                    Some((i.id().as_uuid(), it.key().to_string()))
                });
                match found {
                    Some((asset, what)) => {
                        format!("{} {what}", self.name_in(asset, pending).await?)
                    }
                    None => format!("item {id} (gone)"),
                }
            }
            Target::Clause(id) => {
                let found = p.contexts().find_map(|c| {
                    let k = c.contract_of(id)?;
                    let cl = k.clause(id)?;
                    Some((k.id().as_uuid(), cl.label().to_string()))
                });
                match found {
                    Some((asset, what)) => {
                        format!("{} {what}", self.name_in(asset, pending).await?)
                    }
                    None => format!("clause {id} (gone)"),
                }
            }
            Target::Case(id) => {
                let found = p.contexts().find_map(|c| {
                    let h = c.harness_of(id)?;
                    let k = h.case(id)?;
                    Some((h.id().as_uuid(), k.name().to_string()))
                });
                match found {
                    Some((asset, what)) => {
                        format!("case {} {what}", self.name_in(asset, pending).await?)
                    }
                    None => format!("case {id} (gone)"),
                }
            }
        })
    }

    /// A retired target in words, as a live one is worded and marked `(retired)`: the kind and name of a Context or
    /// an asset, the asset's name and the label, key or name for a part.
    async fn retired_words(&self, r: &Retired, pending: &[AssetName]) -> Result<String, AppError> {
        let target = r.target();
        Ok(match (r.holder(), r.label(), target.as_uuid()) {
            (Some(holder), Some(label), _) => {
                let asset = self.name_in(holder, pending).await?;
                match target {
                    Target::Case(_) => format!("case {asset} {label} (retired)"),
                    _ => format!("{asset} {label} (retired)"),
                }
            }
            (_, _, Some(id)) if !matches!(target, Target::Context(_)) => {
                format!(
                    "{} {} (retired)",
                    target.word(),
                    self.name_in(id, pending).await?
                )
            }
            _ => format!("{} {} (retired)", target.word(), r.context()),
        })
    }

    /// An asset in words: its kind and name, and `(gone)` when the Project does not have it any more.
    async fn asset_words(
        &self,
        kind: &str,
        id: uuid::Uuid,
        here: bool,
        pending: &[AssetName],
    ) -> Result<String, AppError> {
        let name = self.name_in(id, pending).await?;
        Ok(if here {
            format!("{kind} {name}")
        } else {
            format!("{kind} {name} (gone)")
        })
    }
}

/// Why the asset `target` named `name` cannot be pointed at: it was retired, or the Context never had it.
fn missing(p: &Project, target: Target, context: &str, kind: AssetKind, name: &str) -> AppError {
    if p.retired_of(target).is_some() {
        return AppError::AssetRetired {
            context: context.to_string(),
            kind: kind.word(),
            name: name.to_string(),
        };
    }
    AppError::UnknownAsset {
        context: context.to_string(),
        kind: kind.word(),
        name: name.to_string(),
    }
}
