//! The change set use case: several assets put and retired as one transition of the Project.
//!
//! A put or a retirement of one asset is a change set of one, so there is a single path that turns names into ids,
//! lets the domain apply the whole and check its rules once, runs the put-hook gate once on the Project before and
//! after, and commits once, with the names of the assets it made.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::ports::external::ExternalExecutor;
use super::ports::{AssetKind, AssetName, Changes, IdSource, Store};
use super::{
    App, AppError, CaseInput, Checked, ClauseInput, ItemInput, Put, RetiredStatus, Retirement,
    context_id, context_ids, input, refuse_retired,
};
use crate::domain::check::Hook;
use crate::domain::entity::harness::Harness;
use crate::domain::entity::project::{Project, ProjectError};
use crate::domain::vo::case::CaseName;
use crate::domain::vo::change_set::{AssetPut, AssetRetire, CaseDraft, ChangeSet};
use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{
    CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
};
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::signature::Signature;
use crate::domain::vo::source::Source;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// A Contract to make or revise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractPut {
    /// The Context's name.
    pub context: String,
    /// The Contract's name.
    pub name: String,
    /// Its title.
    pub title: String,
    /// Its Clauses; one keeps its id when its label is one the Contract has.
    pub clauses: Vec<ClauseInput>,
}

/// An Interface to make or revise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfacePut {
    /// The Context's name.
    pub context: String,
    /// The Interface's name.
    pub name: String,
    /// Its code.
    pub source: String,
    /// Its Items; one keeps its id when its signature (or the one it replaces) is one the Interface has.
    pub items: Vec<ItemInput>,
}

/// A Harness to make or revise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessPut {
    /// The Context's name.
    pub context: String,
    /// The Harness's name.
    pub name: String,
    /// Its kind; none: the kind it has, or tests for a new one.
    pub kind: Option<HarnessKind>,
    /// Its code.
    pub source: String,
    /// Its Cases. What they cite is named as in a put of the Harness alone, and may be a Clause or an Item that
    /// the same change set makes.
    pub cases: Vec<CaseInput>,
}

/// One thing a change set puts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutInput {
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
}

/// A change set as a person writes it: the assets to put, and the things to retire, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSetInput {
    /// What is put.
    pub puts: Vec<PutInput>,
    /// What is retired.
    pub retires: Vec<RetireInput>,
}

/// What one put of a change set came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutDone {
    /// `uses`, `base_on_contract`, `contract`, `interface` or `harness`.
    pub kind: &'static str,
    /// The Context's name.
    pub context: String,
    /// The asset's name; none for a setting of a Context.
    pub name: Option<String>,
    /// The asset's id; none for a setting of a Context.
    pub id: Option<Uuid>,
    /// The asset's revision after the change; none for a setting of a Context.
    pub revision: Option<Revision>,
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
    ids: BTreeMap<(ContextId, AssetKind, String), Uuid>,
    /// The names to keep with the Project, one for each asset put.
    names: Vec<AssetName>,
    /// The Items of each Interface put: signature and id.
    items: BTreeMap<InterfaceId, Vec<(String, ItemId)>>,
    /// The Clauses of each Contract put: label and id.
    clauses: BTreeMap<ContractId, Vec<(ClauseLabel, ClauseId)>>,
    /// How to say the Items, Clauses and Cases that the change set makes, which the stored Project does not have.
    words: BTreeMap<Target, String>,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Applies a change set: all the puts and retirements together, or nothing.
    ///
    /// Names are turned into ids first (a Case may cite a Clause or an Item that the same change set makes), then
    /// the domain applies the whole to a copy of the Project and checks the rules of the Project once, on the state
    /// it ends in; the put hook runs once on the Project before and after; and one commit keeps the Project and
    /// the names of the assets made. When it is refused nothing is kept, and the error names every problem found
    /// (a Case that cites what it cannot reach is named with its Harness, and what it cites with the Contract or
    /// Interface that has it).
    pub async fn apply_change_set(
        &mut self,
        set: &ChangeSetInput,
    ) -> Result<Checked<ChangeSetDone>, AppError> {
        if set.puts.is_empty() && set.retires.is_empty() {
            return Err(input("the change set has nothing to put or retire"));
        }
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let mut plan = Plan::default();
        let mut problems = Vec::new();
        let mut puts = Vec::new();

        // The definitions first: a Case that cites one finds the ids and labels of the change set's own.
        for put in &set.puts {
            match put {
                PutInput::Contract(c) => {
                    let done = self.contract_put(&p, &mut plan, c).await;
                    keep_or_note(done, &mut puts, &mut problems);
                }
                PutInput::Interface(i) => {
                    let done = self.interface_put(&p, &mut plan, i).await;
                    keep_or_note(done, &mut puts, &mut problems);
                }
                PutInput::Uses { .. } | PutInput::BaseOnContract { .. } | PutInput::Harness(_) => {}
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
                PutInput::Contract(_) | PutInput::Interface(_) => {}
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
            return Err(one_or_many(problems));
        }

        let mut after = p.clone();
        if let Err(e) = after.apply(ChangeSet::new(puts, retires)) {
            return Err(self.set_error(&p, &plan, e).await?);
        }
        let gate = self
            .gate(Hook::Put, at, Some(&p), &after, &plan.names)
            .await?;
        let mut retired = Vec::new();
        for r in after.retired().iter().skip(p.retired().len()) {
            retired.push(self.retired_status(r, &plan.names).await?);
        }
        let done = set
            .puts
            .iter()
            .map(|put| put_done(put, &p, &plan, &after))
            .collect();
        let changes = Changes {
            project: Some(after),
            names: plan.names,
            invocations: gate.invocations,
            external: gate.external,
            ..Changes::default()
        };
        self.store.commit(at, changes).await?;
        Ok(Checked {
            value: ChangeSetDone {
                puts: done,
                retired,
            },
            findings: gate.found,
        })
    }

    /// A change set of one put.
    pub(super) async fn apply_one(
        &mut self,
        put: PutInput,
    ) -> Result<Checked<ChangeSetDone>, AppError> {
        self.apply_change_set(&ChangeSetInput {
            puts: vec![put],
            retires: Vec::new(),
        })
        .await
    }

    /// A change set of one retirement.
    pub(super) async fn retire_one(
        &mut self,
        retire: RetireInput,
    ) -> Result<Checked<Retirement>, AppError> {
        let done = self
            .apply_change_set(&ChangeSetInput {
                puts: Vec::new(),
                retires: vec![retire],
            })
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
        context: &str,
        kind: AssetKind,
        name: &str,
    ) -> Result<Uuid, AppError> {
        let key = (ctx, kind, name.to_string());
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
        plan.ids.insert(key, id);
        plan.names.push(AssetName {
            context: ctx,
            kind,
            name: name.to_string(),
            id,
        });
        Ok(id)
    }

    async fn contract_put(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        c: &ContractPut,
    ) -> Result<AssetPut, AppError> {
        let ctx = context_id(p, &c.context)?;
        let id = ContractId::from_uuid(
            self.reserve(p, plan, ctx, &c.context, AssetKind::Contract, &c.name)
                .await?,
        );
        let old = p.context(ctx).and_then(|x| x.contract(id));
        let mut given = Vec::with_capacity(c.clauses.len());
        for cl in &c.clauses {
            let label = ClauseLabel::try_from(cl.label.as_str()).map_err(input)?;
            let text = ClauseText::try_from(cl.text.as_str()).map_err(input)?;
            let clause = match old.and_then(|k| k.clause_by_label(label)) {
                Some(k) => k.id(),
                None => ClauseId::from_uuid(self.ids.next_uuid()),
            };
            given.push((clause, ClauseContent::new(label, text)));
        }
        let title = Title::try_from(c.title.as_str()).map_err(input)?;
        plan.clauses.insert(
            id,
            given
                .iter()
                .map(|(clause, content)| (content.label(), *clause))
                .collect(),
        );
        for (clause, content) in &given {
            plan.words.insert(
                Target::Clause(*clause),
                format!("{} {}", c.name, content.label()),
            );
        }
        Ok(AssetPut::Contract {
            context: ctx,
            id,
            title,
            clauses: given,
        })
    }

    async fn interface_put(
        &mut self,
        p: &Project,
        plan: &mut Plan,
        i: &InterfacePut,
    ) -> Result<AssetPut, AppError> {
        let ctx = context_id(p, &i.context)?;
        let id = InterfaceId::from_uuid(
            self.reserve(p, plan, ctx, &i.context, AssetKind::Interface, &i.name)
                .await?,
        );
        let old = p.context(ctx).and_then(|x| x.interface(id));
        let find = |sig: &str| {
            old.and_then(|x| x.items().iter().find(|it| it.signature().as_str() == sig))
                .map(|it| it.id())
        };
        let mut given = Vec::with_capacity(i.items.len());
        for it in &i.items {
            let signature = Signature::try_from(it.signature.as_str()).map_err(input)?;
            let lines = LineRange::new(it.lines.0, it.lines.1).map_err(input)?;
            let item = match &it.replaces {
                Some(was) => find(was).ok_or_else(|| AppError::UnknownItem {
                    interface: i.name.clone(),
                    signature: was.clone(),
                })?,
                None => match find(&it.signature) {
                    Some(item) => item,
                    None => ItemId::from_uuid(self.ids.next_uuid()),
                },
            };
            given.push((item, signature, lines));
        }
        plan.items.insert(
            id,
            given
                .iter()
                .map(|(item, signature, _)| (signature.as_str().to_string(), *item))
                .collect(),
        );
        for (item, signature, _) in &given {
            plan.words
                .insert(Target::Item(*item), format!("{} {signature}", i.name));
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
        let ctx = match context_id(p, &h.context) {
            Ok(ctx) => ctx,
            Err(e) => {
                problems.push(e);
                return None;
            }
        };
        let id = match self
            .reserve(p, plan, ctx, &h.context, AssetKind::Harness, &h.name)
            .await
        {
            Ok(id) => HarnessId::from_uuid(id),
            Err(e) => {
                problems.push(e);
                return None;
            }
        };
        let old = p.context(ctx).and_then(|x| x.harness(id));
        let mut cases = Vec::with_capacity(h.cases.len());
        for c in &h.cases {
            let draft = self.case_draft(p, plan, (ctx, old), h, c, problems).await;
            cases.extend(draft);
        }
        if problems.len() > before {
            return None;
        }
        Some(AssetPut::Harness {
            context: ctx,
            id,
            kind: h.kind,
            source: Source::new(h.source.clone()),
            cases,
        })
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
        match CaseDraft::new(id, name, lines, items, clauses) {
            Ok(d) => Some(d),
            Err(e) => {
                problems.push(input(e));
                None
            }
        }
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
                .find(|(sig, _)| *sig == r.signature)
                .map(|(_, id)| *id),
            None => p
                .context(ctx)
                .and_then(|c| c.interface(iface))
                .and_then(|i| {
                    i.items()
                        .iter()
                        .find(|it| it.signature().as_str() == r.signature)
                })
                .map(|it| it.id()),
        };
        found.ok_or_else(|| AppError::UnknownItem {
            interface: r.interface.clone(),
            signature: r.signature.clone(),
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
            contract: r.contract.clone(),
            label: r.label.clone(),
        })
    }

    /// What a retirement is of, by id.
    async fn retire_target(&self, p: &Project, r: &RetireInput) -> Result<AssetRetire, AppError> {
        match r {
            RetireInput::Context(name) => Ok(AssetRetire::Context(context_id(p, name)?)),
            RetireInput::Contract { context, contract } => {
                let ctx = context_id(p, context)?;
                let id = ContractId::from_uuid(
                    self.asset(ctx, context, AssetKind::Contract, contract)
                        .await?,
                );
                refuse_retired(
                    p,
                    Target::Contract(id),
                    context,
                    AssetKind::Contract,
                    contract,
                )?;
                Ok(AssetRetire::Contract { context: ctx, id })
            }
            RetireInput::Interface { context, interface } => {
                let ctx = context_id(p, context)?;
                let id = InterfaceId::from_uuid(
                    self.asset(ctx, context, AssetKind::Interface, interface)
                        .await?,
                );
                refuse_retired(
                    p,
                    Target::Interface(id),
                    context,
                    AssetKind::Interface,
                    interface,
                )?;
                Ok(AssetRetire::Interface { context: ctx, id })
            }
            RetireInput::Harness { context, harness } => {
                let ctx = context_id(p, context)?;
                let id = HarnessId::from_uuid(
                    self.asset(ctx, context, AssetKind::Harness, harness)
                        .await?,
                );
                refuse_retired(p, Target::Harness(id), context, AssetKind::Harness, harness)?;
                Ok(AssetRetire::Harness { context: ctx, id })
            }
        }
    }

    /// A refusal of the domain in the names a person knows: every Case that cites what it cannot reach by its
    /// Harness and name and what it cites by its Contract or Interface and label or signature, the Cases and
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
        let context_name = |id: ContextId| {
            before
                .context(id)
                .map_or_else(|| id.to_string(), |c| c.name().to_string())
        };
        Ok(match e {
            ProjectError::ItemNotReached {
                context,
                case,
                item,
            } => AppError::NotReached {
                case: self.said(before, plan, Target::Case(case)).await?,
                context: context_name(context),
                kind: "item",
                target: self.said(before, plan, Target::Item(item)).await?,
            },
            ProjectError::ClauseNotReached {
                context,
                case,
                clause,
            } => AppError::NotReached {
                case: self.said(before, plan, Target::Case(case)).await?,
                context: context_name(context),
                kind: "clause",
                target: self.said(before, plan, Target::Clause(clause)).await?,
            },
            ProjectError::RetireCited { cited, cases } => {
                let mut said_cited = Vec::with_capacity(cited.len());
                for t in cited {
                    said_cited.push(self.said(before, plan, t).await?);
                }
                let mut said_cases = Vec::with_capacity(cases.len());
                for c in cases {
                    said_cases.push(self.said(before, plan, Target::Case(c)).await?);
                }
                AppError::RetireCited {
                    cited: said_cited,
                    cases: said_cases,
                }
            }
            ProjectError::RetireUsed { context, by } => AppError::RetireUsed {
                context: context_name(context),
                by: by.into_iter().map(context_name).collect(),
            },
            ProjectError::Repeated { what, id } => {
                let name = match plan.names.iter().find(|n| n.id == id) {
                    Some(n) => n.name.clone(),
                    None => match before.contexts().find(|c| c.id().as_uuid() == id) {
                        Some(c) => c.name().to_string(),
                        None => self.name_of(id).await?,
                    },
                };
                AppError::NamedTwice { what, name }
            }
            other => other.into(),
        })
    }

    /// A target in words: what the change set makes as it names it, anything else as the Project describes it.
    async fn said(
        &self,
        before: &Project,
        plan: &Plan,
        target: Target,
    ) -> Result<String, AppError> {
        match plan.words.get(&target) {
            Some(words) => Ok(words.clone()),
            None => self.describe_in(before, target, &plan.names).await,
        }
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
fn put_done(put: &PutInput, before: &Project, plan: &Plan, after: &Project) -> PutDone {
    let (kind, context, asset_kind, name) = match put {
        PutInput::Uses { context, .. } => ("uses", context, None, None),
        PutInput::BaseOnContract { context } => ("base_on_contract", context, None, None),
        PutInput::Contract(c) => (
            "contract",
            &c.context,
            Some(AssetKind::Contract),
            Some(&c.name),
        ),
        PutInput::Interface(i) => (
            "interface",
            &i.context,
            Some(AssetKind::Interface),
            Some(&i.name),
        ),
        PutInput::Harness(h) => (
            "harness",
            &h.context,
            Some(AssetKind::Harness),
            Some(&h.name),
        ),
    };
    let (Some(asset_kind), Some(name), Ok(ctx)) = (asset_kind, name, context_id(before, context))
    else {
        return PutDone {
            kind,
            context: context.clone(),
            name: None,
            id: None,
            revision: None,
        };
    };
    let context = context.clone();
    let name = name.clone();
    let id = plan.ids.get(&(ctx, asset_kind, name.clone())).copied();
    let revision = id.and_then(|id| {
        let c = after.context(ctx)?;
        match asset_kind {
            AssetKind::Contract => c.contract(ContractId::from_uuid(id)).map(|k| k.revision()),
            AssetKind::Interface => c
                .interface(InterfaceId::from_uuid(id))
                .map(|i| i.revision()),
            AssetKind::Harness => c.harness(HarnessId::from_uuid(id)).map(|h| h.revision()),
        }
    });
    PutDone {
        kind,
        context,
        name: Some(name),
        id,
        revision,
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
        .and_then(|d| {
            Some(Put {
                id: make(d.id?),
                revision: d.revision?,
            })
        })
        .ok_or_else(|| input("the change set made no asset"))?;
    Ok(Checked {
        value: put,
        findings,
    })
}
