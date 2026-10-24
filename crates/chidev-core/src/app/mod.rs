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

mod check;
mod judged;
mod model_view;
pub mod ports;

use std::collections::BTreeSet;
use std::fmt::Display;

use thiserror::Error;

pub use self::check::{CheckInfo, Checked, Found, Judgment, Registry, RegistryError, Run};
pub use self::judged::{
    JudgedState, ReportInput, Reported, ReportedFinding, RuleStanding, RuleToJudge, RulesView,
    SubjectInput,
};
pub use self::model_view::{ModelView, VIEW_VERSION};
use self::ports::{AssetKind, AssetName, Changes, IdSource, Invocation, Store, StoreRevision};
pub use crate::domain::check::ParamValue;
use crate::domain::check::{CheckView, Finding, Hook, Mention};
use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestDraft, ChangeRequestError, Crux, Route,
};
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::{Project, ProjectError};
use crate::domain::repository::StoreError;
use crate::domain::service::change_request::{decide, resolve, revisions_now, unapplied};
use crate::domain::service::cite::stale_cites;
use crate::domain::service::pin::{PinError, StaleReason, Staleness, record, staleness};
use crate::domain::vo::case::{CaseCites, CaseName};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
pub use crate::domain::vo::crux::CruxKind;
use crate::domain::vo::crux::{CruxOption, Decision, Reasoned};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
use crate::domain::vo::id::{
    CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
    PinId, ProjectId,
};
pub use crate::domain::vo::intent::Intent;
use crate::domain::vo::intent::Issuer;
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::name::ContextName;
use crate::domain::vo::prose::Prose;
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
    /// No change request has this title.
    #[error("no change request is titled `{0}`")]
    UnknownChangeRequest(String),
    /// A change request has this title already; a change request is addressed by its title.
    #[error("a change request is titled `{0}` already")]
    TitleTaken(String),
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
    /// The change request cannot be opened, decided or resolved.
    #[error(transparent)]
    ChangeRequest(#[from] ChangeRequestError),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// No check has this id.
    #[error("no check is named `{0}`")]
    UnknownRule(String),
    /// The check is not a judged rule, so an agent has nothing to report for it.
    #[error("the rule `{rule}` is {executor}, not judged: only a judged rule takes a report")]
    NotJudged {
        /// The rule.
        rule: String,
        /// Who runs it.
        executor: &'static str,
    },
    /// The policy turned the rule off.
    #[error("the rule `{0}` is off by the policy")]
    RuleOff(String),
    /// A mandatory check found something at `error`; nothing was kept.
    #[error("refused by the mandatory check(s) {}", rules_of(.0))]
    Refused(Vec<Found>),
}

/// The rules of `found`, once each, for a message.
fn rules_of(found: &[Found]) -> String {
    let mut rules: Vec<&str> = found.iter().map(|f| f.rule.as_str()).collect();
    rules.sort_unstable();
    rules.dedup();
    rules.join(", ")
}

fn input(e: impl Display) -> AppError {
    AppError::Input(e.to_string())
}

/// What the checks of a hook read.
fn view<'a>(
    before: Option<&'a Project>,
    after: &'a Project,
    pins: &'a [Pin],
    change_requests: &'a [ChangeRequest],
) -> CheckView<'a> {
    CheckView {
        before,
        after,
        pins,
        change_requests,
    }
}

/// What the checks of a hook gave: the findings for the caller, the records to keep with the change.
struct Gate {
    found: Vec<Found>,
    invocations: Vec<Invocation>,
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

/// An option of a Crux, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionInput {
    /// The choice.
    pub choice: String,
    /// The change it makes to the definitions the Crux is about (the exact new wording); none: it changes none.
    pub change: Option<String>,
    /// What else it affects.
    pub consequence: Option<String>,
}

/// A choice and why (a default, a recommendation, a decision).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasonedInput {
    /// The choice.
    pub choice: String,
    /// Why.
    pub reason: Option<String>,
}

/// The question of a change request a person settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CruxInput {
    /// What kind of question it is.
    pub kind: CruxKind,
    /// The options.
    pub options: Vec<OptionInput>,
    /// The option work goes on with until the person decides.
    pub default: Option<ReasonedInput>,
    /// The option recommended.
    pub recommended: Option<ReasonedInput>,
}

/// A change request as it is opened. Citations name their Context. A change request is addressed by its title,
/// which is unique among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequestInput {
    /// What is asked, in a line.
    pub title: String,
    /// Why it is needed.
    pub situation: Option<String>,
    /// Whether it means to change the behaviour.
    pub intent: Intent,
    /// The title of the broader change request it is a part of.
    pub parent: Option<String>,
    /// Who raises it.
    pub issuer: Option<String>,
    /// The Items it is about.
    pub items: Vec<ItemRef>,
    /// The Clauses it is about.
    pub clauses: Vec<ClauseRef>,
    /// The question for a person; none: the machine resolves it.
    pub crux: Option<CruxInput>,
    /// The findings it deals with.
    pub addresses: Vec<FindingInput>,
}

/// A finding a change request addresses, as a person writes it: the rule, and the Item or the Clause it is about
/// (at most one of them; none: the finding is about no Item or Clause that can be named).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingInput {
    /// The rule (`cite.uncited-clause`).
    pub rule: String,
    /// The Item it is about.
    pub item: Option<ItemRef>,
    /// The Clause it is about.
    pub clause: Option<ClauseRef>,
}

/// What the person decided about a Crux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionInput {
    /// One of the options.
    Chosen(ReasonedInput),
    /// The behaviour in the person's own words.
    Worded {
        /// The behaviour.
        wording: String,
        /// Why.
        reason: Option<String>,
    },
    /// The behaviour left free, as long as these hold.
    Unspecified {
        /// What must still hold.
        keep: Vec<String>,
    },
}

/// Where one change request stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequestStatus {
    /// Its title.
    pub title: String,
    /// `change_behavior` or `keep_behavior`.
    pub intent: &'static str,
    /// The title of the change request it is a part of.
    pub parent: Option<String>,
    /// Who raised it.
    pub issuer: Option<String>,
    /// The kind of its Crux; none: the machine resolves it.
    pub crux: Option<&'static str>,
    /// `open`, `decided` or `resolved`.
    pub state: &'static str,
    /// The decision on its Crux, in words.
    pub decision: Option<String>,
}

/// An unresolved change request the definitions have not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnappliedStatus {
    /// The change request's title.
    pub cr: String,
    /// The decision on its Crux, in words.
    pub decision: Option<String>,
    /// The definitions not changed since, described.
    pub unchanged: Vec<String>,
    /// The definitions gone, described.
    pub gone: Vec<String>,
}

/// Where the Project stands: what is pinned and whether it holds, the citations behind their targets, the change
/// requests, and those not yet applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Every Interface, and every Pin whose Interface is gone.
    pub pins: Vec<PinStatus>,
    /// The citations behind their targets.
    pub stale_cites: Vec<StaleCiteStatus>,
    /// Every change request.
    pub change_requests: Vec<ChangeRequestStatus>,
    /// The change requests not yet applied.
    pub unapplied: Vec<UnappliedStatus>,
    /// What every check finds in the state as it is, each with whether a change request deals with it.
    pub findings: Vec<Found>,
    /// The judged rules never reported or gone stale: what an agent is to judge (`check_rules`).
    pub judged: Vec<RuleStanding>,
}

/// A decision, in words.
fn decision_text(d: &Decision) -> String {
    match d {
        Decision::Chosen(c) => c.choice().to_string(),
        Decision::Worded { wording, .. } => format!("in other words: {wording}"),
        Decision::Unspecified { .. } => "left unspecified".to_string(),
    }
}

fn prose(text: &Option<String>) -> Result<Option<Prose>, AppError> {
    text.as_deref()
        .map(|t| Prose::try_from(t).map_err(input))
        .transpose()
}

fn reasoned(r: &ReasonedInput) -> Result<Reasoned, AppError> {
    Ok(Reasoned::new(
        Choice::try_from(r.choice.as_str()).map_err(input)?,
        prose(&r.reason)?,
    ))
}

/// The use cases over one store and one source of ids.
#[derive(Debug)]
pub struct App<S, G> {
    store: S,
    ids: G,
    registry: Registry,
}

impl<S, G> App<S, G>
where
    S: Store,
    G: IdSource,
{
    /// The use cases over `store`, with fresh ids from `ids`, running the builtin checks at their default levels.
    pub fn new(store: S, ids: G) -> App<S, G> {
        App::with_registry(store, ids, Registry::defaults())
    }

    /// The use cases over `store`, with fresh ids from `ids`, running the checks of `registry`.
    pub fn with_registry(store: S, ids: G, registry: Registry) -> App<S, G> {
        App {
            store,
            ids,
            registry,
        }
    }

    /// The checks and the policy they run under.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The store, given back (to close it).
    pub fn into_store(self) -> S {
        self.store
    }

    /// The store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Makes the Project.
    pub async fn init(&mut self) -> Result<ProjectId, AppError> {
        let at = self.store.revision().await?;
        if self.store.project().await?.is_some() {
            return Err(AppError::AlreadyInitialized);
        }
        let id = ProjectId::from_uuid(self.ids.next_uuid());
        self.commit_project(at, Project::new(id)).await?;
        Ok(id)
    }

    /// Adds a Context named `name` that uses the Contexts named in `uses`.
    pub async fn add_context(
        &mut self,
        name: &str,
        uses: &[String],
    ) -> Result<ContextId, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let uses = context_ids(&p, uses)?;
        let id = ContextId::from_uuid(self.ids.next_uuid());
        let context = crate::domain::entity::context::Context::new(
            id,
            ContextName::try_from(name).map_err(input)?,
            uses,
        )
        .map_err(ProjectError::from)?;
        p.add_context(context)?;
        self.commit_project(at, p).await?;
        Ok(id)
    }

    /// Sets the Contexts the Context named `context` uses.
    pub async fn set_uses(&mut self, context: &str, uses: &[String]) -> Result<(), AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let ctx = context_id(&p, context)?;
        let uses = context_ids(&p, uses)?;
        p.set_uses(ctx, uses)?;
        self.commit_project(at, p).await?;
        Ok(())
    }

    /// Bases the Context named `context` on its Contracts.
    pub async fn base_on_contract(&mut self, context: &str) -> Result<(), AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let ctx = context_id(&p, context)?;
        p.base_on_contract(ctx)?;
        self.commit_project(at, p).await?;
        Ok(())
    }

    /// Makes or revises the Contract named `name` of the Context `context`. A Clause keeps its id when its label
    /// is one the Contract has.
    pub async fn put_contract(
        &mut self,
        context: &str,
        name: &str,
        title: &str,
        clauses: &[ClauseInput],
    ) -> Result<Checked<Put<ContractId>>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Contract, name)
            .await?
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
        let named = AssetName {
            context: ctx,
            kind: AssetKind::Contract,
            name: name.to_string(),
            id: id.as_uuid(),
        };
        let gate = self
            .gate(
                Hook::Put,
                at,
                Some(&before),
                &p,
                std::slice::from_ref(&named),
            )
            .await?;
        self.keep(at, p, named, gate.invocations).await?;
        Ok(Checked {
            value: Put { id, revision },
            findings: gate.found,
        })
    }

    /// Makes or revises the Interface named `name` of the Context `context` from its code and the Items found in
    /// it. An Item keeps its id when its signature (or the one it `replaces`) is one the Interface has.
    pub async fn put_interface(
        &mut self,
        context: &str,
        name: &str,
        source: &str,
        items: &[ItemInput],
    ) -> Result<Checked<Put<InterfaceId>>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Interface, name)
            .await?
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
        let named = AssetName {
            context: ctx,
            kind: AssetKind::Interface,
            name: name.to_string(),
            id: id.as_uuid(),
        };
        let gate = self
            .gate(
                Hook::Put,
                at,
                Some(&before),
                &p,
                std::slice::from_ref(&named),
            )
            .await?;
        self.keep(at, p, named, gate.invocations).await?;
        Ok(Checked {
            value: Put { id, revision },
            findings: gate.found,
        })
    }

    /// Makes or revises the Harness named `name` of the Context `context` from its code and the Cases found in
    /// it. A Case keeps its id when its name is one the Harness has. Each citation records the revision its target
    /// has now: putting a Case says it is written against the definitions as they are.
    pub async fn put_harness(
        &mut self,
        context: &str,
        name: &str,
        source: &str,
        cases: &[CaseInput],
    ) -> Result<Checked<Put<HarnessId>>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Harness, name)
            .await?
            .map(HarnessId::from_uuid);
        let old = id.and_then(|id| p.context(ctx).and_then(|c| c.harness(id)));
        let mut given = Vec::with_capacity(cases.len());
        for c in cases {
            let case_name = CaseName::try_from(c.name.as_str()).map_err(input)?;
            let lines = LineRange::new(c.lines.0, c.lines.1).map_err(input)?;
            let mut items = std::collections::BTreeMap::new();
            for r in &c.items {
                let (item, revision) = self.item(&p, Some(ctx), r).await?;
                items.insert(item, revision);
            }
            let mut clauses = std::collections::BTreeMap::new();
            for r in &c.clauses {
                let (clause, revision) = self.clause(&p, Some(ctx), r).await?;
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
        let named = AssetName {
            context: ctx,
            kind: AssetKind::Harness,
            name: name.to_string(),
            id: id.as_uuid(),
        };
        let gate = self
            .gate(
                Hook::Put,
                at,
                Some(&before),
                &p,
                std::slice::from_ref(&named),
            )
            .await?;
        self.keep(at, p, named, gate.invocations).await?;
        Ok(Checked {
            value: Put { id, revision },
            findings: gate.found,
        })
    }

    /// Pins `code` as the code of the Interface named `interface` of the Context `context`, replacing its Pin.
    pub async fn pin(
        &mut self,
        context: &str,
        interface: &str,
        code: &str,
    ) -> Result<Checked<PinId>, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let ctx = context_id(&p, context)?;
        let iface = self
            .asset(ctx, context, AssetKind::Interface, interface)
            .await?;
        let id = PinId::from_uuid(self.ids.next_uuid());
        let pin = record(
            &p,
            InterfaceId::from_uuid(iface),
            id,
            Digest::of(code.as_bytes()),
        )?;
        let mut pins = self.store.pins().await?;
        pins.retain(|k| k.interface() != pin.interface());
        pins.push(pin.clone());
        let crs = self.store.change_requests().await?;
        let gate = self
            .gate_with(Hook::Pin, at, &view(Some(&p), &p, &pins, &crs), &[])
            .await?;
        let changes = Changes {
            pins: vec![pin],
            invocations: gate.invocations,
            ..Changes::default()
        };
        self.store.commit(at, changes).await?;
        Ok(Checked {
            value: id,
            findings: gate.found,
        })
    }

    /// Opens a change request: what is asked and why, whether it means to change the behaviour, the change request
    /// it is a part of, who raises it, the definitions it is about (each citation names its Context), and the
    /// question for a person when the machine cannot resolve it.
    pub async fn open_change_request(
        &mut self,
        c: &ChangeRequestInput,
    ) -> Result<ChangeRequestId, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let title = Title::try_from(c.title.as_str()).map_err(input)?;
        let existing = self.store.change_requests().await?;
        if existing.iter().any(|cr| *cr.title() == title) {
            return Err(AppError::TitleTaken(title.to_string()));
        }
        let parent = match &c.parent {
            None => None,
            Some(t) => Some(
                existing
                    .iter()
                    .find(|cr| cr.title().as_str() == t.trim())
                    .map(|cr| cr.id())
                    .ok_or_else(|| AppError::UnknownChangeRequest(t.clone()))?,
            ),
        };
        let mut targets = Vec::new();
        for r in &c.items {
            targets.push(Target::Item(self.item(&p, None, r).await?.0));
        }
        for r in &c.clauses {
            targets.push(Target::Clause(self.clause(&p, None, r).await?.0));
        }
        let route = match &c.crux {
            None => Route::Machine,
            Some(q) => {
                let mut options = Vec::with_capacity(q.options.len());
                for o in &q.options {
                    options.push(CruxOption::new(
                        Choice::try_from(o.choice.as_str()).map_err(input)?,
                        prose(&o.change)?,
                        prose(&o.consequence)?,
                    ));
                }
                Route::Person(Crux::new(
                    q.kind,
                    options,
                    q.default.as_ref().map(reasoned).transpose()?,
                    q.recommended.as_ref().map(reasoned).transpose()?,
                )?)
            }
        };
        let issuer = c
            .issuer
            .as_deref()
            .map(|i| Issuer::try_from(i).map_err(input))
            .transpose()?;
        let mut addresses = Vec::with_capacity(c.addresses.len());
        for a in &c.addresses {
            addresses.push(self.finding_ref(&p, a).await?);
        }
        let id = ChangeRequestId::from_uuid(self.ids.next_uuid());
        let cr = ChangeRequest::open(
            id,
            ChangeRequestDraft {
                title,
                situation: prose(&c.situation)?,
                intent: c.intent,
                parent,
                issuer,
                lines: revisions_now(&p, targets),
                route,
                addresses,
            },
        );
        self.commit_change_request(at, cr, Vec::new()).await?;
        Ok(id)
    }

    /// Takes the person's decision on the Crux of the change request titled `title`.
    pub async fn decide_change_request(
        &mut self,
        title: &str,
        decision: &DecisionInput,
    ) -> Result<(), AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let mut cr = self.change_request_titled(title).await?;
        let decision = match decision {
            DecisionInput::Chosen(r) => Decision::Chosen(reasoned(r)?),
            DecisionInput::Worded { wording, reason } => Decision::Worded {
                wording: Prose::try_from(wording.as_str()).map_err(input)?,
                reason: prose(reason)?,
            },
            DecisionInput::Unspecified { keep } => Decision::Unspecified {
                keep: keep
                    .iter()
                    .map(|k| Prose::try_from(k.as_str()).map_err(input))
                    .collect::<Result<_, _>>()?,
            },
        };
        decide(&mut cr, decision, &p)?;
        self.commit_change_request(at, cr, Vec::new()).await?;
        Ok(())
    }

    /// Closes the record of the change request titled `title`, once the change is applied.
    pub async fn resolve_change_request(&mut self, title: &str) -> Result<Checked<()>, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let mut cr = self.change_request_titled(title).await?;
        resolve(&mut cr, &p)?;
        let pins = self.store.pins().await?;
        let mut crs = self.store.change_requests().await?;
        crs.retain(|k| k.id() != cr.id());
        crs.push(cr.clone());
        let gate = self
            .gate_with(Hook::Resolve, at, &view(Some(&p), &p, &pins, &crs), &[])
            .await?;
        self.commit_change_request(at, cr, gate.invocations).await?;
        Ok(Checked {
            value: (),
            findings: gate.found,
        })
    }

    /// Runs every check on the state as it is: what the person would see if each check ran now, with what agents
    /// reported for the judged rules. Nothing is kept, and the runs are not recorded.
    pub async fn check(&self) -> Result<Vec<Found>, AppError> {
        Ok(self.checked().await?.0)
    }

    /// The findings of [`App::check`], and the judged rules not up to date.
    async fn checked(&self) -> Result<(Vec<Found>, Vec<RuleStanding>), AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let pins = self.store.pins().await?;
        let crs = self.store.change_requests().await?;
        let run = self.registry.run(
            Hook::Check,
            at,
            &CheckView {
                before: None,
                after: &p,
                pins: &pins,
                change_requests: &crs,
            },
        );
        let mut found = self.found(&run.findings, &p, &crs, &[]).await?;
        let (judged, standing) = self.judged_standing(&p, &crs, at).await?;
        found.extend(judged);
        Ok((found, standing))
    }

    /// The change request titled `title`.
    async fn change_request_titled(&self, title: &str) -> Result<ChangeRequest, AppError> {
        self.store
            .change_requests()
            .await?
            .into_iter()
            .find(|c| c.title().as_str() == title.trim())
            .ok_or_else(|| AppError::UnknownChangeRequest(title.to_string()))
    }

    /// Where the Project stands.
    pub async fn status(&self) -> Result<Status, AppError> {
        let p = self.project().await?;
        let mut pins = Vec::new();
        let mut seen = BTreeSet::new();
        for c in p.contexts() {
            for i in c.interfaces() {
                seen.insert(i.id());
                let state = match self.store.pin_of(i.id()).await? {
                    None => PinState::Unpinned,
                    Some(pin) => match staleness(&pin, &p) {
                        Staleness::Fresh => PinState::Fresh,
                        Staleness::InterfaceGone => PinState::Gone,
                        Staleness::Stale(why) => {
                            let mut said = Vec::with_capacity(why.len());
                            for r in &why {
                                said.push(self.reason(r).await?);
                            }
                            PinState::Stale(said)
                        }
                    },
                };
                pins.push(PinStatus {
                    context: c.name().to_string(),
                    interface: self.name_of(i.id().as_uuid()).await?,
                    state,
                });
            }
        }
        for pin in self.store.pins().await? {
            if !seen.contains(&pin.interface()) {
                pins.push(PinStatus {
                    context: String::new(),
                    interface: self.name_of(pin.interface().as_uuid()).await?,
                    state: PinState::Gone,
                });
            }
        }
        let mut stale = Vec::new();
        for s in stale_cites(&p) {
            stale.push(StaleCiteStatus {
                case: case_name(&p, s.case),
                target: self.describe(&p, s.target).await?,
                cited: s.cited.number(),
                now: s.now.number(),
            });
        }
        let crs = self.store.change_requests().await?;
        let (findings, judged) = self.checked().await?;
        let title_of = |id: ChangeRequestId| {
            crs.iter()
                .find(|c| c.id() == id)
                .map(|c| c.title().to_string())
                .unwrap_or_else(|| id.to_string())
        };
        let decision_of =
            |c: &ChangeRequest| c.crux().and_then(|q| q.decision()).map(decision_text);
        let records = crs
            .iter()
            .map(|c| ChangeRequestStatus {
                title: c.title().to_string(),
                intent: c.intent().word(),
                parent: c.parent().map(title_of),
                issuer: c.issuer().map(|i| i.to_string()),
                crux: c.crux().map(|q| q.kind().word()),
                state: c.state().word(),
                decision: decision_of(c),
            })
            .collect();
        let mut open = Vec::new();
        for u in unapplied(crs.iter(), &p) {
            let Some(cr) = crs.iter().find(|c| c.id() == u.cr) else {
                continue;
            };
            open.push(UnappliedStatus {
                cr: cr.title().to_string(),
                decision: decision_of(cr),
                unchanged: self.describe_all(&p, &u.unchanged).await?,
                gone: self.describe_all(&p, &u.gone).await?,
            });
        }
        Ok(Status {
            pins,
            stale_cites: stale,
            change_requests: records,
            unapplied: open,
            findings,
            judged,
        })
    }

    async fn project(&self) -> Result<Project, AppError> {
        self.store.project().await?.ok_or(AppError::NoProject)
    }

    /// Commits `p` alone, as the store stood at `at` when the use case began to read.
    async fn commit_project(&self, at: StoreRevision, p: Project) -> Result<(), AppError> {
        let changes = Changes {
            project: Some(p),
            ..Changes::default()
        };
        Ok(self.store.commit(at, changes).await?)
    }

    /// Commits `cr` alone, as the store stood at `at` when the use case began to read.
    async fn commit_change_request(
        &self,
        at: StoreRevision,
        cr: ChangeRequest,
        invocations: Vec<Invocation>,
    ) -> Result<(), AppError> {
        let changes = Changes {
            change_requests: vec![cr],
            invocations,
            ..Changes::default()
        };
        Ok(self.store.commit(at, changes).await?)
    }

    /// Commits `p` and the name of the asset that was just made or revised, together: a Project holding an asset
    /// its name does not reach (or the reverse) is never kept. The `invocations` of the checks that ran go with
    /// them.
    async fn keep(
        &self,
        at: StoreRevision,
        p: Project,
        name: AssetName,
        invocations: Vec<Invocation>,
    ) -> Result<(), AppError> {
        let changes = Changes {
            project: Some(p),
            names: vec![name],
            invocations,
            ..Changes::default()
        };
        Ok(self.store.commit(at, changes).await?)
    }

    /// Runs the checks of `hook` on `after`, the Project as the use case would keep it, with the Pins and the
    /// change requests as the store has them. `pending` are the names the use case is about to give, which the
    /// store does not have yet.
    async fn gate(
        &self,
        hook: Hook,
        at: StoreRevision,
        before: Option<&Project>,
        after: &Project,
        pending: &[AssetName],
    ) -> Result<Gate, AppError> {
        let pins = self.store.pins().await?;
        let crs = self.store.change_requests().await?;
        self.gate_with(hook, at, &view(before, after, &pins, &crs), pending)
            .await
    }

    /// Runs the checks of `hook` on `after`, with the Pins and change requests the use case would leave. A finding
    /// of a mandatory check at `error` refuses the use case before anything is kept.
    async fn gate_with(
        &self,
        hook: Hook,
        at: StoreRevision,
        view: &CheckView<'_>,
        pending: &[AssetName],
    ) -> Result<Gate, AppError> {
        let run = self.registry.run(hook, at, view);
        let (after, crs) = (view.after, view.change_requests);
        if !run.refusing.is_empty() {
            return Err(AppError::Refused(
                self.found(&run.refusing, after, crs, pending).await?,
            ));
        }
        Ok(Gate {
            found: self.found(&run.findings, after, crs, pending).await?,
            invocations: run.invocations,
        })
    }

    /// The findings as a caller sees them: their state among `crs`, and what they are about in words.
    async fn found(
        &self,
        findings: &[Finding],
        p: &Project,
        crs: &[ChangeRequest],
        pending: &[AssetName],
    ) -> Result<Vec<Found>, AppError> {
        let mut out = Vec::with_capacity(findings.len());
        for f in findings {
            let about = match f.subject() {
                None => None,
                Some(s) => Some(self.describe_subject(p, crs, s, pending).await?),
            };
            out.push(Found {
                rule: f.rule().clone(),
                level: f.level(),
                state: f.state_in(crs),
                message: self.worded(f, pending).await?,
                about,
                subject: f.subject(),
                stale: false,
                judgment: None,
            });
        }
        Ok(out)
    }

    /// The message of a finding, with the names of the things it mentions written after it
    /// (`the code was pinned against what has moved: contract `rounding` changed`).
    async fn worded(&self, f: &Finding, pending: &[AssetName]) -> Result<String, AppError> {
        if f.mentions().is_empty() {
            return Ok(f.message().to_string());
        }
        let mut said = Vec::with_capacity(f.mentions().len());
        for m in f.mentions() {
            let (kind, id) = match m {
                Mention::Interface(id) => ("interface", id.as_uuid()),
                Mention::Harness(id) => ("harness", id.as_uuid()),
                Mention::Contract(id) => ("contract", id.as_uuid()),
            };
            said.push(format!(
                "{kind} `{}` changed",
                self.name_in(id, pending).await?
            ));
        }
        Ok(format!("{}: {}", f.message(), said.join(", ")))
    }

    /// What a finding is about, in words.
    async fn describe_subject(
        &self,
        p: &Project,
        crs: &[ChangeRequest],
        subject: Subject,
        pending: &[AssetName],
    ) -> Result<String, AppError> {
        Ok(match subject {
            Subject::Context(id) => p
                .context(id)
                .map(|c| format!("context {}", c.name()))
                .unwrap_or_else(|| format!("context {id}")),
            Subject::Interface(id) => {
                format!("interface {}", self.name_in(id.as_uuid(), pending).await?)
            }
            Subject::Item(id) => self.describe_in(p, Target::Item(id), pending).await?,
            Subject::Clause(id) => self.describe_in(p, Target::Clause(id), pending).await?,
            Subject::Case(id) => format!("case {}", case_name(p, id)),
            Subject::ChangeRequest(id) => crs
                .iter()
                .find(|c| c.id() == id)
                .map(|c| format!("change request `{}`", c.title()))
                .unwrap_or_else(|| format!("change request {id}")),
        })
    }

    /// The finding a person names (`input`) as the thing a change request addresses.
    async fn finding_ref(&self, p: &Project, named: &FindingInput) -> Result<FindingRef, AppError> {
        let rule = RuleId::try_from(named.rule.as_str()).map_err(input)?;
        let subject = match (&named.item, &named.clause) {
            (None, None) => None,
            (Some(r), None) => Some(Subject::Item(self.item(p, None, r).await?.0)),
            (None, Some(r)) => Some(Subject::Clause(self.clause(p, None, r).await?.0)),
            (Some(_), Some(_)) => {
                return Err(AppError::Input(
                    "a finding is about an item or a clause, not both".to_string(),
                ));
            }
        };
        Ok(FindingRef::new(rule, subject))
    }

    /// The id of the asset of `kind` named `name` in the Context `ctx` (named `context`).
    async fn asset(
        &self,
        ctx: ContextId,
        context: &str,
        kind: AssetKind,
        name: &str,
    ) -> Result<uuid::Uuid, AppError> {
        self.store
            .asset_id(ctx, kind, name)
            .await?
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
    async fn item(
        &self,
        p: &Project,
        default: Option<ContextId>,
        r: &ItemRef,
    ) -> Result<(ItemId, Revision), AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), default)?;
        let iface = InterfaceId::from_uuid(
            self.asset(ctx, &ctx_name, AssetKind::Interface, &r.interface)
                .await?,
        );
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
    async fn clause(
        &self,
        p: &Project,
        default: Option<ContextId>,
        r: &ClauseRef,
    ) -> Result<(ClauseId, Revision), AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), default)?;
        let contract = ContractId::from_uuid(
            self.asset(ctx, &ctx_name, AssetKind::Contract, &r.contract)
                .await?,
        );
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
    async fn name_of(&self, id: uuid::Uuid) -> Result<String, AppError> {
        self.name_in(id, &[]).await
    }

    /// The name of an asset, a `pending` one (given by the use case, not kept yet) first.
    async fn name_in(&self, id: uuid::Uuid, pending: &[AssetName]) -> Result<String, AppError> {
        if let Some(n) = pending.iter().find(|n| n.id == id) {
            return Ok(n.name.clone());
        }
        Ok(self
            .store
            .asset_name(id)
            .await?
            .unwrap_or_else(|| id.to_string()))
    }

    /// A Pin's reason, in words.
    async fn reason(&self, r: &StaleReason) -> Result<String, AppError> {
        Ok(match r {
            StaleReason::Interface => "the interface changed".to_string(),
            StaleReason::Harness(id) => format!("harness `{}`", self.name_of(id.as_uuid()).await?),
            StaleReason::Contract(id) => {
                format!("contract `{}`", self.name_of(id.as_uuid()).await?)
            }
            StaleReason::UsedInterface(id) => {
                format!("interface `{}`", self.name_of(id.as_uuid()).await?)
            }
        })
    }

    /// Several targets, in words, in order.
    async fn describe_all(&self, p: &Project, targets: &[Target]) -> Result<Vec<String>, AppError> {
        let mut said = Vec::with_capacity(targets.len());
        for t in targets {
            said.push(self.describe(p, *t).await?);
        }
        Ok(said)
    }

    /// A citation's target, in words: the asset's name and the signature or label.
    async fn describe(&self, p: &Project, target: Target) -> Result<String, AppError> {
        self.describe_in(p, target, &[]).await
    }

    /// A citation's target, in words, with the names a use case is about to give.
    async fn describe_in(
        &self,
        p: &Project,
        target: Target,
        pending: &[AssetName],
    ) -> Result<String, AppError> {
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
            Some((asset, what)) => format!("{} {what}", self.name_in(asset, pending).await?),
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
