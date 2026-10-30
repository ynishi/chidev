//! The change set use case: several assets put and retired as one transition of the Project.
//!
//! A put or a retirement of one asset is a change set of one, so there is a single path that turns names into ids,
//! lets the domain apply the whole and check its rules once, runs the checks of an edit once on the Project before and
//! after (so that it reports what the change set introduced), and commits once, with the names of the assets it
//! made.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::commit::{EditTrigger, ModelDelta};
use super::ports::external::ExternalExecutor;
use super::ports::items::{FoundItem, ItemsError};
use super::ports::{AssetKind, IdSource, NamedAsset, Store};
use super::{
    App, AppError, CaseInput, Checked, ClauseInput, ClauseRef, ItemInput, Names, Put, RenameInput,
    RetiredStatus, Retirement, context_id, context_ids, context_of, input, refuse_retired,
};
use crate::domain::entity::change_request::ChangeRequestError;
use crate::domain::entity::context::Context;
use crate::domain::entity::contract::Contract;
use crate::domain::entity::harness::Harness;
use crate::domain::entity::project::{Project, ProjectError};
use crate::domain::service::change_request::frame;
use crate::domain::service::footprint::footprint_between;
use crate::domain::violation::{Party, Violation};
use crate::domain::vo::case::CaseName;
use crate::domain::vo::change_set::{
    AssetPut, AssetRetire, CaseDraft, ChangeSet, ExampleDraft, HarnessDraft,
};
use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::clause_edit::{ClauseEdit, ClauseNameProblem};
use crate::domain::vo::example::Example;
use crate::domain::vo::harness_kind::CodeKind;
use crate::domain::vo::id::{
    CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
};
use crate::domain::vo::item_key::ItemKey;
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::name::{AssetName, ContextName};
use crate::domain::vo::revision::Revision;
use crate::domain::vo::source::Source;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// A Contract to make or revise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractPut {
    /// The Context's name.
    pub context: String,
    /// The Contract's name.
    pub name: AssetName,
    /// Its title.
    pub title: String,
    /// All of its Clauses, in order; one keeps its id when its label (or the one it `replaces`) is one the Contract
    /// has.
    pub clauses: Vec<ClauseInput>,
}

/// Some Clauses of a Contract that exists to change: the others stay as they are, in their order, and so does the
/// title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClausesPut {
    /// The Context's name.
    pub context: String,
    /// The Contract's name.
    pub contract: AssetName,
    /// The Clauses to set. One whose label the Contract has gets the sentence given; one that `replaces` a label
    /// the Contract has takes the Clause's place with the new label and the sentence; any other is new, and is
    /// appended after the Clauses the Contract has, in the order given.
    pub clauses: Vec<ClauseInput>,
    /// The labels of the Clauses to remove.
    pub remove: Vec<String>,
}

/// An Interface to make or revise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfacePut {
    /// The Context's name.
    pub context: String,
    /// The Interface's name.
    pub name: AssetName,
    /// Its code.
    pub source: String,
    /// Its Items, written or found.
    pub items: ItemsInput,
}

/// Where the Items of an Interface put come from. One keeps its id when its key (or the one it replaces) is one the
/// Interface has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemsInput {
    /// They are all of them, and no provider is asked.
    Written(Vec<ItemInput>),
    /// The provider of the language the Interface is laid out with finds them in `source`, before the set is planned
    /// and before the store is read, so a provider that fails leaves everything as it was.
    Found {
        /// The Item that had the key `from` is the one found under `to`, and keeps its id (what `replaces` is for
        /// Items that are written). Only the keys named are renamed.
        renames: Vec<RenameInput>,
    },
}

/// A Harness to make or revise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessPut {
    /// The Context's name.
    pub context: String,
    /// The Harness's name.
    pub name: AssetName,
    /// The Interfaces of its Context, by name, whose pinned code a sim Harness, or a Harness of examples, builds
    /// into the program it runs (one that the same change set makes is fine). It is not a citation: it asks no
    /// Item of them and puts the Harness on no Pin's basis. A Harness of tests builds none. A put states all of
    /// it, as it states the code.
    pub builds: Vec<AssetName>,
    /// What it holds: code and the Cases in it, or examples. The kind is inside it, so a put cannot say examples
    /// and give code.
    pub content: HarnessPutContent,
}

/// What a put of a Harness says it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessPutContent {
    /// Code, and the Cases found in it.
    Code {
        /// Its kind; none: the kind it has, or tests for a new one. A Harness that holds examples must be told.
        kind: Option<CodeKind>,
        /// Its code.
        source: String,
        /// Its Cases. What they cite is named as in a put of the Harness alone, and may be a Clause or an Item
        /// that the same change set makes. A Case of tests cites an Item at least; a Case of a sim cites Clauses
        /// only.
        cases: Vec<CaseInput>,
    },
    /// Examples, one for each Case.
    Examples(Vec<ExampleInput>),
}

/// An example Case of a Harness of examples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleInput {
    /// Its name; an existing Case of the same name keeps its id.
    pub name: String,
    /// The Clauses it shows (one at least). An example cites no Item.
    pub clauses: Vec<ClauseRef>,
    /// The example.
    pub example: ExampleTexts,
}

/// The texts of an example, as an operation words them. This is the application's own type; the domain has the same
/// four texts in its own words ([`Example`]), and each is made from the other once, at its edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleTexts {
    /// What the program is started with: words split on white space.
    pub args: String,
    /// What the program is given to read.
    pub stdin: String,
    /// What the program is expected to give, ending with a line `#exit <code>`.
    pub expected: String,
    /// What the program is expected to write as errors; none: not compared.
    pub stderr: Option<String>,
}

/// One thing a change set puts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutInput {
    /// A Context is added. The other puts of the set can name it, and it may use the Contexts the set adds.
    Context {
        /// Its name.
        name: String,
        /// The Contexts it uses, by name.
        uses: Vec<String>,
    },
    /// The Contexts a Context uses, by name.
    Uses {
        /// The Context.
        context: String,
        /// The Contexts it uses.
        uses: Vec<String>,
    },
    /// A Context is based on its Contracts (after the Contracts of the change set are in).
    BaseOnContract {
        /// The Context.
        context: String,
    },
    /// A Contract.
    Contract(ContractPut),
    /// Some Clauses of a Contract that exists.
    Clauses(ClausesPut),
    /// An Interface.
    Interface(InterfacePut),
    /// A Harness.
    Harness(HarnessPut),
}

/// One thing a change set retires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetireInput {
    /// A Context, by name.
    Context(String),
    /// A Contract of a Context.
    Contract {
        /// The Context.
        context: String,
        /// The Contract.
        contract: AssetName,
    },
    /// An Interface of a Context.
    Interface {
        /// The Context.
        context: String,
        /// The Interface.
        interface: AssetName,
    },
    /// A Harness of a Context.
    Harness {
        /// The Context.
        context: String,
        /// The Harness.
        harness: AssetName,
    },
}

/// A change set as a person writes it: the assets to put, and the things to retire, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSetInput {
    /// The title of the change request this change set is applied for. The request must exist and not be resolved.
    /// What the set moves is recorded as work of that request, in the same commit: it is what `status` reads to say
    /// whether the request's targets were revised or retired, and it is judged against the request's own frame. The
    /// frame also bounds which citations of the Cases of a Harness put in the set are brought up to the revision of
    /// what they cite: those the frame holds, and new ones; a stale citation outside it stays as it was.
    /// None: the set belongs to no request, shows in the targets of none, is judged by revision for the `keep`
    /// of any request alone, and brings every citation of a Harness it puts up.
    pub change_request: Option<String>,
    /// What is put.
    pub puts: Vec<PutInput>,
    /// What is retired.
    pub retires: Vec<RetireInput>,
}

/// The kind of setting of a Context that a change set put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// The other Contexts it uses.
    Uses,
    /// The Contract it is based on.
    BaseOnContract,
}

impl SettingKind {
    /// The word for the setting: `uses` or `base_on_contract`.
    pub fn word(&self) -> &'static str {
        match self {
            SettingKind::Uses => "uses",
            SettingKind::BaseOnContract => "base_on_contract",
        }
    }
}

/// The kind of asset that a change set put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetPutKind {
    /// A Contract put whole.
    Contract,
    /// Clauses put into a Contract.
    Clauses,
    /// An Interface.
    Interface,
    /// A Harness.
    Harness,
}

impl AssetPutKind {
    /// The word for the asset put kind: `contract`, `clauses`, `interface` or `harness`.
    pub fn word(&self) -> &'static str {
        match self {
            AssetPutKind::Contract => "contract",
            AssetPutKind::Clauses => "clauses",
            AssetPutKind::Interface => "interface",
            AssetPutKind::Harness => "harness",
        }
    }
}

/// What one put of a change set came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutDone {
    /// A Context was added.
    Context {
        /// The Context's name.
        name: String,
        /// The Context's id.
        id: Uuid,
        /// The Context's revision after the change.
        revision: Revision,
    },
    /// A setting of a Context was changed.
    Setting {
        /// Which setting.
        kind: SettingKind,
        /// The Context's name.
        context: String,
    },
    /// An asset was put.
    Asset {
        /// Which asset kind.
        kind: AssetPutKind,
        /// The Context's name.
        context: String,
        /// The asset's name.
        name: String,
        /// The asset's id.
        id: Uuid,
        /// The asset's revision after the change.
        revision: Revision,
    },
}

impl PutDone {
    /// The kind word of the put (`context`, `uses`, `base_on_contract`, `contract`, `clauses`, `interface` or `harness`).
    pub fn kind(&self) -> &'static str {
        match self {
            PutDone::Context { .. } => "context",
            PutDone::Setting { kind, .. } => kind.word(),
            PutDone::Asset { kind, .. } => kind.word(),
        }
    }

    /// The name of the Context the put is about.
    pub fn context(&self) -> &str {
        match self {
            PutDone::Context { name, .. } => name.as_str(),
            PutDone::Setting { context, .. } => context.as_str(),
            PutDone::Asset { context, .. } => context.as_str(),
        }
    }
}

/// What a change set did: each put, in the order listed, and everything that left the model (what the puts dropped
/// and what was retired, in the order recorded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSetDone {
    /// The puts, in the order given.
    pub puts: Vec<PutDone>,
    /// What went.
    pub retired: Vec<RetiredStatus>,
}

/// What the use case worked out of the names: the ids, and the words for the things it made.
#[derive(Default)]
struct Plan {
    /// The id of each asset put, by Context, kind and name.
    ids: BTreeMap<(ContextId, AssetKind, AssetName), Uuid>,
    /// The names to keep with the Project, one for each asset put.
    names: Vec<NamedAsset>,
    /// The Items of each Interface put: key and id.
    items: BTreeMap<InterfaceId, Vec<(String, ItemId)>>,
    /// The Clauses of each Contract put: label and id.
    clauses: BTreeMap<ContractId, Vec<(ClauseLabel, ClauseId)>>,
    /// How to say the Items, Clauses and Cases that the change set makes, which the stored Project does not have.
    words: BTreeMap<Target, String>,
}

impl Plan {
    /// Records that the asset under `key` is `id` in this change set, with the name to keep for it; gives `id`.
    fn name(&mut self, key: (ContextId, AssetKind, AssetName), id: Uuid) -> Uuid {
        let (context, kind, name) = key.clone();
        self.ids.insert(key, id);
        self.names.push(NamedAsset {
            context,
            kind,
            name,
            id,
        });
        id
    }
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Applies a change set: all the puts and retirements together, or nothing.
    ///
    /// The Items of an Interface put with none are found first, by the provider of its language, and before the
    /// store is read: the input is settled (and a provider that fails or disagrees refuses the set) before anything
    /// that depends on the store's revision, and what is applied then is a function of the input and the store.
    ///
    /// Names are turned into ids next (a Case may cite a Clause or an Item that the same change set makes), then
    /// the domain applies the whole to a copy of the Project and checks the rules of the Project once, on the state
    /// it ends in; the checks of an edit run once on the Project before and after; and one commit keeps the Project and
    /// the names of the assets made. When it is refused nothing is kept, and the error names every problem found
    /// (a Case that cites what it cannot reach is named with its Harness, and what it cites with the Contract or
    /// Interface that has it).
    ///
    /// A set applied `for` a change request records what it moved (the targets whose revision differs between the
    /// Project before and after, with the containers they were in) as work of that request, in the same commit; the
    /// request must exist and not be resolved. The checks of an edit see the request with that work, so a move
    /// outside its frame is found by the set that makes it.
    ///
    /// The set also carries the frame of that request into the Project: a Harness it puts brings up a citation of a
    /// Case (records it at the revision its target has) only where the frame holds it, or the citation is new, so a
    /// stale Case of another part stays stale and does not move. A set for no request brings every citation up.
    pub async fn apply_change_set(
        &mut self,
        set: &ChangeSetInput,
    ) -> Result<Checked<ChangeSetDone>, AppError> {
        self.apply_set(EditTrigger::ApplyChangeSet, set).await
    }

    /// [`App::apply_change_set`], for the use case `trigger`: a put or a retirement of one thing is a change set of
    /// one, and every use case that edits the model by name goes through here.
    async fn apply_set(
        &mut self,
        trigger: EditTrigger,
        set: &ChangeSetInput,
    ) -> Result<Checked<ChangeSetDone>, AppError> {
        if set.puts.is_empty() && set.retires.is_empty() {
            return Err(input("the change set has nothing to put or retire"));
        }
        let found = self.found_items(set).await?;
        let at = self.store.revision().await?;
        let mut applied_for = match &set.change_request {
            Some(title) => {
                let cr = self.change_request_titled(title).await?;
                if cr.is_resolved() {
                    return Err(AppError::on(&cr, ChangeRequestError::AlreadyResolved));
                }
                Some(cr)
            }
            None => None,
        };
        let p = self.project().await?;
        let mut plan = Plan::default();
        let mut problems = Vec::new();
        let mut puts = Vec::new();

        // The Contexts the set adds first: the other puts name them, so they are resolved against `named`, the
        // Project with them in (`p` stays the Project before, for what the set moved).
        let mut named = p.clone();
        let mut fresh = Vec::new();
        for put in &set.puts {
            if let PutInput::Context { name, .. } = put {
                fresh.push((name.as_str(), ContextId::from_uuid(self.ids.next_uuid())));
            }
        }
        let mut waiting = Vec::new();
        for put in &set.puts {
            let PutInput::Context { name, uses } = put else {
                continue;
            };
            let id = fresh.iter().find(|(n, _)| n == name).map(|(_, id)| *id);
            let built = ContextName::try_from(name.as_str())
                .map_err(AppError::from)
                .and_then(|n| {
                    let uses = uses
                        .iter()
                        .map(|u| match fresh.iter().find(|(n, _)| n == u) {
                            Some((_, id)) => Ok(*id),
                            None => context_id(&p, u),
                        })
                        .collect::<Result<BTreeSet<_>, _>>()?;
                    let id = id.ok_or_else(|| input("the Context has no id"))?;
                    Context::new(id, n, uses).map_err(|e| AppError::from(ProjectError::from(e)))
                });
            match built {
                Ok(c) => waiting.push(c),
                Err(e) => problems.push(e),
            }
        }
        // A Context may use another one the set adds, whatever order they are listed in.
        loop {
            let before = waiting.len();
            let mut left = Vec::new();
            for c in waiting {
                if named.add_context(c.clone()).is_ok() {
                    puts.push(AssetPut::Context(c));
                } else {
                    left.push(c);
                }
            }
            waiting = left;
            if waiting.is_empty() || waiting.len() == before {
                break;
            }
        }
        for c in waiting {
            if let Err(e) = named.add_context(c) {
                problems.push(e.into());
            }
        }
        let p_before = p;
        let p = named;

        // The definitions first: a Case that cites one finds the ids and labels of the change set's own. The
        // Contracts and Interfaces the set makes come before the `clauses` entries: a `clauses` entry is put onto a
        // Contract that is, so the ones the set makes are named first, whatever order the entries are listed in.
        for (n, put) in set.puts.iter().enumerate() {
            match put {
                PutInput::Contract(c) => {
                    let done = self.contract_put(&p, &mut plan, c).await;
                    keep_or_note(done, &mut puts, &mut problems);
                }
                PutInput::Interface(i) => {
                    let found = found.get(&n).map(Vec::as_slice);
                    let done = self.interface_put(&p, &mut plan, i, found).await;
                    keep_or_note(done, &mut puts, &mut problems);
                }
                PutInput::Context { .. }
                | PutInput::Clauses(_)
                | PutInput::Uses { .. }
                | PutInput::BaseOnContract { .. }
                | PutInput::Harness(_) => {}
            }
        }
        for put in &set.puts {
            if let PutInput::Clauses(c) = put
                && let Some(done) = self.clauses_put(&p, &mut plan, c, &mut problems).await
            {
                puts.push(done);
            }
        }
        for put in &set.puts {
            match put {
                PutInput::Harness(h) => {
                    if let Some(done) = self.harness_put(&p, &mut plan, h, &mut problems).await {
                        puts.push(done);
                    }
                }
                PutInput::Uses { context, uses } => {
                    match context_id(&p, context).and_then(|c| Ok((c, context_ids(&p, uses)?))) {
                        Ok((context, uses)) => puts.push(AssetPut::Uses { context, uses }),
                        Err(e) => problems.push(e),
                    }
                }
                PutInput::BaseOnContract { context } => match context_id(&p, context) {
                    Ok(context) => puts.push(AssetPut::BaseOnContract { context }),
                    Err(e) => problems.push(e),
                },
                PutInput::Context { .. }
                | PutInput::Contract(_)
                | PutInput::Clauses(_)
                | PutInput::Interface(_) => {}
            }
        }
        let mut retires = Vec::new();
        for retire in &set.retires {
            match self.retire_target(&p, retire).await {
                Ok(r) => retires.push(r),
                Err(e) => problems.push(e),
            }
        }
        if !problems.is_empty() {
            // The domain would refuse a thing named twice only when it applies the set, which it does not get to:
            // so the repeats of what was planned are said with the problems found. An entry that could not be
            // planned is not among them: a repeat with it is said once it is right.
            let planned = ChangeSet::new(puts, retires);
            for e in Project::repeated(&planned) {
                problems.push(self.named_error(&p, &plan, e).await?);
            }
            return Err(one_or_many(problems));
        }

        let mut after = p_before.clone();
        let mut change_set = ChangeSet::new(puts, retires);
        if let Some(cr) = &applied_for {
            change_set = change_set.for_frame(frame(cr));
        }
        if let Err(e) = after.apply(change_set) {
            return Err(self.set_error(&p, &plan, e).await?);
        }
        if let Some(cr) = &mut applied_for {
            cr.record_work(footprint_between(at.0 + 1, &p_before, &after))?;
        }
        let mut retired = Vec::new();
        for r in after.retired().iter().skip(p.retired().len()) {
            retired.push(self.retired_status(r, &plan.names).await?);
        }
        let done = set
            .puts
            .iter()
            .map(|put| put_done(put, &p, &plan, &after))
            .collect::<Result<Vec<PutDone>, AppError>>()?;
        // The change request the set is applied for gains the work in the state after, which is how an edit can
        // introduce a finding about its request.
        let mut delta = ModelDelta::new().with_project(after).with_names(plan.names);
        if let Some(cr) = applied_for {
            delta = delta.with_change_request(cr);
        }
        let findings = self.commit_edit(trigger, at, delta).await?;
        Ok(Checked {
            value: ChangeSetDone {
                puts: done,
                retired,
            },
            findings,
        })
    }

    /// A change set of one put.
    pub(super) async fn apply_one(
        &mut self,
        trigger: EditTrigger,
        put: PutInput,
    ) -> Result<Checked<ChangeSetDone>, AppError> {
        self.apply_set(
            trigger,
            &ChangeSetInput {
                change_request: None,
                puts: vec![put],
                retires: Vec::new(),
            },
        )
        .await
    }

    /// A change set of one retirement.
    pub(super) async fn retire_one(
        &mut self,
        trigger: EditTrigger,
        retire: RetireInput,
    ) -> Result<Checked<Retirement>, AppError> {
        let done = self
            .apply_set(
                trigger,
                &ChangeSetInput {
                    change_request: None,
                    puts: Vec::new(),
                    retires: vec![retire],
                },
            )
            .await?;
        Ok(Checked {
            value: Retirement {
                retired: done.value.retired,
            },
            findings: done.findings,
        })
    }

    /// The id of the asset of `kind` named `name` in `ctx`: the one it has, or a new one. It is named once in
    /// the plan, however often the change set mentions it.
    async fn reserve(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        ctx: ContextId,
        context: &ContextName,
        kind: AssetKind,
        name: &AssetName,
    ) -> Result<Uuid, AppError> {
        let key = (ctx, kind, name.clone());
        if let Some(id) = plan.ids.get(&key) {
            return Ok(*id);
        }
        let id = match self.store.asset_id(ctx, kind, name).await? {
            Some(id) => {
                refuse_retired(p, target_of(kind, id), context, kind, name)?;
                id
            }
            None => self.ids.next_uuid(),
        };
        Ok(plan.name(key, id))
    }

    /// The id of the asset of `kind` named `name` in `ctx`, which must already be named: by an entry of this
    /// change set, or in the store. It never draws an id, so it makes no name.
    async fn reserved(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        ctx: ContextId,
        context: &ContextName,
        kind: AssetKind,
        name: &AssetName,
    ) -> Result<Uuid, AppError> {
        let key = (ctx, kind, name.clone());
        if let Some(id) = plan.ids.get(&key) {
            return Ok(*id);
        }
        match self.store.asset_id(ctx, kind, name).await? {
            Some(id) => {
                refuse_retired(p, target_of(kind, id), context, kind, name)?;
                Ok(plan.name(key, id))
            }
            None => Err(AppError::UnknownAsset {
                context: context.clone(),
                kind: kind.word(),
                name: name.clone(),
            }),
        }
    }

    async fn contract_put(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        c: &ContractPut,
    ) -> Result<AssetPut, AppError> {
        let (ctx, context) = context_of(p, &c.context)?;
        let id = ContractId::from_uuid(
            self.reserve(p, plan, ctx, &context, AssetKind::Contract, &c.name)
                .await?,
        );
        let old = p.context(ctx).and_then(|x| x.contract(id));
        let read = self.read_clauses(&c.clauses, &[]);
        let found = Contract::clauses_whole(old, &read.edits);
        let given = read.settle((&context, &c.name), found)?;
        let title = Title::try_from(c.title.as_str()).map_err(input)?;
        Ok(contract_asset(plan, (ctx, id), &c.name, title, given))
    }

    /// Some Clauses of a Contract to change: the Contract as it is, with the Clauses given set and the ones to remove
    /// gone. The Contract must exist before the change set: a `clauses` entry makes no name. The names are resolved
    /// by the Contract ([`Contract::clauses_partial`]), so that a Case of the set can cite a Clause by its label as
    /// it is then, and the entry goes to the domain as [`AssetPut::Clauses`].
    ///
    /// A Contract that another entry of the same set makes is not in the Project yet, so nothing is resolved here:
    /// the entry goes to the domain all the same, which refuses it as a Contract named twice
    /// (`change-set.named-twice`), the same as any asset named twice. Such an entry is kept even when some of its
    /// Clauses cannot be read (the problem is noted, and the set is refused anyway), so that the repeat is said with
    /// the problems of planning. For a Contract the Project has, a problem is noted and there is no put.
    async fn clauses_put(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        c: &ClausesPut,
        problems: &mut Vec<AppError>,
    ) -> Option<AssetPut> {
        let ids = async {
            let (ctx, context) = context_of(p, &c.context)?;
            let id = self
                .reserved(p, plan, ctx, &context, AssetKind::Contract, &c.contract)
                .await?;
            Ok::<_, AppError>((ctx, context, ContractId::from_uuid(id)))
        };
        let (ctx, context, id) = match ids.await {
            Ok(found) => found,
            Err(e) => {
                problems.push(e);
                return None;
            }
        };
        let read = self.read_clauses(&c.clauses, &c.remove);
        if let Some(old) = p.context(ctx).and_then(|x| x.contract(id)) {
            let found = old.clauses_partial(&read.edits, &read.remove);
            match read.settle((&context, &c.contract), found) {
                Ok(given) => record_clauses(plan, id, &c.contract, &given),
                Err(e) => {
                    problems.push(e);
                    return None;
                }
            }
        } else {
            problems.extend(read.problems);
        }
        Some(AssetPut::Clauses {
            context: ctx,
            id,
            edits: read.edits,
            remove: read.remove,
        })
    }

    /// The Clauses of a put as edits, each with an id drawn for the Clause it makes if it makes one (the domain
    /// draws none), and the labels to remove. What cannot be read is a problem, and every one is kept: the names
    /// of what was read are the Contract's to check.
    fn read_clauses(&mut self, given: &[ClauseInput], remove: &[String]) -> ClausesRead {
        let mut problems = Vec::new();
        let mut edits = Vec::with_capacity(given.len());
        for cl in given {
            let read = ClauseLabel::try_from(cl.label.as_str())
                .map_err(input)
                .and_then(|label| {
                    let text = ClauseText::try_from(cl.text.as_str()).map_err(input)?;
                    let replaces = cl
                        .replaces
                        .as_deref()
                        .map(ClauseLabel::try_from)
                        .transpose()
                        .map_err(input)?;
                    Ok((label, text, replaces))
                });
            match read {
                Ok((label, text, replaces)) => {
                    let id = ClauseId::from_uuid(self.ids.next_uuid());
                    edits.push(ClauseEdit::new(label, text, replaces, id));
                }
                Err(e) => problems.push(e),
            }
        }
        let mut removed = Vec::with_capacity(remove.len());
        for label in remove {
            match ClauseLabel::try_from(label.as_str()) {
                Ok(label) => removed.push(label),
                Err(e) => problems.push(input(e)),
            }
        }
        ClausesRead {
            edits,
            remove: removed,
            problems,
        }
    }

    /// The Items the provider finds for each Interface put that gives none, by the index of the put in the set. It
    /// asks nothing of the store: it is the first step of a set, so that a provider that fails refuses the set
    /// before the store is read, and the Items are input like any other when the set is planned. An Interface put
    /// with its Items asks no provider. Every Interface that cannot be given its Items is named.
    async fn found_items(
        &self,
        set: &ChangeSetInput,
    ) -> Result<BTreeMap<usize, Vec<FoundItem>>, AppError> {
        let mut found = BTreeMap::new();
        let mut problems = Vec::new();
        for (n, put) in set.puts.iter().enumerate() {
            let PutInput::Interface(i) = put else {
                continue;
            };
            if let ItemsInput::Found { .. } = i.items {
                match self.find_items(i).await {
                    Ok(items) => {
                        found.insert(n, items);
                    }
                    Err(e) => problems.push(e),
                }
            }
        }
        if problems.is_empty() {
            Ok(found)
        } else {
            Err(one_or_many(problems))
        }
    }

    /// The Items of `i.source`, from the provider of the language `i` is laid out with.
    async fn find_items(&self, i: &InterfacePut) -> Result<Vec<FoundItem>, AppError> {
        let context = ContextName::try_from(i.context.as_str())?;
        let lang = self
            .items
            .language(&context, &i.name)
            .map_err(|e| items_error(&context, &i.name, None, e))?;
        self.items
            .items(&lang, &i.source)
            .await
            .map_err(|e| items_error(&context, &i.name, Some(&lang), e))
    }

    /// An Interface to put. `found` is what the provider found for it, when it was put with no `items`.
    async fn interface_put(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        i: &InterfacePut,
        found: Option<&[FoundItem]>,
    ) -> Result<AssetPut, AppError> {
        let (ctx, context) = context_of(p, &i.context)?;
        let id = InterfaceId::from_uuid(
            self.reserve(p, plan, ctx, &context, AssetKind::Interface, &i.name)
                .await?,
        );
        let old = p.context(ctx).and_then(|x| x.interface(id));
        let find = |key: &str| {
            let key = ItemKey::try_from(key).ok()?;
            old.and_then(|x| x.item_by_key(&key)).map(|it| it.id())
        };
        let items = match (&i.items, found) {
            (ItemsInput::Written(items), _) => items.clone(),
            (ItemsInput::Found { renames }, Some(found)) => {
                renamed((&context, i), renames, found, |key| find(key).is_some())?
            }
            (ItemsInput::Found { .. }, None) => {
                return Err(input("the items of the interface were not found"));
            }
        };
        // A rename is refused before any id is given: the domain would find an id twice, which only says that,
        // not which names are wrong.
        let mut replaced: Vec<&str> = Vec::new();
        for it in &items {
            let Some(was) = it.replaces.as_deref() else {
                continue;
            };
            if replaced.contains(&was) {
                return Err(AppError::RenameConflict {
                    context: context.clone(),
                    interface: i.name.clone(),
                    key: was.to_string(),
                    side: "replaces",
                });
            }
            if it.key != was && items.iter().any(|other| other.key == was) {
                return Err(AppError::ReplacesKept {
                    context: context.clone(),
                    interface: i.name.clone(),
                    key: was.to_string(),
                });
            }
            replaced.push(was);
        }
        let mut given = Vec::with_capacity(items.len());
        for it in &items {
            let key = ItemKey::try_from(it.key.as_str()).map_err(input)?;
            let lines = LineRange::new(it.lines.0, it.lines.1).map_err(input)?;
            let item = match &it.replaces {
                Some(was) => find(was).ok_or_else(|| AppError::UnknownItem {
                    context: context.clone(),
                    interface: i.name.clone(),
                    key: was.clone(),
                })?,
                None => match find(&it.key) {
                    Some(item) => item,
                    None => ItemId::from_uuid(self.ids.next_uuid()),
                },
            };
            given.push((item, key, lines));
        }
        plan.items.insert(
            id,
            given
                .iter()
                .map(|(item, key, _)| (key.as_str().to_string(), *item))
                .collect(),
        );
        for (item, key, _) in &given {
            plan.words
                .insert(Target::Item(*item), format!("{} {key}", i.name));
        }
        Ok(AssetPut::Interface {
            context: ctx,
            id,
            source: Source::new(i.source.clone()),
            items: given,
        })
    }

    /// A Harness to put. Every citation that cannot be resolved is added to `problems`, not only the first.
    async fn harness_put(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        h: &HarnessPut,
        problems: &mut Vec<AppError>,
    ) -> Option<AssetPut> {
        let before = problems.len();
        let (ctx, context) = match context_of(p, &h.context) {
            Ok(found) => found,
            Err(e) => {
                problems.push(e);
                return None;
            }
        };
        let id = match self
            .reserve(p, plan, ctx, &context, AssetKind::Harness, &h.name)
            .await
        {
            Ok(id) => HarnessId::from_uuid(id),
            Err(e) => {
                problems.push(e);
                return None;
            }
        };
        // A Harness put is named by the set, so that a refusal that is about it says its name before it is kept.
        plan.words
            .insert(Target::Harness(id), format!("harness {}", h.name));
        let old = p.context(ctx).and_then(|x| x.harness(id));
        let content = match &h.content {
            HarnessPutContent::Code {
                kind,
                source,
                cases: inputs,
            } => {
                let mut cases = Vec::with_capacity(inputs.len());
                for c in inputs {
                    let draft = self.case_draft(p, plan, (ctx, old), h, c, problems).await;
                    cases.extend(draft);
                }
                HarnessDraft::Code {
                    kind: *kind,
                    source: Source::new(source.clone()),
                    cases,
                }
            }
            HarnessPutContent::Examples(inputs) => {
                let mut cases = Vec::with_capacity(inputs.len());
                for e in inputs {
                    let draft = self
                        .example_draft(p, plan, (ctx, old), h, e, problems)
                        .await;
                    cases.extend(draft);
                }
                HarnessDraft::Examples(cases)
            }
        };
        let mut builds = BTreeSet::new();
        for name in &h.builds {
            match self.build_interface(plan, ctx, &context, name).await {
                Ok(interface) => {
                    builds.insert(interface);
                }
                Err(e) => problems.push(e),
            }
        }
        if problems.len() > before {
            return None;
        }
        Some(AssetPut::Harness {
            context: ctx,
            id,
            builds,
            content,
        })
    }

    /// The id of the Interface `name` of the Context `ctx` that a sim Harness builds: one the change set puts, or
    /// the Project's.
    async fn build_interface(
        &self,
        plan: &Plan,
        ctx: ContextId,
        context: &ContextName,
        name: &AssetName,
    ) -> Result<InterfaceId, AppError> {
        let id = match plan.ids.get(&(ctx, AssetKind::Interface, name.clone())) {
            Some(id) => *id,
            None => self.asset(ctx, context, AssetKind::Interface, name).await?,
        };
        Ok(InterfaceId::from_uuid(id))
    }

    /// A Case to put in the Harness `h`, which is in the Context `scope.0` and is `scope.1` there now.
    async fn case_draft(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        scope: (ContextId, Option<&Harness>),
        h: &HarnessPut,
        c: &CaseInput,
        problems: &mut Vec<AppError>,
    ) -> Option<CaseDraft> {
        let (ctx, old) = scope;
        let name = match CaseName::try_from(c.name.as_str()) {
            Ok(n) => n,
            Err(e) => {
                problems.push(input(e));
                return None;
            }
        };
        let lines = match LineRange::new(c.lines.0, c.lines.1) {
            Ok(l) => l,
            Err(e) => {
                problems.push(input(e));
                return None;
            }
        };
        let mut items = BTreeSet::new();
        for r in &c.items {
            match self.cite_item(p, plan, ctx, r).await {
                Ok(item) => {
                    items.insert(item);
                }
                Err(e) => problems.push(e),
            }
        }
        let mut clauses = BTreeSet::new();
        for r in &c.clauses {
            match self.cite_clause(p, plan, ctx, r).await {
                Ok(clause) => {
                    clauses.insert(clause);
                }
                Err(e) => problems.push(e),
            }
        }
        let id = match old.and_then(|x| x.cases().iter().find(|k| *k.name() == name)) {
            Some(k) => k.id(),
            None => CaseId::from_uuid(self.ids.next_uuid()),
        };
        plan.words
            .insert(Target::Case(id), format!("case {} {}", h.name, c.name));
        Some(CaseDraft::new(id, name, lines, items, clauses))
    }

    /// An example Case to put in the Harness `h`, which is in the Context `scope.0` and is `scope.1` there now. Its
    /// texts become the domain's example here; one whose expected output has no `#exit` line is a refusal of the
    /// rule `examples.expected-exit`, about the Case, as every other problem of the input is collected.
    async fn example_draft(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        scope: (ContextId, Option<&Harness>),
        h: &HarnessPut,
        e: &ExampleInput,
        problems: &mut Vec<AppError>,
    ) -> Option<ExampleDraft> {
        let (ctx, old) = scope;
        let name = match CaseName::try_from(e.name.as_str()) {
            Ok(n) => n,
            Err(err) => {
                problems.push(input(err));
                return None;
            }
        };
        let mut clauses = BTreeSet::new();
        for r in &e.clauses {
            match self.cite_clause(p, plan, ctx, r).await {
                Ok(clause) => {
                    clauses.insert(clause);
                }
                Err(err) => problems.push(err),
            }
        }
        let id = match old.and_then(|x| x.cases().iter().find(|k| *k.name() == name)) {
            Some(k) => k.id(),
            None => CaseId::from_uuid(self.ids.next_uuid()),
        };
        let words = format!("case {} {}", h.name, e.name);
        plan.words.insert(Target::Case(id), words.clone());
        let t = &e.example;
        let example = match Example::new(
            t.args.clone(),
            t.stdin.clone(),
            t.expected.clone(),
            t.stderr.clone(),
        ) {
            Ok(example) => example,
            Err(err) => {
                let mut names = Names::default();
                names.insert(Party::Target(Target::Case(id)), words);
                problems.push(AppError::Violated {
                    violations: vec![err.violation(id)],
                    names,
                });
                return None;
            }
        };
        Some(ExampleDraft::new(id, name, example, clauses))
    }

    /// The id of the Item `r` cites, among the Interfaces the change set puts and the Project's.
    async fn cite_item(
        &self,
        p: &Project,
        plan: &Plan,
        default: ContextId,
        r: &super::ItemRef,
    ) -> Result<ItemId, AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), Some(default))?;
        let iface = match plan
            .ids
            .get(&(ctx, AssetKind::Interface, r.interface.clone()))
        {
            Some(id) => *id,
            None => {
                self.asset(ctx, &ctx_name, AssetKind::Interface, &r.interface)
                    .await?
            }
        };
        let iface = InterfaceId::from_uuid(iface);
        let found = match plan.items.get(&iface) {
            Some(items) => items
                .iter()
                .find(|(key, _)| *key == r.key)
                .map(|(_, id)| *id),
            None => {
                let key = ItemKey::try_from(r.key.as_str()).map_err(input)?;
                p.context(ctx)
                    .and_then(|c| c.interface(iface))
                    .and_then(|i| i.item_by_key(&key))
                    .map(|it| it.id())
            }
        };
        found.ok_or_else(|| AppError::UnknownItem {
            context: ctx_name.clone(),
            interface: r.interface.clone(),
            key: r.key.clone(),
        })
    }

    /// The id of the Clause `r` cites, among the Contracts the change set puts and the Project's.
    async fn cite_clause(
        &self,
        p: &Project,
        plan: &Plan,
        default: ContextId,
        r: &super::ClauseRef,
    ) -> Result<ClauseId, AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), Some(default))?;
        let contract = match plan
            .ids
            .get(&(ctx, AssetKind::Contract, r.contract.clone()))
        {
            Some(id) => *id,
            None => {
                self.asset(ctx, &ctx_name, AssetKind::Contract, &r.contract)
                    .await?
            }
        };
        let contract = ContractId::from_uuid(contract);
        let label = ClauseLabel::try_from(r.label.as_str()).map_err(input)?;
        let found = match plan.clauses.get(&contract) {
            Some(clauses) => clauses.iter().find(|(l, _)| *l == label).map(|(_, id)| *id),
            None => p
                .context(ctx)
                .and_then(|c| c.contract(contract))
                .and_then(|k| k.clause_by_label(label))
                .map(|k| k.id()),
        };
        found.ok_or_else(|| AppError::UnknownClause {
            context: ctx_name.clone(),
            contract: r.contract.clone(),
            label: r.label.clone(),
        })
    }

    /// What a retirement is of, by id.
    async fn retire_target(&self, p: &Project, r: &RetireInput) -> Result<AssetRetire, AppError> {
        match r {
            RetireInput::Context(name) => Ok(AssetRetire::Context(context_id(p, name)?)),
            RetireInput::Contract { context, contract } => {
                let (ctx, context) = context_of(p, context)?;
                let id = ContractId::from_uuid(
                    self.asset(ctx, &context, AssetKind::Contract, contract)
                        .await?,
                );
                refuse_retired(
                    p,
                    Target::Contract(id),
                    &context,
                    AssetKind::Contract,
                    contract,
                )?;
                Ok(AssetRetire::Contract { context: ctx, id })
            }
            RetireInput::Interface { context, interface } => {
                let (ctx, context) = context_of(p, context)?;
                let id = InterfaceId::from_uuid(
                    self.asset(ctx, &context, AssetKind::Interface, interface)
                        .await?,
                );
                refuse_retired(
                    p,
                    Target::Interface(id),
                    &context,
                    AssetKind::Interface,
                    interface,
                )?;
                Ok(AssetRetire::Interface { context: ctx, id })
            }
            RetireInput::Harness { context, harness } => {
                let (ctx, context) = context_of(p, context)?;
                let id = HarnessId::from_uuid(
                    self.asset(ctx, &context, AssetKind::Harness, harness)
                        .await?,
                );
                refuse_retired(
                    p,
                    Target::Harness(id),
                    &context,
                    AssetKind::Harness,
                    harness,
                )?;
                Ok(AssetRetire::Harness { context: ctx, id })
            }
        }
    }

    /// A refusal of the domain in the names a person knows: every Case that cites what it cannot reach by its
    /// Harness and name and what it cites by its Contract or Interface and label or key, the Cases and
    /// Contexts that stand in the way of a retirement, the thing named twice. `before` is the Project as it was.
    async fn set_error(
        &self,
        before: &Project,
        plan: &Plan,
        e: ProjectError,
    ) -> Result<AppError, AppError> {
        let all = match e {
            ProjectError::Many(all) => all,
            one => vec![one],
        };
        let mut out = Vec::with_capacity(all.len());
        for e in all {
            out.push(self.named_error(before, plan, e).await?);
        }
        Ok(match out.as_slice() {
            [only] => only.clone(),
            _ => AppError::Many(out),
        })
    }

    async fn named_error(
        &self,
        before: &Project,
        plan: &Plan,
        e: ProjectError,
    ) -> Result<AppError, AppError> {
        let violations = e.violations();
        let mut names = Names::default();
        for party in violations.iter().flat_map(Violation::parties) {
            let words = self
                .words_of(before, &plan.words, &plan.names, party)
                .await?;
            names.insert(party, words);
        }
        Ok(AppError::Violated { violations, names })
    }
}

/// The refusal that `e`, from the source of Items, is for the Interface `interface` of `context`; `lang` is its
/// language once known.
pub(super) fn items_error(
    context: &ContextName,
    interface: &AssetName,
    lang: Option<&str>,
    e: ItemsError,
) -> AppError {
    let (context, interface) = (context.clone(), interface.clone());
    let language = || lang.unwrap_or("?").to_string();
    match e {
        ItemsError::NoLanguage => AppError::ItemsRequired { context, interface },
        ItemsError::NoProvider => AppError::ItemsNoProvider {
            context,
            interface,
            lang: lang.map(str::to_string),
        },
        ItemsError::Protocol(message) => AppError::ItemsProtocol {
            context,
            interface,
            lang: language(),
            message,
        },
        ItemsError::Failed(message) => AppError::ItemsFailed {
            context,
            interface,
            lang: language(),
            message,
        },
    }
}

/// The Items the provider found, as Items that are written: the one found under the `to` of a rename replaces its
/// `from`, so it takes that Item's id, and no other does. `had` says whether the Interface had an Item with a key.
/// A rename that cannot be carried out is named, whichever it is, and none is ignored: a `from` or a `to` named
/// twice, a `to` the provider did not find, a `from` the Interface did not have, a `from` the provider still finds.
fn renamed(
    (context, i): (&ContextName, &InterfacePut),
    renames: &[RenameInput],
    found: &[FoundItem],
    had: impl Fn(&str) -> bool,
) -> Result<Vec<ItemInput>, AppError> {
    let mut problems = Vec::new();
    for (n, r) in renames.iter().enumerate() {
        let earlier = &renames[..n];
        let (context, interface) = (context.clone(), i.name.clone());
        let conflict = |key: &str, side| AppError::RenameConflict {
            context: context.clone(),
            interface: interface.clone(),
            key: key.to_string(),
            side,
        };
        let unknown = |key: &str, field| AppError::RenameUnknown {
            context: context.clone(),
            interface: interface.clone(),
            key: key.to_string(),
            field,
        };
        if earlier.iter().any(|e| e.from == r.from) {
            problems.push(conflict(&r.from, "from"));
        } else if earlier.iter().any(|e| e.to == r.to) {
            problems.push(conflict(&r.to, "to"));
        } else if !found.iter().any(|f| f.key == r.to) {
            problems.push(unknown(&r.to, "to"));
        } else if !had(&r.from) {
            problems.push(unknown(&r.from, "from"));
        } else if r.from != r.to && found.iter().any(|f| f.key == r.from) {
            problems.push(AppError::ReplacesKept {
                context: context.clone(),
                interface: interface.clone(),
                key: r.from.clone(),
            });
        }
    }
    if !problems.is_empty() {
        return Err(one_or_many(problems));
    }
    // A key the provider gave twice is the domain's to refuse (`interface.key-taken`); only the first is renamed.
    let mut seen = BTreeSet::new();
    Ok(found
        .iter()
        .map(|f| ItemInput {
            key: f.key.clone(),
            lines: f.lines,
            replaces: if seen.insert(f.key.as_str()) {
                renames
                    .iter()
                    .find(|r| r.to == f.key)
                    .map(|r| r.from.clone())
            } else {
                None
            },
        })
        .collect())
}

/// The Clauses of a put as they were read: the edits, the labels to remove, and what could not be read.
struct ClausesRead {
    edits: Vec<ClauseEdit>,
    remove: Vec<ClauseLabel>,
    problems: Vec<AppError>,
}

impl ClausesRead {
    /// The list of Clauses `found` gives, when what was read had no problem; else every problem, the ones in the
    /// reading and the ones with the names. `at` is the Context and the Contract's name.
    fn settle(
        &self,
        at: (&ContextName, &AssetName),
        found: Result<Vec<(ClauseId, ClauseContent)>, Vec<ClauseNameProblem>>,
    ) -> Result<Vec<(ClauseId, ClauseContent)>, AppError> {
        let mut problems = self.problems.clone();
        match found {
            Ok(list) if problems.is_empty() => return Ok(list),
            Ok(_) => {}
            Err(names) => problems.extend(names.iter().map(|n| name_error(at, n))),
        }
        Err(one_or_many(problems))
    }
}

/// The refusal that a problem with the names of a put is, in the words a person knows. `at` is the Context and the
/// Contract's name.
fn name_error(at: (&ContextName, &AssetName), problem: &ClauseNameProblem) -> AppError {
    let (context, contract) = (at.0.clone(), at.1.clone());
    match problem {
        ClauseNameProblem::Unknown { label, field } => AppError::ClauseUnknown {
            context,
            contract,
            label: label.to_string(),
            field: field.word(),
        },
        ClauseNameProblem::Conflict { label, side } => AppError::ClauseRenameConflict {
            context,
            contract,
            label: label.to_string(),
            side: side.word(),
        },
        ClauseNameProblem::ReplacesKept { label } => AppError::ClauseReplacesKept {
            context,
            contract,
            label: label.to_string(),
        },
    }
}

/// Records the Clauses of the Contract `id` as the put leaves them (all of them, the ones the put did not name
/// included): a Case of the same set cites any of them by its label as it is then. `name` is the Contract's.
fn record_clauses(
    plan: &mut Plan,
    id: ContractId,
    name: &AssetName,
    clauses: &[(ClauseId, ClauseContent)],
) {
    plan.clauses.insert(
        id,
        clauses
            .iter()
            .map(|(clause, content)| (content.label(), *clause))
            .collect(),
    );
    for (clause, content) in clauses {
        plan.words.insert(
            Target::Clause(*clause),
            format!("{name} {}", content.label()),
        );
    }
}

/// The Contract `id` of `ctx` to put with `clauses` as its whole list of Clauses, in order.
fn contract_asset(
    plan: &mut Plan,
    (ctx, id): (ContextId, ContractId),
    name: &AssetName,
    title: Title,
    clauses: Vec<(ClauseId, ClauseContent)>,
) -> AssetPut {
    record_clauses(plan, id, name, &clauses);
    AssetPut::Contract {
        context: ctx,
        id,
        title,
        clauses,
    }
}

/// The error of several problems: the one, or all of them.
fn one_or_many(problems: Vec<AppError>) -> AppError {
    match problems.as_slice() {
        [only] => only.clone(),
        _ => AppError::Many(problems),
    }
}

/// Keeps what was built, or notes why it was not.
fn keep_or_note(
    done: Result<AssetPut, AppError>,
    puts: &mut Vec<AssetPut>,
    problems: &mut Vec<AppError>,
) {
    match done {
        Ok(put) => puts.push(put),
        Err(e) => problems.push(e),
    }
}

/// The Target an asset of `kind` with this id is.
fn target_of(kind: AssetKind, id: Uuid) -> Target {
    match kind {
        AssetKind::Contract => Target::Contract(ContractId::from_uuid(id)),
        AssetKind::Interface => Target::Interface(InterfaceId::from_uuid(id)),
        AssetKind::Harness => Target::Harness(HarnessId::from_uuid(id)),
    }
}

/// What a put came to, from the plan and the Project after the change (`before` resolves the Context's name).
fn put_done(
    put: &PutInput,
    before: &Project,
    plan: &Plan,
    after: &Project,
) -> Result<PutDone, AppError> {
    match put {
        PutInput::Context { name, .. } => {
            let found = context_id(after, name)
                .ok()
                .and_then(|id| after.context(id))
                .map(|c| (c.id().as_uuid(), c.revision()));
            let (id, revision) = found.ok_or_else(|| {
                AppError::Defect(format!(
                    "the context `{name}` has no id or revision after put"
                ))
            })?;
            Ok(PutDone::Context {
                name: name.clone(),
                id,
                revision,
            })
        }
        PutInput::Uses { context, .. } => Ok(PutDone::Setting {
            kind: SettingKind::Uses,
            context: context.clone(),
        }),
        PutInput::BaseOnContract { context } => Ok(PutDone::Setting {
            kind: SettingKind::BaseOnContract,
            context: context.clone(),
        }),
        PutInput::Contract(c) => asset_put_done(
            AssetPutKind::Contract,
            AssetKind::Contract,
            &c.context,
            &c.name,
            before,
            plan,
            after,
        ),
        PutInput::Clauses(c) => asset_put_done(
            AssetPutKind::Clauses,
            AssetKind::Contract,
            &c.context,
            &c.contract,
            before,
            plan,
            after,
        ),
        PutInput::Interface(i) => asset_put_done(
            AssetPutKind::Interface,
            AssetKind::Interface,
            &i.context,
            &i.name,
            before,
            plan,
            after,
        ),
        PutInput::Harness(h) => asset_put_done(
            AssetPutKind::Harness,
            AssetKind::Harness,
            &h.context,
            &h.name,
            before,
            plan,
            after,
        ),
    }
}

fn asset_put_done(
    put_kind: AssetPutKind,
    asset_kind: AssetKind,
    context: &str,
    name: &AssetName,
    before: &Project,
    plan: &Plan,
    after: &Project,
) -> Result<PutDone, AppError> {
    let ctx = context_id(before, context)
        .map_err(|_| AppError::Defect(format!("context `{context}` not found in before")))?;
    let id = plan
        .ids
        .get(&(ctx, asset_kind, name.clone()))
        .copied()
        .ok_or_else(|| {
            AppError::Defect(format!(
                "{} `{name}` in context `{context}` has no id in plan",
                put_kind.word()
            ))
        })?;
    let c = after
        .context(ctx)
        .ok_or_else(|| AppError::Defect(format!("context `{context}` not found in after")))?;
    let revision = match asset_kind {
        AssetKind::Contract => c.contract(ContractId::from_uuid(id)).map(|k| k.revision()),
        AssetKind::Interface => c
            .interface(InterfaceId::from_uuid(id))
            .map(|i| i.revision()),
        AssetKind::Harness => c.harness(HarnessId::from_uuid(id)).map(|h| h.revision()),
    }
    .ok_or_else(|| {
        AppError::Defect(format!(
            "{} `{name}` in context `{context}` has no revision in after",
            put_kind.word()
        ))
    })?;
    Ok(PutDone::Asset {
        kind: put_kind,
        context: context.to_string(),
        name: name.to_string(),
        id,
        revision,
    })
}

/// The change set of one setting of a Context (its `uses`, or what it is based on), which makes no asset.
pub(super) fn only_setting(done: Checked<ChangeSetDone>) -> Checked<()> {
    Checked {
        value: (),
        findings: done.findings,
    }
}

/// The one asset a change set of one put made.
pub(super) fn only_put<I>(
    done: Checked<ChangeSetDone>,
    make: fn(Uuid) -> I,
) -> Result<Checked<Put<I>>, AppError> {
    let Checked { value, findings } = done;
    let put = value
        .puts
        .into_iter()
        .next()
        .and_then(|d| match d {
            PutDone::Asset { id, revision, .. } => Some(Put {
                id: make(id),
                revision,
            }),
            _ => None,
        })
        .ok_or_else(|| input("the change set made no asset"))?;
    Ok(Checked {
        value: put,
        findings,
    })
}
