//! The Project: the root that holds the Contexts of one repository and the rules between them.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use uuid::Uuid;

mod apply;

use super::context::{Context, ContextError};
use super::contract::{Clause, Contract};
use super::harness::{Case, CaseFound, Harness, SimShapeProblem};
use super::interface::{Interface, Item, ItemFound};
use crate::domain::vo::clause::ClauseContent;
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{
    CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId, ProjectId,
};
use crate::domain::vo::name::ContextName;
use crate::domain::vo::policy::{Policy, PolicyError, PolicyLineId};
use crate::domain::vo::prose::Prose;
use crate::domain::vo::retired::Retired;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::source::Source;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// Which rule of the Project a change would break. The change is not made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectError {
    /// No Context has this id.
    #[error("the context {0} is not in the project")]
    UnknownContext(ContextId),
    /// A Context with this id is already in the Project.
    #[error("the context {0} is already in the project")]
    DuplicateId(ContextId),
    /// Two Contexts would have one name.
    #[error("the context name `{0}` is taken")]
    DuplicateName(ContextName),
    /// A Context uses a Context that is not in the Project.
    #[error("the context {context} uses {used}, which is not in the project")]
    UnknownUse {
        /// The Context that uses it.
        context: ContextId,
        /// The id that is not there.
        used: ContextId,
    },
    /// Contexts would use each other in a ring; the ids are those on it, in the order of use.
    #[error("the contexts would use each other in a cycle: {0:?}")]
    Cycle(Vec<ContextId>),
    /// An asset or a definition would have an id that another one in the Project has, or that the Project retired
    /// (a retired id is reserved for good). A citation names its target by id alone, so an id is one thing in the
    /// whole Project.
    #[error("the id {0} is given to two things in the project, or was retired")]
    IdTaken(Uuid),
    /// Retiring would take away Items or Clauses that Cases which stay still cite. Retire those Cases first (or put
    /// their Harness again without the citation).
    #[error(
        "cannot retire: the cases {} cite what would go",
        listed(.cases)
    )]
    RetireCited {
        /// The Items and Clauses that would go and are cited, in order.
        cited: Vec<Target>,
        /// The Cases that stay and cite them, in order.
        cases: Vec<CaseId>,
    },
    /// Retiring a Context that other Contexts use. Change their uses (or retire them) first.
    #[error("cannot retire the context {context}: the contexts {} use it", listed(.by))]
    RetireUsed {
        /// The Context to retire.
        context: ContextId,
        /// The Contexts that use it, in order.
        by: Vec<ContextId>,
    },
    /// A Case cites an Item that is neither in its Context's Interfaces nor in those of a Context it uses.
    #[error(
        "the case {case} of the context {context} cites the item {item}, which it cannot reach"
    )]
    ItemNotReached {
        /// The Context that owns the Case.
        context: ContextId,
        /// The Case.
        case: CaseId,
        /// The Item cited.
        item: ItemId,
    },
    /// A Case cites a Clause that is neither in its Context's Contracts nor in those of a Context it uses.
    #[error(
        "the case {case} of the context {context} cites the clause {clause}, which it cannot reach"
    )]
    ClauseNotReached {
        /// The Context that owns the Case.
        context: ContextId,
        /// The Case.
        case: CaseId,
        /// The Clause cited.
        clause: ClauseId,
    },
    /// A sim Harness builds an Interface that is neither in its Context nor in a Context it uses.
    #[error(
        "the sim harness {harness} of the context {context} builds the interface {interface}, which it cannot reach; put the harness again without it, or keep the interface"
    )]
    BuildNotReached {
        /// The Context that owns the Harness.
        context: ContextId,
        /// The Harness.
        harness: HarnessId,
        /// The Interface it builds.
        interface: InterfaceId,
    },
    /// A sim Harness that is put is not the shape the protocol runs, or the Context it is put in has more than one
    /// sim Harness (see [`SimShapeProblem`]).
    #[error("a sim harness is not the shape the protocol runs: {0}")]
    SimShape(SimShapeProblem),
    /// A change set names one thing twice (puts it twice, or puts and retires it): which end state it wants is not
    /// said.
    #[error("the change set names the {what} {id} twice")]
    Repeated {
        /// The kind of thing (`contract`, `interface`, `harness`, or `context` for a Context's settings or its
        /// retirement).
        what: &'static str,
        /// Its id.
        id: Uuid,
    },
    /// A change set breaks several rules; it is not applied. Every problem is here, in the order found, and none is
    /// itself a `Many`.
    #[error("the change set is refused, {} problems: {}", .0.len(), joined(.0))]
    Many(Vec<ProjectError>),
    /// The change breaks a rule of one Context.
    #[error(transparent)]
    Context(#[from] ContextError),
    /// The decision Policy would break a rule of its own.
    #[error(transparent)]
    Policy(#[from] PolicyError),
}

/// The Contexts of one repository. Every change is made on a copy and kept only when the Project still holds:
/// names are unique, every Context it uses is in the Project, `uses` forms no ring, every id of an asset or a
/// definition is given once in the whole Project, every Case cites only Items and Clauses its Context owns or
/// uses, a sim Harness builds only Interfaces its Context owns or uses, and a sim Harness that a change puts has
/// the shape the protocol runs and is the only one in its Context. A change to one Context can break a
/// citation in another (an Item removed from an Interface that another Context's Case cites), so the whole Project
/// is checked each time.
///
/// A change that touches several assets and is coherent only as a whole is one [`ChangeSet`](crate::domain::vo::change_set::ChangeSet), applied by
/// [`Project::apply`]: the rules are checked once, on the state it ends in, not after each asset. A put or a retirement
/// of one asset is a change set of one.
///
/// Its revision is 1 when it is made and goes one up each time a Context is added to it or retired. It
/// does not move when a Context changes: the Context has a revision of its own. So a change request that adds a
/// Context expects the Project to change. The decision Policy it holds has a revision of its own, and does not move
/// the Project's.
///
/// Things are taken out of the live model by retiring them, never by deleting: the Project keeps a [`Retired`]
/// record (names and revisions, not the text) and reserves the id for good. Retiring a Context retires everything
/// it owns; retiring an asset retires its Clauses, Items or Cases (the parts go with the whole). A Clause, an Item
/// or a Case that a change drops from its asset is recorded as retired too. What a Case that stays cites, or what
/// another Context uses, cannot be retired (the peers are refused, not cascaded): retire the one that refers first.
/// What lies outside the Project (Pins, change requests, measurements) refers by id and is not touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    id: ProjectId,
    revision: Revision,
    contexts: BTreeMap<ContextId, Context>,
    policy: Policy,
    retired: Vec<Retired>,
}

impl Project {
    /// A Project with no Contexts and an empty decision Policy, at its first revision.
    pub fn new(id: ProjectId) -> Project {
        Project {
            id,
            revision: Revision::first(),
            contexts: BTreeMap::new(),
            policy: Policy::new(),
            retired: Vec::new(),
        }
    }

    /// A Project as a store kept it, with its revision, its Policy and what it retired. Every rule of the Project
    /// is checked on it, as on a change: a retired id is given to no live thing and to no other retired one.
    ///
    /// The one exception is the shape of a sim Harness ([`ProjectError::SimShape`]). It is a rule of what a change
    /// puts, not of what a store holds: a rule made stricter is held to what is written again and not to what was
    /// kept (a ratchet), because a store that is refused on reading cannot be mended by a put. The sim Harnesses a
    /// store keeps in a shape the protocol does not run are read as they are, and the `sim.shape-kept` check
    /// reports them.
    pub(crate) fn restore(
        id: ProjectId,
        revision: Revision,
        contexts: Vec<Context>,
        policy: Policy,
        retired: Vec<Retired>,
    ) -> Result<Project, ProjectError> {
        let mut map = BTreeMap::new();
        for c in contexts {
            if map.contains_key(&c.id()) {
                return Err(ProjectError::DuplicateId(c.id()));
            }
            map.insert(c.id(), c);
        }
        check(&map, &retired)?;
        Ok(Project {
            id,
            revision,
            contexts: map,
            policy,
            retired,
        })
    }

    /// What the Project retired, in the order it was retired (a whole before its parts).
    pub fn retired(&self) -> &[Retired] {
        &self.retired
    }

    /// The record of `target`, when the Project retired it.
    pub fn retired_of(&self, target: Target) -> Option<&Retired> {
        self.retired.iter().find(|r| r.target() == target)
    }

    /// The decision Policy: the numbered lines that settle a choice without asking a decision maker.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Replaces the decision Policy with `lines`. Its revision goes one up when they differ from the ones it has.
    pub fn put_policy(&mut self, lines: Vec<(PolicyLineId, Prose)>) -> Result<(), ProjectError> {
        self.policy = self.policy.put(lines)?;
        Ok(())
    }

    /// Its id.
    pub fn id(&self) -> ProjectId {
        self.id
    }

    /// How many times its set of Contexts has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The Context with this id.
    pub fn context(&self, id: ContextId) -> Option<&Context> {
        self.contexts.get(&id)
    }

    /// The Context with this name.
    pub fn context_by_name(&self, name: &ContextName) -> Option<&Context> {
        self.contexts.values().find(|c| c.name() == name)
    }

    /// The Contexts, in the order of their ids.
    pub fn contexts(&self) -> impl Iterator<Item = &Context> {
        self.contexts.values()
    }

    /// Adds a Context. The Project goes to its next revision.
    pub fn add_context(&mut self, context: Context) -> Result<(), ProjectError> {
        if self.contexts.contains_key(&context.id()) {
            return Err(ProjectError::DuplicateId(context.id()));
        }
        let mut next = self.contexts.clone();
        next.insert(context.id(), context);
        self.keep(next, Vec::new(), &BTreeSet::new())?;
        self.revision = self.revision.next();
        Ok(())
    }

    /// Retires a Context and everything it owns. The Project goes to its next revision. Refused when another
    /// Context uses it, or when a Case of another Context cites what it owns.
    pub fn retire_context(&mut self, id: ContextId) -> Result<(), ProjectError> {
        let context = self
            .contexts
            .get(&id)
            .ok_or(ProjectError::UnknownContext(id))?;
        let by: Vec<ContextId> = self
            .contexts
            .values()
            .filter(|c| c.uses().contains(&id))
            .map(Context::id)
            .collect();
        if !by.is_empty() {
            return Err(ProjectError::RetireUsed { context: id, by });
        }
        let at = self.revision.next();
        let name = context.name();
        let mut records = vec![Retired::whole(
            Target::Context(id),
            name.clone(),
            context.revision(),
            at,
        )];
        for k in context.contracts() {
            records.extend(contract_records(name, k, at));
        }
        for i in context.interfaces() {
            records.extend(interface_records(name, i, at));
        }
        for h in context.harnesses() {
            records.extend(harness_records(name, h, at));
        }
        let mut next = self.contexts.clone();
        next.remove(&id);
        self.retire(next, records)?;
        self.revision = at;
        Ok(())
    }

    /// Retires a Contract of a Context with its Clauses. The Context goes to its next revision. A Context based on
    /// its Contracts keeps one at least.
    pub fn retire_contract(
        &mut self,
        context: ContextId,
        id: ContractId,
    ) -> Result<(), ProjectError> {
        self.retire_asset(context, |c| c.retire_contract(id).map(Taken::Contract))
    }

    /// Retires an Interface of a Context with its Items. The Context goes to its next revision.
    pub fn retire_interface(
        &mut self,
        context: ContextId,
        id: InterfaceId,
    ) -> Result<(), ProjectError> {
        self.retire_asset(context, |c| c.retire_interface(id).map(Taken::Interface))
    }

    /// Retires a Harness of a Context with its Cases. The Context goes to its next revision.
    pub fn retire_harness(
        &mut self,
        context: ContextId,
        id: HarnessId,
    ) -> Result<(), ProjectError> {
        self.retire_asset(context, |c| c.retire_harness(id).map(Taken::Harness))
    }

    /// Takes an asset out of a copy of the Context with `take`, and keeps the result and the records of what went
    /// when the Project still holds.
    fn retire_asset(
        &mut self,
        context: ContextId,
        take: impl FnOnce(&mut Context) -> Result<Taken, ContextError>,
    ) -> Result<(), ProjectError> {
        let mut next = self.contexts.clone();
        let c = next
            .get_mut(&context)
            .ok_or(ProjectError::UnknownContext(context))?;
        let taken = take(c)?;
        let records = taken.records(c.name(), c.revision());
        self.retire(next, records)
    }

    /// Keeps `next` as the Contexts and `records` as retired, unless Cases that stay cite what `records` take away
    /// or the Project would break a rule.
    fn retire(
        &mut self,
        next: BTreeMap<ContextId, Context>,
        records: Vec<Retired>,
    ) -> Result<(), ProjectError> {
        refuse_cited(&next, &records)?;
        self.keep(next, records, &BTreeSet::new())
    }

    /// Sets the Contexts a Context uses.
    pub fn set_uses(
        &mut self,
        context: ContextId,
        uses: BTreeSet<ContextId>,
    ) -> Result<(), ProjectError> {
        self.change(context, &BTreeSet::new(), |c| c.set_uses(uses))
    }

    /// Bases a Context on its Contracts, of which it must have one. There is no way back.
    pub fn base_on_contract(&mut self, context: ContextId) -> Result<(), ProjectError> {
        self.change(context, &BTreeSet::new(), Context::base_on_contract)
    }

    /// Makes or revises a Contract of a Context.
    pub fn put_contract(
        &mut self,
        context: ContextId,
        id: ContractId,
        title: Title,
        clauses: Vec<(ClauseId, ClauseContent)>,
    ) -> Result<(), ProjectError> {
        self.change(context, &BTreeSet::new(), |c| {
            c.put_contract(id, title, clauses)
        })
    }

    /// Makes or revises an Interface of a Context.
    pub fn put_interface(
        &mut self,
        context: ContextId,
        id: InterfaceId,
        source: Source,
        items: Vec<ItemFound>,
    ) -> Result<(), ProjectError> {
        self.change(context, &BTreeSet::new(), |c| {
            c.put_interface(id, source, items)
        })
    }

    /// Makes or revises a Harness of a Context. A new one is of tests, and an existing one keeps its kind.
    pub fn put_harness(
        &mut self,
        context: ContextId,
        id: HarnessId,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), ProjectError> {
        self.put_harness_as(context, id, None, BTreeSet::new(), source, cases)
    }

    /// Makes or revises a Harness of a Context as one of `kind` (none: the kind it has, or tests for a new one),
    /// which builds `builds` (a sim Harness; the Interfaces must be ones the Context owns or uses). A sim Harness
    /// put is held to the shape the protocol runs ([`ProjectError::SimShape`]).
    pub fn put_harness_as(
        &mut self,
        context: ContextId,
        id: HarnessId,
        kind: Option<HarnessKind>,
        builds: BTreeSet<InterfaceId>,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), ProjectError> {
        self.change(context, &BTreeSet::from([id]), |c| {
            c.put_harness(id, kind, builds, source, cases)
        })
    }

    /// Applies `f` to a copy of the Context and keeps the result when the Project still holds. A Clause, an Item or
    /// a Case that `f` dropped from an asset is recorded as retired. `put` is the Harnesses `f` puts.
    fn change(
        &mut self,
        context: ContextId,
        put: &BTreeSet<HarnessId>,
        f: impl FnOnce(&mut Context) -> Result<(), ContextError>,
    ) -> Result<(), ProjectError> {
        let mut next = self.contexts.clone();
        let c = next
            .get_mut(&context)
            .ok_or(ProjectError::UnknownContext(context))?;
        let before = c.clone();
        f(c)?;
        let dropped = dropped_leaves(&before, c);
        self.keep(next, dropped, put)
    }

    /// Takes `next` as the Contexts, and `added` as retired besides, when they break no rule of the Project. `put`
    /// is the Harnesses the change puts: the shape of a sim Harness is held to what is put and no other (see
    /// [`sim_shape_problems`]).
    fn keep(
        &mut self,
        next: BTreeMap<ContextId, Context>,
        added: Vec<Retired>,
        put: &BTreeSet<HarnessId>,
    ) -> Result<(), ProjectError> {
        let mut retired = self.retired.clone();
        retired.extend(added);
        check(&next, &retired)?;
        let shape = sim_shape_problems(&next, put);
        if !shape.is_empty() {
            return Err(refused(shape));
        }
        self.contexts = next;
        self.retired = retired;
        Ok(())
    }
}

/// An asset taken out of a Context.
enum Taken {
    Contract(Contract),
    Interface(Interface),
    Harness(Harness),
}

impl Taken {
    /// The records of the asset and its parts, retired when the Context was at `at`.
    fn records(&self, context: &ContextName, at: Revision) -> Vec<Retired> {
        match self {
            Taken::Contract(k) => contract_records(context, k, at),
            Taken::Interface(i) => interface_records(context, i, at),
            Taken::Harness(h) => harness_records(context, h, at),
        }
    }
}

/// The records of a Contract and its Clauses, retired when what held them was at `at`.
fn contract_records(context: &ContextName, k: &Contract, at: Revision) -> Vec<Retired> {
    let whole = Retired::whole(Target::Contract(k.id()), context.clone(), k.revision(), at);
    std::iter::once(whole)
        .chain(k.clauses().iter().map(|c| clause_record(context, k, c, at)))
        .collect()
}

fn clause_record(context: &ContextName, k: &Contract, c: &Clause, at: Revision) -> Retired {
    Retired::leaf(
        Target::Clause(c.id()),
        context.clone(),
        k.id().as_uuid(),
        c.content().label().to_string(),
        c.revision(),
        at,
    )
}

/// The records of an Interface and its Items, retired when what held them was at `at`.
fn interface_records(context: &ContextName, i: &Interface, at: Revision) -> Vec<Retired> {
    let whole = Retired::whole(Target::Interface(i.id()), context.clone(), i.revision(), at);
    std::iter::once(whole)
        .chain(i.items().iter().map(|it| item_record(context, i, it, at)))
        .collect()
}

fn item_record(context: &ContextName, i: &Interface, it: &Item, at: Revision) -> Retired {
    Retired::leaf(
        Target::Item(it.id()),
        context.clone(),
        i.id().as_uuid(),
        it.signature().to_string(),
        it.revision(),
        at,
    )
}

/// The records of a Harness and its Cases, retired when what held them was at `at`.
fn harness_records(context: &ContextName, h: &Harness, at: Revision) -> Vec<Retired> {
    let whole = Retired::whole(Target::Harness(h.id()), context.clone(), h.revision(), at);
    std::iter::once(whole)
        .chain(h.cases().iter().map(|c| case_record(context, h, c, at)))
        .collect()
}

fn case_record(context: &ContextName, h: &Harness, c: &Case, at: Revision) -> Retired {
    Retired::leaf(
        Target::Case(c.id()),
        context.clone(),
        h.id().as_uuid(),
        c.name().to_string(),
        c.revision(),
        at,
    )
}

/// The Clauses, Items and Cases the assets of `before` had that the same assets of `after` do not, recorded as
/// retired at the revision their asset has in `after`.
fn dropped_leaves(before: &Context, after: &Context) -> Vec<Retired> {
    let name = after.name();
    let mut out = Vec::new();
    for k in before.contracts() {
        if let Some(now) = after.contract(k.id()) {
            for c in k.clauses().iter().filter(|c| now.clause(c.id()).is_none()) {
                out.push(clause_record(name, k, c, now.revision()));
            }
        }
    }
    for i in before.interfaces() {
        if let Some(now) = after.interface(i.id()) {
            for it in i.items().iter().filter(|it| now.item(it.id()).is_none()) {
                out.push(item_record(name, i, it, now.revision()));
            }
        }
    }
    for h in before.harnesses() {
        if let Some(now) = after.harness(h.id()) {
            for c in h.cases().iter().filter(|c| now.case(c.id()).is_none()) {
                out.push(case_record(name, h, c, now.revision()));
            }
        }
    }
    out
}

/// Refuses when a Case in `next` (a Case that stays) cites an Item or a Clause that `records` retire.
fn refuse_cited(
    next: &BTreeMap<ContextId, Context>,
    records: &[Retired],
) -> Result<(), ProjectError> {
    let going: BTreeSet<Target> = records
        .iter()
        .map(Retired::target)
        .filter(|t| matches!(t, Target::Item(_) | Target::Clause(_)))
        .collect();
    let mut cited = BTreeSet::new();
    let mut cases = Vec::new();
    for case in next
        .values()
        .flat_map(|c| c.harnesses())
        .flat_map(|h| h.cases())
    {
        let cites = case.cites();
        let hit: Vec<Target> = cites
            .items()
            .keys()
            .map(|i| Target::Item(*i))
            .chain(cites.clauses().keys().map(|k| Target::Clause(*k)))
            .filter(|t| going.contains(t))
            .collect();
        if !hit.is_empty() {
            cases.push(case.id());
            cited.extend(hit);
        }
    }
    if cases.is_empty() {
        Ok(())
    } else {
        Err(ProjectError::RetireCited {
            cited: cited.into_iter().collect(),
            cases,
        })
    }
}

/// The error of a refused change: the one problem, or all of them.
pub(crate) fn refused(problems: Vec<ProjectError>) -> ProjectError {
    match problems.as_slice() {
        [only] => only.clone(),
        _ => ProjectError::Many(problems),
    }
}

/// The problems of shape of the sim Harnesses in `put` that `next` has, each once: what each is in its Context
/// (see [`Context::sim_problems`]), which includes how many sim Harnesses the Context has. A Harness in `put`
/// that `next` no longer has, or has as a Harness of tests, has none; a sim Harness not in `put` is not asked, so
/// a shape that was kept before the rule held does not stop a change that does not put it (a ratchet; see
/// [`Project::restore`]).
///
/// Every entry a change makes is in `next`, so a Harness put and another retired in the same change is judged on
/// the state it ends in, as the other rules are.
///
/// A Clause the Context does not reach at all is not foreign here: it is [`ProjectError::ClauseNotReached`], and
/// one citation is refused for one rule.
pub(crate) fn sim_shape_problems(
    next: &BTreeMap<ContextId, Context>,
    put: &BTreeSet<HarnessId>,
) -> Vec<ProjectError> {
    let mut found: Vec<SimShapeProblem> = Vec::new();
    for context in next.values() {
        let reached = |clause: ClauseId| {
            std::iter::once(context)
                .chain(context.uses().iter().filter_map(|u| next.get(u)))
                .any(|c| c.contract_of(clause).is_some())
        };
        for harness in context.harnesses().filter(|h| put.contains(&h.id())) {
            for problem in context.sim_problems(harness) {
                if let SimShapeProblem::ClauseForeign { clause, .. } = problem
                    && !reached(clause)
                {
                    continue;
                }
                if !found.contains(&problem) {
                    found.push(problem);
                }
            }
        }
    }
    found.into_iter().map(ProjectError::SimShape).collect()
}

/// The ids of `ids`, separated by commas, for a message.
fn listed<T: std::fmt::Display>(ids: &[T]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The rules of the Project over `contexts` and what it retired.
fn check(contexts: &BTreeMap<ContextId, Context>, retired: &[Retired]) -> Result<(), ProjectError> {
    check_structure(contexts, retired)?;
    match cite_problems(contexts, &BTreeSet::new()).into_iter().next() {
        Some(first) => Err(first),
        None => Ok(()),
    }
}

/// The rules of the Project other than what the Cases cite: names, uses, and the ids.
fn check_structure(
    contexts: &BTreeMap<ContextId, Context>,
    retired: &[Retired],
) -> Result<(), ProjectError> {
    let mut names = BTreeSet::new();
    for c in contexts.values() {
        if !names.insert(c.name()) {
            return Err(ProjectError::DuplicateName(c.name().clone()));
        }
        if let Some(used) = c.uses().iter().find(|u| !contexts.contains_key(u)) {
            return Err(ProjectError::UnknownUse {
                context: c.id(),
                used: *used,
            });
        }
    }
    if let Some(ring) = find_cycle(contexts) {
        return Err(ProjectError::Cycle(ring));
    }
    check_ids(contexts, retired)
}

/// The messages of `errors`, separated by semicolons.
fn joined(errors: &[ProjectError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Every id of an asset or of a definition in an asset is given once in the whole Project, and no id of a Context,
/// an asset or a definition is one the Project retired: those are reserved. The retired ids are distinct too.
fn check_ids(
    contexts: &BTreeMap<ContextId, Context>,
    retired: &[Retired],
) -> Result<(), ProjectError> {
    let mut seen = BTreeSet::new();
    let mut once = |id: Uuid| {
        if seen.insert(id) {
            Ok(())
        } else {
            Err(ProjectError::IdTaken(id))
        }
    };
    for id in retired.iter().filter_map(|r| r.target().as_uuid()) {
        once(id)?;
    }
    for c in contexts.values() {
        // A Context id is checked against the retired ones only: Context ids and asset ids are kept apart.
        if retired
            .iter()
            .any(|r| r.target().as_uuid() == Some(c.id().as_uuid()))
        {
            return Err(ProjectError::IdTaken(c.id().as_uuid()));
        }
        for k in c.contracts() {
            once(k.id().as_uuid())?;
            for clause in k.clauses() {
                once(clause.id().as_uuid())?;
            }
        }
        for i in c.interfaces() {
            once(i.id().as_uuid())?;
            for item in i.items() {
                once(item.id().as_uuid())?;
            }
        }
        for h in c.harnesses() {
            once(h.id().as_uuid())?;
            for case in h.cases() {
                once(case.id().as_uuid())?;
            }
        }
    }
    Ok(())
}

/// Every Item and Clause a Case of any Context cites that is neither in its Context nor in a Context it uses, one
/// problem for each citation, in the order of the Contexts and Cases. A citation of a target in `skip` is left out:
/// it is reported by the refusal to retire it.
fn cite_problems(
    contexts: &BTreeMap<ContextId, Context>,
    skip: &BTreeSet<Target>,
) -> Vec<ProjectError> {
    let mut out = Vec::new();
    for context in contexts.values() {
        let reach: Vec<&Context> = std::iter::once(context)
            .chain(context.uses().iter().filter_map(|u| contexts.get(u)))
            .collect();
        let has_item = |item: &ItemId| reach.iter().any(|c| c.interface_of(*item).is_some());
        let has_clause = |clause: &ClauseId| reach.iter().any(|c| c.contract_of(*clause).is_some());
        // An Interface a sim Harness builds is not cited, so a retirement does not report it: it is never skipped.
        for harness in context.harnesses() {
            for interface in harness
                .builds()
                .iter()
                .filter(|i| !reach.iter().any(|c| c.interface(**i).is_some()))
            {
                out.push(ProjectError::BuildNotReached {
                    context: context.id(),
                    harness: harness.id(),
                    interface: *interface,
                });
            }
        }
        for case in context.harnesses().flat_map(|h| h.cases()) {
            for item in case.cites().items().keys().filter(|i| !has_item(i)) {
                if !skip.contains(&Target::Item(*item)) {
                    out.push(ProjectError::ItemNotReached {
                        context: context.id(),
                        case: case.id(),
                        item: *item,
                    });
                }
            }
            for clause in case.cites().clauses().keys().filter(|k| !has_clause(k)) {
                if !skip.contains(&Target::Clause(*clause)) {
                    out.push(ProjectError::ClauseNotReached {
                        context: context.id(),
                        case: case.id(),
                        clause: *clause,
                    });
                }
            }
        }
    }
    out
}

/// The ids on one cycle of `uses`, in the order of use, or `None` when there is none. Every id in a `uses` is in
/// `contexts` (checked before).
fn find_cycle(contexts: &BTreeMap<ContextId, Context>) -> Option<Vec<ContextId>> {
    fn visit(
        id: ContextId,
        contexts: &BTreeMap<ContextId, Context>,
        path: &mut Vec<ContextId>,
        done: &mut BTreeSet<ContextId>,
    ) -> Option<Vec<ContextId>> {
        if let Some(at) = path.iter().position(|p| *p == id) {
            return Some(path[at..].to_vec());
        }
        if done.contains(&id) {
            return None;
        }
        path.push(id);
        for next in contexts[&id].uses() {
            if let Some(ring) = visit(*next, contexts, path, done) {
                return Some(ring);
            }
        }
        path.pop();
        done.insert(id);
        None
    }
    let mut done = BTreeSet::new();
    contexts
        .keys()
        .find_map(|id| visit(*id, contexts, &mut Vec::new(), &mut done))
}

#[cfg(test)]
mod fixture {
    //! A Project as a store keeps it, whatever the rules of a put are now.

    use super::*;
    use crate::domain::entity::context::ContextHead;

    impl Project {
        /// This Project with the Harnesses of the Context `context` replaced by `harnesses`, as a store keeps them:
        /// read and not put, so that the shape of a sim Harness is not asked.
        pub(crate) fn with_harnesses_kept(
            &self,
            context: ContextId,
            harnesses: Vec<Harness>,
        ) -> Project {
            let contexts = self
                .contexts
                .values()
                .map(|c| {
                    if c.id() != context {
                        return c.clone();
                    }
                    let head = ContextHead {
                        id: c.id(),
                        revision: c.revision(),
                        name: c.name().clone(),
                        basis: c.basis(),
                        uses: c.uses().clone(),
                    };
                    Context::restore(
                        head,
                        c.contracts().cloned().collect(),
                        c.interfaces().cloned().collect(),
                        harnesses.clone(),
                    )
                    .unwrap()
                })
                .collect();
            Project::restore(
                self.id,
                self.revision,
                contexts,
                self.policy.clone(),
                self.retired.clone(),
            )
            .unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entity::context::Basis;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseLabel, ClauseText};
    use crate::domain::vo::id::test_uuid;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::signature::Signature;

    fn ctx_id(n: u128) -> ContextId {
        ContextId::from_uuid(test_uuid(n))
    }

    fn item(n: u128) -> ItemId {
        ItemId::from_uuid(test_uuid(n))
    }

    fn clause(n: u128) -> ClauseId {
        ClauseId::from_uuid(test_uuid(n))
    }

    fn context(n: u128, name: &str, uses: &[u128]) -> Context {
        Context::new(
            ctx_id(n),
            ContextName::try_from(name).unwrap(),
            uses.iter().map(|u| ctx_id(*u)).collect(),
        )
        .unwrap()
    }

    /// A Project with `core` (1) using `base` (2); `base` has the Interface with Item 20 and the Contract with
    /// Clause 30, and `core` has the Interface with Item 10 and the Contract with Clause 11.
    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(context(2, "base", &[])).unwrap();
        p.add_context(context(1, "core", &[2])).unwrap();
        put_interface(&mut p, 1, 101, 10);
        put_interface(&mut p, 2, 201, 20);
        put_contract(&mut p, 1, 102, 11);
        put_contract(&mut p, 2, 202, 30);
        p
    }

    fn put_interface(p: &mut Project, ctx: u128, id: u128, item_id: u128) {
        p.put_interface(
            ctx_id(ctx),
            InterfaceId::from_uuid(test_uuid(id)),
            Source::new("fn f()\n".to_string()),
            vec![(
                item(item_id),
                Signature::try_from("fn f()").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
    }

    fn put_contract(p: &mut Project, ctx: u128, id: u128, clause_id: u128) {
        p.put_contract(
            ctx_id(ctx),
            ContractId::from_uuid(test_uuid(id)),
            Title::try_from("T").unwrap(),
            vec![(
                clause(clause_id),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("a").unwrap(),
                ),
            )],
        )
        .unwrap();
    }

    /// Puts on `core` a Harness with one Case citing `items` and `clauses`.
    fn put_harness(p: &mut Project, items: &[u128], clauses: &[u128]) -> Result<(), ProjectError> {
        let cites = CaseCites::new(
            items
                .iter()
                .map(|i| (item(*i), Revision::first()))
                .collect(),
            clauses
                .iter()
                .map(|k| (clause(*k), Revision::first()))
                .collect(),
        )
        .unwrap();
        p.put_harness(
            ctx_id(1),
            HarnessId::from_uuid(test_uuid(103)),
            Source::new("fn t() {}\n".to_string()),
            vec![(
                CaseId::from_uuid(test_uuid(50)),
                CaseName::try_from("t").unwrap(),
                LineRange::new(1, 1).unwrap(),
                cites,
            )],
        )
    }

    #[test]
    fn contexts_are_found_by_id_and_by_name() {
        let p = project();
        assert_eq!(p.context(ctx_id(1)).unwrap().name().as_str(), "core");
        let base = ContextName::try_from("base").unwrap();
        assert_eq!(p.context_by_name(&base).unwrap().id(), ctx_id(2));
        assert_eq!(p.contexts().count(), 2);
    }

    #[test]
    fn the_project_goes_to_its_next_revision_when_a_context_is_added_and_not_when_one_changes() {
        let mut p = project();
        // Made at the first revision, and one more for each of the two Contexts added.
        assert_eq!(p.revision().number(), 3);
        p.add_context(context(3, "extra", &[])).unwrap();
        assert_eq!(p.revision().number(), 4);
        // A refused addition leaves it.
        assert!(p.add_context(context(3, "again", &[])).is_err());
        assert_eq!(p.revision().number(), 4);
        // A change inside a Context moves the Context, not the Project.
        let core = p.context(ctx_id(1)).unwrap().revision();
        p.set_uses(ctx_id(1), BTreeSet::new()).unwrap();
        assert_eq!(p.revision().number(), 4);
        assert!(p.context(ctx_id(1)).unwrap().revision() > core);
    }

    #[test]
    fn a_context_goes_to_its_next_revision_for_a_new_asset_and_a_new_use_not_for_a_changed_asset() {
        let mut p = project();
        let rev = |p: &Project| p.context(ctx_id(1)).unwrap().revision().number();
        // `project()` put an Interface and a Contract on `core`, and set its uses when it was made.
        let start = rev(&p);
        // The same Interface again, changed: the asset moves, the Context does not.
        p.put_interface(
            ctx_id(1),
            InterfaceId::from_uuid(test_uuid(101)),
            Source::new("fn g()\n".to_string()),
            vec![(
                item(10),
                Signature::try_from("fn g()").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        assert_eq!(rev(&p), start);
        // A new Harness is a new asset.
        put_harness(&mut p, &[10], &[]).unwrap();
        assert_eq!(rev(&p), start + 1);
        // New uses move it, the same uses again do not, and being based on the Contracts does.
        p.set_uses(ctx_id(1), BTreeSet::new()).unwrap();
        assert_eq!(rev(&p), start + 2);
        p.set_uses(ctx_id(1), BTreeSet::new()).unwrap();
        assert_eq!(rev(&p), start + 2);
        p.base_on_contract(ctx_id(1)).unwrap();
        assert_eq!(rev(&p), start + 3);
    }

    #[test]
    fn a_duplicate_id_or_name_is_refused() {
        let mut p = project();
        assert_eq!(
            p.add_context(context(1, "other", &[])),
            Err(ProjectError::DuplicateId(ctx_id(1)))
        );
        assert_eq!(
            p.add_context(context(3, "core", &[])),
            Err(ProjectError::DuplicateName(
                ContextName::try_from("core").unwrap()
            ))
        );
    }

    #[test]
    fn a_use_of_a_missing_context_or_a_cycle_is_refused_and_changes_nothing() {
        let mut p = project();
        let before = p.clone();
        assert_eq!(
            p.add_context(context(3, "x", &[9])),
            Err(ProjectError::UnknownUse {
                context: ctx_id(3),
                used: ctx_id(9)
            })
        );
        assert!(matches!(
            p.set_uses(ctx_id(2), BTreeSet::from([ctx_id(1)])),
            Err(ProjectError::Cycle(_))
        ));
        assert_eq!(p, before);
    }

    #[test]
    fn a_case_may_cite_its_own_and_its_used_contexts_items_and_clauses() {
        let mut p = project();
        put_harness(&mut p, &[10, 20], &[11, 30]).unwrap();
        assert_eq!(p.context(ctx_id(1)).unwrap().harnesses().count(), 1);
    }

    #[test]
    fn an_id_given_twice_in_the_project_is_refused_and_changes_nothing() {
        let mut p = project();
        let before = p.clone();
        // Interface 201 is `base`'s; `core` cannot have an Interface with that id
        assert_eq!(
            p.put_interface(
                ctx_id(1),
                InterfaceId::from_uuid(test_uuid(201)),
                Source::new("fn g()\n".to_string()),
                Vec::new()
            ),
            Err(ProjectError::IdTaken(test_uuid(201)))
        );
        // Item 20 is in `base`'s Interface; another Interface of `core` cannot declare it
        assert_eq!(
            p.put_interface(
                ctx_id(1),
                InterfaceId::from_uuid(test_uuid(105)),
                Source::new("fn g()\n".to_string()),
                vec![(
                    item(20),
                    Signature::try_from("fn g()").unwrap(),
                    LineRange::new(1, 1).unwrap()
                )]
            ),
            Err(ProjectError::IdTaken(test_uuid(20)))
        );
        assert_eq!(p, before);
    }

    #[test]
    fn a_context_holds_several_interfaces_and_a_case_cites_any_of_them() {
        let mut p = project();
        put_interface(&mut p, 1, 106, 12);
        assert_eq!(p.context(ctx_id(1)).unwrap().interfaces().count(), 2);
        put_harness(&mut p, &[10, 12], &[]).unwrap();
    }

    #[test]
    fn a_case_that_cites_what_it_cannot_reach_is_refused() {
        let mut p = project();
        let core = ctx_id(1);
        let case = CaseId::from_uuid(test_uuid(50));
        assert_eq!(
            put_harness(&mut p, &[99], &[]),
            Err(ProjectError::ItemNotReached {
                context: core,
                case,
                item: item(99)
            })
        );
        assert_eq!(
            put_harness(&mut p, &[10], &[99]),
            Err(ProjectError::ClauseNotReached {
                context: core,
                case,
                clause: clause(99)
            })
        );
        // `base` does not use `core`, so a Case of `base` cannot cite Item 10 of `core`
        let cites = CaseCites::new(
            BTreeMap::from([(item(10), Revision::first())]),
            BTreeMap::new(),
        )
        .unwrap();
        let found = vec![(
            case,
            CaseName::try_from("t").unwrap(),
            LineRange::new(1, 1).unwrap(),
            cites,
        )];
        assert!(matches!(
            p.put_harness(
                ctx_id(2),
                HarnessId::from_uuid(test_uuid(203)),
                Source::new("t\n".to_string()),
                found
            ),
            Err(ProjectError::ItemNotReached { .. })
        ));
    }

    #[test]
    fn a_change_elsewhere_that_would_orphan_a_citation_is_refused_and_changes_nothing() {
        let mut p = project();
        put_harness(&mut p, &[10, 20], &[]).unwrap();
        let before = p.clone();
        // `base` drops Item 20, which a Case of `core` cites
        assert!(matches!(
            p.put_interface(
                ctx_id(2),
                InterfaceId::from_uuid(test_uuid(201)),
                Source::new("x\n".to_string()),
                Vec::new()
            ),
            Err(ProjectError::ItemNotReached { .. })
        ));
        // `core` stops using `base`
        assert!(matches!(
            p.set_uses(ctx_id(1), BTreeSet::new()),
            Err(ProjectError::ItemNotReached { .. })
        ));
        assert_eq!(p, before);
    }

    #[test]
    fn the_policy_is_replaced_whole_and_does_not_move_the_projects_revision() {
        let mut p = project();
        let revision = p.revision();
        assert!(p.policy().lines().is_empty());
        let line = |n: &str, t: &str| {
            (
                PolicyLineId::try_from(n).unwrap(),
                Prose::try_from(t).unwrap(),
            )
        };
        p.put_policy(vec![line("P1", "a"), line("P2", "b")])
            .unwrap();
        assert_eq!(p.policy().lines().len(), 2);
        p.put_policy(vec![line("P2", "b")]).unwrap();
        assert_eq!(p.policy().lines().len(), 1);
        assert_eq!(p.policy().revision().number(), 3);
        assert_eq!(p.revision(), revision);
        // A refused policy changes nothing.
        let before = p.clone();
        assert!(
            p.put_policy(vec![line("P1", "a"), line("P1", "b")])
                .is_err()
        );
        assert_eq!(p, before);
    }

    #[test]
    fn a_context_is_based_on_its_contract_through_the_project() {
        let mut p = project();
        p.base_on_contract(ctx_id(1)).unwrap();
        assert_eq!(p.context(ctx_id(1)).unwrap().basis(), Basis::Contract);
        assert_eq!(
            p.base_on_contract(ctx_id(1)),
            Err(ProjectError::Context(ContextError::AlreadyOnContract))
        );
        assert_eq!(
            p.base_on_contract(ctx_id(9)),
            Err(ProjectError::UnknownContext(ctx_id(9)))
        );
    }

    fn interface_id(n: u128) -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(n))
    }

    fn contract_id(n: u128) -> ContractId {
        ContractId::from_uuid(test_uuid(n))
    }

    fn harness_id(n: u128) -> HarnessId {
        HarnessId::from_uuid(test_uuid(n))
    }

    fn targets(p: &Project) -> Vec<Target> {
        p.retired().iter().map(Retired::target).collect()
    }

    #[test]
    fn retiring_an_asset_retires_its_parts_and_moves_the_context_not_the_project() {
        let mut p = project();
        let project_at = p.revision();
        let core = p.context(ctx_id(1)).unwrap().revision();
        p.retire_interface(ctx_id(1), interface_id(101)).unwrap();
        assert!(
            p.context(ctx_id(1))
                .unwrap()
                .interface(interface_id(101))
                .is_none()
        );
        assert_eq!(p.revision(), project_at);
        let after = p.context(ctx_id(1)).unwrap().revision();
        assert_eq!(after.number(), core.number() + 1);
        // the asset, then its Item, both at the Context's new revision
        assert_eq!(
            targets(&p),
            [Target::Interface(interface_id(101)), Target::Item(item(10))]
        );
        let whole = p.retired_of(Target::Interface(interface_id(101))).unwrap();
        assert_eq!(whole.last_revision(), Revision::first());
        assert_eq!(whole.at(), after);
        assert_eq!(whole.context().as_str(), "core");
        let leaf = p.retired_of(Target::Item(item(10))).unwrap();
        assert_eq!(leaf.holder(), Some(test_uuid(101)));
        assert_eq!(leaf.label(), Some("fn f()"));
        assert_eq!(leaf.at(), after);
        // a Contract and a Harness retire the same way
        put_harness(&mut p, &[20], &[11]).unwrap();
        p.retire_harness(ctx_id(1), harness_id(103)).unwrap();
        p.retire_contract(ctx_id(1), contract_id(102)).unwrap();
        assert_eq!(p.retired().len(), 2 + 2 + 2);
        assert!(
            p.retired_of(Target::Case(CaseId::from_uuid(test_uuid(50))))
                .is_some()
        );
        assert!(p.retired_of(Target::Clause(clause(11))).is_some());
        // an asset the Context does not have, or a Context the Project does not have
        assert_eq!(
            p.retire_harness(ctx_id(1), harness_id(103)),
            Err(ProjectError::Context(ContextError::NoSuchAsset {
                kind: "harness",
                id: test_uuid(103)
            }))
        );
        assert_eq!(
            p.retire_harness(ctx_id(9), harness_id(103)),
            Err(ProjectError::UnknownContext(ctx_id(9)))
        );
    }

    #[test]
    fn retiring_a_context_retires_everything_it_owns_and_moves_the_project() {
        let mut p = project();
        put_harness(&mut p, &[10], &[11]).unwrap();
        // `core` uses `base`; nothing uses `core`
        let before = p.revision();
        p.retire_context(ctx_id(1)).unwrap();
        assert_eq!(p.revision().number(), before.number() + 1);
        assert!(p.context(ctx_id(1)).is_none());
        assert_eq!(
            targets(&p),
            [
                Target::Context(ctx_id(1)),
                Target::Contract(contract_id(102)),
                Target::Clause(clause(11)),
                Target::Interface(interface_id(101)),
                Target::Item(item(10)),
                Target::Harness(harness_id(103)),
                Target::Case(CaseId::from_uuid(test_uuid(50))),
            ]
        );
        // every record is at the Project's new revision, and the Context's last one is the one it had
        assert!(p.retired().iter().all(|r| r.at() == p.revision()));
        let ctx = p.retired_of(Target::Context(ctx_id(1))).unwrap();
        assert_eq!(ctx.context().as_str(), "core");
        assert!(ctx.last_revision().number() > 1);
        assert_eq!(p.context(ctx_id(2)).unwrap().interfaces().count(), 1);
        assert_eq!(
            p.retire_context(ctx_id(1)),
            Err(ProjectError::UnknownContext(ctx_id(1)))
        );
    }

    #[test]
    fn what_a_case_that_stays_cites_cannot_be_retired_in_the_same_or_another_harness_or_context() {
        let mut p = project();
        put_harness(&mut p, &[10, 20], &[11, 30]).unwrap();
        let before = p.clone();
        let case = CaseId::from_uuid(test_uuid(50));
        // an Interface and a Contract of `core`, cited by its Case in Harness 103
        assert_eq!(
            p.retire_interface(ctx_id(1), interface_id(101)),
            Err(ProjectError::RetireCited {
                cited: vec![Target::Item(item(10))],
                cases: vec![case]
            })
        );
        assert_eq!(
            p.retire_contract(ctx_id(1), contract_id(102)),
            Err(ProjectError::RetireCited {
                cited: vec![Target::Clause(clause(11))],
                cases: vec![case]
            })
        );
        // the Interface and the Contract of `base`, cited by a Case of another Context through `uses`
        assert!(matches!(
            p.retire_interface(ctx_id(2), interface_id(201)),
            Err(ProjectError::RetireCited { .. })
        ));
        assert!(matches!(
            p.retire_contract(ctx_id(2), contract_id(202)),
            Err(ProjectError::RetireCited { .. })
        ));
        assert_eq!(p, before);
        // a second Harness of `core`, with a Case that cites the same Item, keeps the Interface too
        let cites = CaseCites::new(
            BTreeMap::from([(item(10), Revision::first())]),
            BTreeMap::new(),
        )
        .unwrap();
        p.put_harness(
            ctx_id(1),
            harness_id(104),
            Source::new("fn u() {}\n".to_string()),
            vec![(
                CaseId::from_uuid(test_uuid(51)),
                CaseName::try_from("u").unwrap(),
                LineRange::new(1, 1).unwrap(),
                cites,
            )],
        )
        .unwrap();
        p.retire_harness(ctx_id(1), harness_id(103)).unwrap();
        assert_eq!(
            p.retire_interface(ctx_id(1), interface_id(101)),
            Err(ProjectError::RetireCited {
                cited: vec![Target::Item(item(10))],
                cases: vec![CaseId::from_uuid(test_uuid(51))]
            })
        );
        // the Harness that cites goes first, then the Interface it cited
        p.retire_harness(ctx_id(1), harness_id(104)).unwrap();
        p.retire_interface(ctx_id(1), interface_id(101)).unwrap();
        // a Case of `core` cites `base`: `base` cannot be retired while `core` uses it, and `core` is not
        // retired because it is used by none
        let mut q = project();
        put_harness(&mut q, &[20], &[]).unwrap();
        assert_eq!(
            q.retire_context(ctx_id(2)),
            Err(ProjectError::RetireUsed {
                context: ctx_id(2),
                by: vec![ctx_id(1)]
            })
        );
        q.retire_context(ctx_id(1)).unwrap();
        q.retire_context(ctx_id(2)).unwrap();
        assert_eq!(q.contexts().count(), 0);
    }

    #[test]
    fn a_context_that_another_uses_is_refused_until_the_use_goes() {
        let mut p = project();
        let before = p.clone();
        assert_eq!(
            p.retire_context(ctx_id(2)),
            Err(ProjectError::RetireUsed {
                context: ctx_id(2),
                by: vec![ctx_id(1)]
            })
        );
        assert_eq!(p, before);
        p.set_uses(ctx_id(1), BTreeSet::new()).unwrap();
        p.retire_context(ctx_id(2)).unwrap();
    }

    #[test]
    fn a_retired_id_is_reserved_for_good() {
        let mut p = project();
        p.retire_interface(ctx_id(1), interface_id(101)).unwrap();
        let before = p.clone();
        // the asset, its Item, and the Context cannot be made again under the same ids
        assert_eq!(
            p.put_interface(
                ctx_id(1),
                interface_id(101),
                Source::new("fn f()\n".to_string()),
                Vec::new()
            ),
            Err(ProjectError::IdTaken(test_uuid(101)))
        );
        assert_eq!(
            p.put_interface(
                ctx_id(1),
                interface_id(105),
                Source::new("fn f()\n".to_string()),
                vec![(
                    item(10),
                    Signature::try_from("fn f()").unwrap(),
                    LineRange::new(1, 1).unwrap()
                )]
            ),
            Err(ProjectError::IdTaken(test_uuid(10)))
        );
        p.retire_context(ctx_id(1)).unwrap();
        assert_eq!(
            p.add_context(context(1, "again", &[])),
            Err(ProjectError::IdTaken(test_uuid(1)))
        );
        // a name is not reserved, only ids: a new Context may take the name `core`
        p.add_context(context(3, "core", &[])).unwrap();
        assert_ne!(p, before);
    }

    #[test]
    fn a_leaf_a_put_drops_is_recorded_as_retired() {
        let mut p = project();
        let at = |p: &Project| {
            p.context(ctx_id(1))
                .unwrap()
                .interface(interface_id(101))
                .unwrap()
                .revision()
        };
        // the Interface is put again without its only Item: the Item is retired at the Interface's new revision
        p.put_interface(
            ctx_id(1),
            interface_id(101),
            Source::new("\n".to_string()),
            Vec::new(),
        )
        .unwrap();
        let r = p.retired_of(Target::Item(item(10))).unwrap();
        assert_eq!(r.holder(), Some(test_uuid(101)));
        assert_eq!(r.label(), Some("fn f()"));
        assert_eq!(r.last_revision(), Revision::first());
        assert_eq!(r.at(), at(&p));
        assert_eq!(p.retired().len(), 1);
        // the same for a Clause and a Case
        put_harness(&mut p, &[20], &[11]).unwrap();
        p.put_harness(
            ctx_id(1),
            harness_id(103),
            Source::new("\n".to_string()),
            Vec::new(),
        )
        .unwrap();
        p.put_contract(
            ctx_id(1),
            contract_id(102),
            Title::try_from("T").unwrap(),
            Vec::new(),
        )
        .unwrap();
        let kinds: Vec<&str> = p.retired().iter().map(|r| r.target().word()).collect();
        assert_eq!(kinds, ["item", "case", "clause"]);
        // an Item that is put again by the same id is not retired
        let mut q = project();
        put_interface(&mut q, 1, 101, 10);
        assert!(q.retired().is_empty());
    }

    #[test]
    fn the_last_contract_of_a_context_based_on_its_contracts_is_not_retired() {
        let mut p = project();
        p.base_on_contract(ctx_id(1)).unwrap();
        assert_eq!(
            p.retire_contract(ctx_id(1), contract_id(102)),
            Err(ProjectError::Context(ContextError::LastContract))
        );
        assert!(p.retired().is_empty());
        // a second Contract lets the first go
        p.put_contract(
            ctx_id(1),
            contract_id(110),
            Title::try_from("U").unwrap(),
            vec![(
                clause(111),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("b").unwrap(),
                ),
            )],
        )
        .unwrap();
        p.retire_contract(ctx_id(1), contract_id(102)).unwrap();
    }

    #[test]
    fn a_stored_project_with_a_live_id_that_is_retired_is_refused() {
        let mut p = project();
        p.retire_interface(ctx_id(1), interface_id(101)).unwrap();
        let contexts: Vec<Context> = p.contexts().cloned().collect();
        let mut live = project();
        let retired = p.retired().to_vec();
        // restored as it is, it holds
        assert!(
            Project::restore(
                p.id(),
                p.revision(),
                contexts,
                p.policy().clone(),
                retired.clone()
            )
            .is_ok()
        );
        // the Interface is live in `live` and retired in `retired`
        live.retire_context(ctx_id(1)).unwrap();
        let all: Vec<Context> = project().contexts().cloned().collect();
        assert_eq!(
            Project::restore(
                p.id(),
                p.revision(),
                all,
                p.policy().clone(),
                retired.clone()
            ),
            Err(ProjectError::IdTaken(test_uuid(101)))
        );
        // the same record twice
        let twice = [retired.clone(), retired].concat();
        assert!(matches!(
            Project::restore(p.id(), p.revision(), Vec::new(), p.policy().clone(), twice),
            Err(ProjectError::IdTaken(_))
        ));
    }

    fn sim_id(n: u128) -> HarnessId {
        HarnessId::from_uuid(test_uuid(n))
    }

    fn sim_case_id(n: u128) -> CaseId {
        CaseId::from_uuid(test_uuid(1000 + n))
    }

    /// A Case of a sim named `name`, on line 1, citing these Clauses and no Item.
    fn sim_case(n: u128, name: &str, clauses: &[u128]) -> CaseFound {
        (
            sim_case_id(n),
            CaseName::try_from(name).unwrap(),
            LineRange::new(1, 1).unwrap(),
            CaseCites::of(
                BTreeMap::new(),
                clauses
                    .iter()
                    .map(|k| (clause(*k), Revision::first()))
                    .collect(),
            ),
        )
    }

    /// Puts the sim Harness `id` of `core`, which builds Interface 101, with these Cases.
    fn put_sim(p: &mut Project, id: u128, cases: Vec<CaseFound>) -> Result<(), ProjectError> {
        p.put_harness_as(
            ctx_id(1),
            sim_id(id),
            Some(HarnessKind::Sim),
            BTreeSet::from([InterfaceId::from_uuid(test_uuid(101))]),
            Source::new("script\n".to_string()),
            cases,
        )
    }

    /// The problem of shape that is the whole of a refusal.
    fn only_shape(err: ProjectError) -> SimShapeProblem {
        match err {
            ProjectError::SimShape(p) => p,
            other => panic!("not a problem of shape: {other:?}"),
        }
    }

    #[test]
    fn a_sim_harness_put_on_its_own_is_held_to_the_shape_the_protocol_runs() {
        let id = sim_id(500);
        let mut p = project();
        let before = p.clone();
        // no sweep
        let err = put_sim(&mut p, 500, vec![sim_case(50, "regress-a", &[11])]).unwrap_err();
        assert_eq!(
            only_shape(err),
            SimShapeProblem::SweepOnce {
                harness: id,
                count: 0,
                extra: None
            }
        );
        // a Clause of the Context it uses is reached, but is not one of its Contracts
        let err = put_sim(&mut p, 500, vec![sim_case(50, "sweep", &[11, 30])]).unwrap_err();
        assert_eq!(
            only_shape(err),
            SimShapeProblem::ClauseForeign {
                harness: id,
                case: sim_case_id(50),
                clause: clause(30)
            }
        );
        // a sim that builds nothing
        let err = p
            .put_harness_as(
                ctx_id(1),
                id,
                Some(HarnessKind::Sim),
                BTreeSet::new(),
                Source::new("script\n".to_string()),
                vec![sim_case(50, "sweep", &[11])],
            )
            .unwrap_err();
        assert_eq!(only_shape(err), SimShapeProblem::BuildsNone { harness: id });
        // several at once are one refusal of all of them
        let err = put_sim(&mut p, 500, vec![sim_case(50, "stray", &[])]).unwrap_err();
        assert!(
            matches!(&err, ProjectError::Many(all) if all.len() == 2),
            "{err:?}"
        );
        assert_eq!(p, before, "a refused put changes nothing");
        // and the shape the protocol runs is kept
        put_sim(&mut p, 500, vec![sim_case(50, "sweep", &[11])]).unwrap();
        assert_eq!(
            p.context(ctx_id(1)).unwrap().harness(id).unwrap().kind(),
            HarnessKind::Sim
        );
        // a Harness of tests is not asked
        put_harness(&mut p, &[10], &[]).unwrap();
    }

    #[test]
    fn a_second_sim_harness_put_on_its_own_is_refused_until_the_first_is_retired() {
        let mut p = project();
        put_sim(&mut p, 500, vec![sim_case(50, "sweep", &[11])]).unwrap();
        let err = put_sim(&mut p, 501, vec![sim_case(51, "sweep", &[11])]).unwrap_err();
        assert_eq!(
            only_shape(err),
            SimShapeProblem::OnePerContext {
                context: ctx_id(1),
                harnesses: vec![sim_id(500), sim_id(501)]
            }
        );
        p.retire_harness(ctx_id(1), sim_id(500)).unwrap();
        put_sim(&mut p, 501, vec![sim_case(51, "sweep", &[11])]).unwrap();
    }

    #[test]
    fn a_shape_a_store_kept_is_read_and_stops_only_a_put_of_that_harness() {
        let stray = Harness::of_kind(
            HarnessKind::Sim,
            sim_id(500),
            BTreeSet::new(),
            Source::new("script\n".to_string()),
            vec![sim_case(50, "stray", &[11])],
        )
        .unwrap();
        // restored: the store is read as it is
        let kept = project().with_harnesses_kept(ctx_id(1), vec![stray]);
        assert_eq!(kept.context(ctx_id(1)).unwrap().harnesses().count(), 1);
        // every other way to change the Project goes through
        let mut p = kept.clone();
        p.set_uses(ctx_id(1), BTreeSet::new()).unwrap();
        put_contract(&mut p, 1, 105, 12);
        put_interface(&mut p, 1, 106, 13);
        p.add_context(context(3, "extra", &[])).unwrap();
        p.retire_interface(ctx_id(1), interface_id(106)).unwrap();
        put_harness(&mut p, &[10], &[]).unwrap();
        // a put of it is held to the shape, whether it is put as a sim or with the kind it has
        let mut p = kept.clone();
        let err = put_sim(&mut p, 500, vec![sim_case(50, "stray", &[11])]).unwrap_err();
        assert!(matches!(err, ProjectError::Many(_)), "{err:?}");
        let err = p
            .put_harness(
                ctx_id(1),
                sim_id(500),
                Source::new("script\n".to_string()),
                vec![sim_case(50, "stray", &[11])],
            )
            .unwrap_err();
        assert!(matches!(err, ProjectError::Many(_)), "{err:?}");
        assert_eq!(p, kept);
        // put in the shape the protocol runs, it is mended
        p.put_harness_as(
            ctx_id(1),
            sim_id(500),
            None,
            BTreeSet::from([InterfaceId::from_uuid(test_uuid(101))]),
            Source::new("script\n".to_string()),
            vec![sim_case(50, "sweep", &[11])],
        )
        .unwrap();
    }
}
