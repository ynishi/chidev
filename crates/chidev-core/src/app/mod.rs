//! The use cases: read what they need, decide with [`crate::domain`], record
//! the outcome.
//!
//! What a use case needs from the outside world is a port: a trait declared
//! here, by its consumer, and implemented in [`crate::infra`]. Code here calls
//! ports, never the file system, a process or the environment directly.
//!
//! The use cases speak in the names a person writes (a Context's name, an
//! asset's name, a clause label, a signature, a case name) and turn them into
//! the ids the domain works with: a definition given again under the same
//! label, signature or case name keeps its id, and a new one gets a fresh id.

pub mod ports;

use std::collections::BTreeSet;
use std::fmt::Display;

use thiserror::Error;

use self::ports::{AssetKind, AssetNames, IdSource};
use crate::domain::entity::crux::{Crux, CruxError};
use crate::domain::entity::project::{Project, ProjectError};
use crate::domain::repository::{CruxRepository, PinRepository, ProjectRepository, StoreError};
use crate::domain::service::cite::stale_cites;
use crate::domain::service::crux::{answer, revisions_now, unapplied};
use crate::domain::service::pin::{PinError, StaleReason, Staleness, record, staleness};
use crate::domain::vo::case::{CaseCites, CaseName};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{
    CaseId, ClauseId, ContextId, ContractId, CruxId, HarnessId, InterfaceId, ItemId, PinId,
    ProjectId,
};
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::name::ContextName;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::signature::Signature;
use crate::domain::vo::source::Source;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// Why a use case did nothing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppError {
    /// There is no Project yet.
    #[error("there is no project yet; init one first")]
    NoProject,
    /// There is a Project already.
    #[error("there is a project already")]
    AlreadyInitialized,
    /// No Context has this name.
    #[error("no context is named `{0}`")]
    UnknownContext(String),
    /// No asset of this kind has this name in the Context.
    #[error("the context `{context}` has no {kind} named `{name}`")]
    UnknownAsset {
        /// The Context.
        context: String,
        /// The kind of asset.
        kind: &'static str,
        /// The name.
        name: String,
    },
    /// The Interface declares no Item with this signature.
    #[error("the interface `{interface}` declares no `{signature}`")]
    UnknownItem {
        /// The Interface.
        interface: String,
        /// The signature looked for.
        signature: String,
    },
    /// The Contract has no Clause with this label.
    #[error("the contract `{contract}` has no clause {label}")]
    UnknownClause {
        /// The Contract.
        contract: String,
        /// The label looked for.
        label: String,
    },
    /// No Crux has this title.
    #[error("no crux is titled `{0}`")]
    UnknownCrux(String),
    /// A citation outside a Harness does not say which Context it is in.
    #[error("a citation outside a harness names its context")]
    ContextNotGiven,
    /// A value given is not well formed (a name, a label, a line range, ...).
    #[error("{0}")]
    Input(String),
    /// The change breaks a rule of the Project.
    #[error(transparent)]
    Project(#[from] ProjectError),
    /// The code cannot be pinned.
    #[error(transparent)]
    Pin(#[from] PinError),
    /// The Crux cannot be made or answered.
    #[error(transparent)]
    Crux(#[from] CruxError),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn input(e: impl Display) -> AppError {
    AppError::Input(e.to_string())
}

/// A Clause of a Contract, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseInput {
    /// Its label (`C1`).
    pub label: String,
    /// Its sentence.
    pub text: String,
}

/// An Item of an Interface, as found in the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemInput {
    /// Its declaration.
    pub signature: String,
    /// The first and last line it is on, from 1.
    pub lines: (usize, usize),
    /// The signature it had before, when it changed: the Item keeps its id. Without it, an Item whose signature
    /// matches one the Interface has keeps that id, and any other is new.
    pub replaces: Option<String>,
}

/// A citation of an Item: the Interface (by name) and the signature, in a Context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    /// The Context; within a Harness, its own when not given.
    pub context: Option<String>,
    /// The Interface.
    pub interface: String,
    /// The signature of the Item.
    pub signature: String,
}

/// A citation of a Clause: the Contract (by name) and the label, in a Context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseRef {
    /// The Context; within a Harness, its own when not given.
    pub context: Option<String>,
    /// The Contract.
    pub contract: String,
    /// The label of the Clause.
    pub label: String,
}

/// A Case of a Harness, as found in the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseInput {
    /// Its name.
    pub name: String,
    /// The first and last line it is on, from 1.
    pub lines: (usize, usize),
    /// The Items it checks (one at least).
    pub items: Vec<ItemRef>,
    /// The Clauses it checks.
    pub clauses: Vec<ClauseRef>,
}

/// An asset made or revised: its id and the revision it is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Put<I> {
    /// The asset's id.
    pub id: I,
    /// Its revision after the change.
    pub revision: Revision,
}

/// Where the Pin of one Interface stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinState {
    /// The code of the Interface was never pinned.
    Unpinned,
    /// Nothing the Pin stood on changed.
    Fresh,
    /// The pinned Interface is gone.
    Gone,
    /// These moved, each described.
    Stale(Vec<String>),
}

/// The Pin state of one Interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinStatus {
    /// The Context's name.
    pub context: String,
    /// The Interface's name.
    pub interface: String,
    /// Where its Pin stands.
    pub state: PinState,
}

/// A citation a Case has not been brought up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleCiteStatus {
    /// The Case's name.
    pub case: String,
    /// What it cites, described.
    pub target: String,
    /// The revision it recorded.
    pub cited: u64,
    /// The revision the target has now.
    pub now: u64,
}

/// A Crux answered other than its default whose definitions have not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnappliedStatus {
    /// The Crux's title.
    pub crux: String,
    /// The answer.
    pub answer: String,
    /// The definitions not changed since the answer, described.
    pub unchanged: Vec<String>,
    /// The definitions gone, described.
    pub gone: Vec<String>,
}

/// Where the Project stands: what is pinned and whether it holds, the citations behind their targets, and the
/// answers not yet applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Every Interface, and every Pin whose Interface is gone.
    pub pins: Vec<PinStatus>,
    /// The citations behind their targets.
    pub stale_cites: Vec<StaleCiteStatus>,
    /// The answers not yet applied.
    pub unapplied: Vec<UnappliedStatus>,
}

/// The use cases over one store and one source of ids.
#[derive(Debug)]
pub struct App<S, G> {
    store: S,
    ids: G,
}

impl<S, G> App<S, G>
where
    S: ProjectRepository + PinRepository + CruxRepository + AssetNames,
    G: IdSource,
{
    /// The use cases over `store`, with fresh ids from `ids`.
    pub fn new(store: S, ids: G) -> App<S, G> {
        App { store, ids }
    }

    /// The store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Makes the Project.
    pub fn init(&mut self) -> Result<ProjectId, AppError> {
        if self.store.project()?.is_some() {
            return Err(AppError::AlreadyInitialized);
        }
        let id = ProjectId::from_uuid(self.ids.next_uuid());
        self.store.save_project(&Project::new(id))?;
        Ok(id)
    }

    /// Adds a Context named `name` that uses the Contexts named in `uses`.
    pub fn add_context(&mut self, name: &str, uses: &[String]) -> Result<ContextId, AppError> {
        let mut p = self.project()?;
        let uses = context_ids(&p, uses)?;
        let id = ContextId::from_uuid(self.ids.next_uuid());
        let context = crate::domain::entity::context::Context::new(
            id,
            ContextName::try_from(name).map_err(input)?,
            uses,
        )
        .map_err(ProjectError::from)?;
        p.add_context(context)?;
        self.store.save_project(&p)?;
        Ok(id)
    }

    /// Sets the Contexts the Context named `context` uses.
    pub fn set_uses(&mut self, context: &str, uses: &[String]) -> Result<(), AppError> {
        let mut p = self.project()?;
        let ctx = context_id(&p, context)?;
        let uses = context_ids(&p, uses)?;
        p.set_uses(ctx, uses)?;
        self.store.save_project(&p)?;
        Ok(())
    }

    /// Bases the Context named `context` on its Contracts.
    pub fn base_on_contract(&mut self, context: &str) -> Result<(), AppError> {
        let mut p = self.project()?;
        let ctx = context_id(&p, context)?;
        p.base_on_contract(ctx)?;
        self.store.save_project(&p)?;
        Ok(())
    }

    /// Makes or revises the Contract named `name` of the Context `context`. A Clause keeps its id when its label
    /// is one the Contract has.
    pub fn put_contract(
        &mut self,
        context: &str,
        name: &str,
        title: &str,
        clauses: &[ClauseInput],
    ) -> Result<Put<ContractId>, AppError> {
        let mut p = self.project()?;
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Contract, name)?
            .map(ContractId::from_uuid);
        let old = id.and_then(|id| p.context(ctx).and_then(|c| c.contract(id)));
        let mut given = Vec::with_capacity(clauses.len());
        for c in clauses {
            let label = ClauseLabel::try_from(c.label.as_str()).map_err(input)?;
            let text = ClauseText::try_from(c.text.as_str()).map_err(input)?;
            let clause = match old.and_then(|k| k.clause_by_label(label)) {
                Some(k) => k.id(),
                None => ClauseId::from_uuid(self.ids.next_uuid()),
            };
            given.push((clause, ClauseContent::new(label, text)));
        }
        let id = id.unwrap_or_else(|| ContractId::from_uuid(self.ids.next_uuid()));
        p.put_contract(ctx, id, Title::try_from(title).map_err(input)?, given)?;
        let revision = p
            .context(ctx)
            .and_then(|c| c.contract(id))
            .map(|k| k.revision())
            .unwrap_or_else(Revision::first);
        self.keep(&p, ctx, AssetKind::Contract, name, id.as_uuid())?;
        Ok(Put { id, revision })
    }

    /// Makes or revises the Interface named `name` of the Context `context` from its code and the Items found in
    /// it. An Item keeps its id when its signature (or the one it `replaces`) is one the Interface has.
    pub fn put_interface(
        &mut self,
        context: &str,
        name: &str,
        source: &str,
        items: &[ItemInput],
    ) -> Result<Put<InterfaceId>, AppError> {
        let mut p = self.project()?;
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Interface, name)?
            .map(InterfaceId::from_uuid);
        let old = id.and_then(|id| p.context(ctx).and_then(|c| c.interface(id)));
        let find = |sig: &str| {
            old.and_then(|i| i.items().iter().find(|it| it.signature().as_str() == sig))
                .map(|it| it.id())
        };
        let mut given = Vec::with_capacity(items.len());
        for it in items {
            let signature = Signature::try_from(it.signature.as_str()).map_err(input)?;
            let lines = LineRange::new(it.lines.0, it.lines.1).map_err(input)?;
            let item = match &it.replaces {
                Some(before) => find(before).ok_or_else(|| AppError::UnknownItem {
                    interface: name.to_string(),
                    signature: before.clone(),
                })?,
                None => match find(&it.signature) {
                    Some(item) => item,
                    None => ItemId::from_uuid(self.ids.next_uuid()),
                },
            };
            given.push((item, signature, lines));
        }
        let id = id.unwrap_or_else(|| InterfaceId::from_uuid(self.ids.next_uuid()));
        p.put_interface(ctx, id, Source::new(source.to_string()), given)?;
        let revision = p
            .context(ctx)
            .and_then(|c| c.interface(id))
            .map(|i| i.revision())
            .unwrap_or_else(Revision::first);
        self.keep(&p, ctx, AssetKind::Interface, name, id.as_uuid())?;
        Ok(Put { id, revision })
    }

    /// Makes or revises the Harness named `name` of the Context `context` from its code and the Cases found in
    /// it. A Case keeps its id when its name is one the Harness has. Each citation records the revision its target
    /// has now: putting a Case says it is written against the definitions as they are.
    pub fn put_harness(
        &mut self,
        context: &str,
        name: &str,
        source: &str,
        cases: &[CaseInput],
    ) -> Result<Put<HarnessId>, AppError> {
        let mut p = self.project()?;
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Harness, name)?
            .map(HarnessId::from_uuid);
        let old = id.and_then(|id| p.context(ctx).and_then(|c| c.harness(id)));
        let mut given = Vec::with_capacity(cases.len());
        for c in cases {
            let case_name = CaseName::try_from(c.name.as_str()).map_err(input)?;
            let lines = LineRange::new(c.lines.0, c.lines.1).map_err(input)?;
            let mut items = std::collections::BTreeMap::new();
            for r in &c.items {
                let (item, revision) = self.item(&p, Some(ctx), r)?;
                items.insert(item, revision);
            }
            let mut clauses = std::collections::BTreeMap::new();
            for r in &c.clauses {
                let (clause, revision) = self.clause(&p, Some(ctx), r)?;
                clauses.insert(clause, revision);
            }
            let cites = CaseCites::new(items, clauses).map_err(input)?;
            let case = match old.and_then(|h| h.cases().iter().find(|k| *k.name() == case_name)) {
                Some(k) => k.id(),
                None => CaseId::from_uuid(self.ids.next_uuid()),
            };
            given.push((case, case_name, lines, cites));
        }
        let id = id.unwrap_or_else(|| HarnessId::from_uuid(self.ids.next_uuid()));
        p.put_harness(ctx, id, Source::new(source.to_string()), given)?;
        let revision = p
            .context(ctx)
            .and_then(|c| c.harness(id))
            .map(|h| h.revision())
            .unwrap_or_else(Revision::first);
        self.keep(&p, ctx, AssetKind::Harness, name, id.as_uuid())?;
        Ok(Put { id, revision })
    }

    /// Pins `code` as the code of the Interface named `interface` of the Context `context`, replacing its Pin.
    pub fn pin(&mut self, context: &str, interface: &str, code: &str) -> Result<PinId, AppError> {
        let p = self.project()?;
        let ctx = context_id(&p, context)?;
        let iface = self.asset(ctx, context, AssetKind::Interface, interface)?;
        let id = PinId::from_uuid(self.ids.next_uuid());
        let pin = record(
            &p,
            InterfaceId::from_uuid(iface),
            id,
            Digest::of(code.as_bytes()),
        )?;
        self.store.save_pin(&pin)?;
        Ok(id)
    }

    /// Raises a Crux: a question with its choices, the default work goes on with, and the definitions written on
    /// the default (each citation names its Context).
    pub fn raise_crux(
        &mut self,
        title: &str,
        choices: &[String],
        default: &str,
        items: &[ItemRef],
        clauses: &[ClauseRef],
    ) -> Result<CruxId, AppError> {
        let p = self.project()?;
        let mut targets = Vec::new();
        for r in items {
            targets.push(Target::Item(self.item(&p, None, r)?.0));
        }
        for r in clauses {
            targets.push(Target::Clause(self.clause(&p, None, r)?.0));
        }
        let choices = choices
            .iter()
            .map(|c| Choice::try_from(c.as_str()).map_err(input))
            .collect::<Result<Vec<_>, _>>()?;
        let id = CruxId::from_uuid(self.ids.next_uuid());
        let crux = Crux::new(
            id,
            Title::try_from(title).map_err(input)?,
            choices,
            Choice::try_from(default).map_err(input)?,
            revisions_now(&p, targets),
        )?;
        self.store.save_crux(&crux)?;
        Ok(id)
    }

    /// Takes the person's answer to the Crux titled `title`.
    pub fn answer_crux(&mut self, title: &str, choice: &str) -> Result<(), AppError> {
        let p = self.project()?;
        let mut crux = self
            .store
            .cruxes()?
            .into_iter()
            .find(|c| c.title().as_str() == title.trim())
            .ok_or_else(|| AppError::UnknownCrux(title.to_string()))?;
        answer(&mut crux, Choice::try_from(choice).map_err(input)?, &p)?;
        self.store.save_crux(&crux)?;
        Ok(())
    }

    /// Where the Project stands.
    pub fn status(&self) -> Result<Status, AppError> {
        let p = self.project()?;
        let mut pins = Vec::new();
        let mut seen = BTreeSet::new();
        for c in p.contexts() {
            for i in c.interfaces() {
                seen.insert(i.id());
                let state = match self.store.pin_of(i.id())? {
                    None => PinState::Unpinned,
                    Some(pin) => match staleness(&pin, &p) {
                        Staleness::Fresh => PinState::Fresh,
                        Staleness::InterfaceGone => PinState::Gone,
                        Staleness::Stale(why) => PinState::Stale(
                            why.iter()
                                .map(|r| self.reason(r))
                                .collect::<Result<_, _>>()?,
                        ),
                    },
                };
                pins.push(PinStatus {
                    context: c.name().to_string(),
                    interface: self.name_of(i.id().as_uuid())?,
                    state,
                });
            }
        }
        for pin in self.store.pins()? {
            if !seen.contains(&pin.interface()) {
                pins.push(PinStatus {
                    context: String::new(),
                    interface: self.name_of(pin.interface().as_uuid())?,
                    state: PinState::Gone,
                });
            }
        }
        let mut stale = Vec::new();
        for s in stale_cites(&p) {
            stale.push(StaleCiteStatus {
                case: case_name(&p, s.case),
                target: self.describe(&p, s.target)?,
                cited: s.cited.number(),
                now: s.now.number(),
            });
        }
        let cruxes = self.store.cruxes()?;
        let mut open = Vec::new();
        for u in unapplied(cruxes.iter(), &p) {
            let Some(crux) = cruxes.iter().find(|c| c.id() == u.crux) else {
                continue;
            };
            open.push(UnappliedStatus {
                crux: crux.title().to_string(),
                answer: crux.answer().map(|a| a.to_string()).unwrap_or_default(),
                unchanged: u
                    .unchanged
                    .iter()
                    .map(|t| self.describe(&p, *t))
                    .collect::<Result<_, _>>()?,
                gone: u
                    .gone
                    .iter()
                    .map(|t| self.describe(&p, *t))
                    .collect::<Result<_, _>>()?,
            });
        }
        Ok(Status {
            pins,
            stale_cites: stale,
            unapplied: open,
        })
    }

    fn project(&self) -> Result<Project, AppError> {
        self.store.project()?.ok_or(AppError::NoProject)
    }

    /// Keeps `p` and names the asset `id` that was just made or revised.
    fn keep(
        &mut self,
        p: &Project,
        ctx: ContextId,
        kind: AssetKind,
        name: &str,
        id: uuid::Uuid,
    ) -> Result<(), AppError> {
        self.store.save_project(p)?;
        self.store.name_asset(ctx, kind, name, id)?;
        Ok(())
    }

    /// The id of the asset of `kind` named `name` in the Context `ctx` (named `context`).
    fn asset(
        &self,
        ctx: ContextId,
        context: &str,
        kind: AssetKind,
        name: &str,
    ) -> Result<uuid::Uuid, AppError> {
        self.store
            .asset_id(ctx, kind, name)?
            .ok_or_else(|| AppError::UnknownAsset {
                context: context.to_string(),
                kind: kind.word(),
                name: name.to_string(),
            })
    }

    /// The Context a citation is in: the one it names, else `default`.
    fn cite_context(
        &self,
        p: &Project,
        named: Option<&String>,
        default: Option<ContextId>,
    ) -> Result<(ContextId, String), AppError> {
        match (named, default) {
            (Some(n), _) => Ok((context_id(p, n)?, n.clone())),
            (None, Some(id)) => Ok((
                id,
                p.context(id)
                    .map(|c| c.name().to_string())
                    .unwrap_or_default(),
            )),
            (None, None) => Err(AppError::ContextNotGiven),
        }
    }

    /// The id and current revision of the Item `r` cites.
    fn item(
        &self,
        p: &Project,
        default: Option<ContextId>,
        r: &ItemRef,
    ) -> Result<(ItemId, Revision), AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), default)?;
        let iface = InterfaceId::from_uuid(self.asset(
            ctx,
            &ctx_name,
            AssetKind::Interface,
            &r.interface,
        )?);
        p.context(ctx)
            .and_then(|c| c.interface(iface))
            .and_then(|i| {
                i.items()
                    .iter()
                    .find(|it| it.signature().as_str() == r.signature)
            })
            .map(|it| (it.id(), it.revision()))
            .ok_or_else(|| AppError::UnknownItem {
                interface: r.interface.clone(),
                signature: r.signature.clone(),
            })
    }

    /// The id and current revision of the Clause `r` cites.
    fn clause(
        &self,
        p: &Project,
        default: Option<ContextId>,
        r: &ClauseRef,
    ) -> Result<(ClauseId, Revision), AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), default)?;
        let contract =
            ContractId::from_uuid(self.asset(ctx, &ctx_name, AssetKind::Contract, &r.contract)?);
        let label = ClauseLabel::try_from(r.label.as_str()).map_err(input)?;
        p.context(ctx)
            .and_then(|c| c.contract(contract))
            .and_then(|k| k.clause_by_label(label))
            .map(|k| (k.id(), k.revision()))
            .ok_or_else(|| AppError::UnknownClause {
                contract: r.contract.clone(),
                label: r.label.clone(),
            })
    }

    /// The name of an asset, or its id when it has none.
    fn name_of(&self, id: uuid::Uuid) -> Result<String, AppError> {
        Ok(self.store.asset_name(id)?.unwrap_or_else(|| id.to_string()))
    }

    /// A Pin's reason, in words.
    fn reason(&self, r: &StaleReason) -> Result<String, AppError> {
        Ok(match r {
            StaleReason::Interface => "the interface changed".to_string(),
            StaleReason::Harness(id) => format!("harness `{}`", self.name_of(id.as_uuid())?),
            StaleReason::Contract(id) => format!("contract `{}`", self.name_of(id.as_uuid())?),
            StaleReason::UsedInterface(id) => {
                format!("interface `{}`", self.name_of(id.as_uuid())?)
            }
        })
    }

    /// A citation's target, in words: the asset's name and the signature or label.
    fn describe(&self, p: &Project, target: Target) -> Result<String, AppError> {
        let found = p.contexts().find_map(|c| match target {
            Target::Item(id) => c.interface_of(id).and_then(|i| {
                i.item(id)
                    .map(|it| (i.id().as_uuid(), it.signature().to_string()))
            }),
            Target::Clause(id) => c.contract_of(id).and_then(|k| {
                k.clause(id)
                    .map(|cl| (k.id().as_uuid(), cl.content().label().to_string()))
            }),
        });
        Ok(match found {
            Some((asset, what)) => format!("{} {what}", self.name_of(asset)?),
            None => match target {
                Target::Item(id) => format!("item {id} (gone)"),
                Target::Clause(id) => format!("clause {id} (gone)"),
            },
        })
    }
}

/// The id of the Context named `name`.
fn context_id(p: &Project, name: &str) -> Result<ContextId, AppError> {
    let n = ContextName::try_from(name).map_err(input)?;
    p.context_by_name(&n)
        .map(|c| c.id())
        .ok_or_else(|| AppError::UnknownContext(name.to_string()))
}

/// The ids of the Contexts named in `names`.
fn context_ids(p: &Project, names: &[String]) -> Result<BTreeSet<ContextId>, AppError> {
    names.iter().map(|n| context_id(p, n)).collect()
}

/// The name of the Case with this id.
fn case_name(p: &Project, id: CaseId) -> String {
    p.contexts()
        .flat_map(|c| c.harnesses())
        .find_map(|h| h.case(id))
        .map(|c| c.name().to_string())
        .unwrap_or_else(|| id.to_string())
}
