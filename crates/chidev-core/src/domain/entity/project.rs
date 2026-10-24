//! The Project: the root that holds the Contexts of one repository and the rules between them.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use uuid::Uuid;

use super::context::{Context, ContextError};
use super::harness::CaseFound;
use super::interface::ItemFound;
use crate::domain::vo::clause::ClauseContent;
use crate::domain::vo::id::{
    CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId, ProjectId,
};
use crate::domain::vo::name::ContextName;
use crate::domain::vo::policy::{Policy, PolicyError, PolicyLineId};
use crate::domain::vo::prose::Prose;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::source::Source;
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
    /// An asset or a definition would have an id that another one in the Project has. A citation names its
    /// target by id alone, so an id is one thing in the whole Project.
    #[error("the id {0} is given to two things in the project")]
    IdTaken(Uuid),
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
    /// The change breaks a rule of one Context.
    #[error(transparent)]
    Context(#[from] ContextError),
    /// The decision Policy would break a rule of its own.
    #[error(transparent)]
    Policy(#[from] PolicyError),
}

/// The Contexts of one repository. Every change is made on a copy and kept only when the Project still holds:
/// names are unique, every Context it uses is in the Project, `uses` forms no ring, every id of an asset or a
/// definition is given once in the whole Project, and every Case cites only Items and Clauses its Context owns or
/// uses. A change to one Context can break a citation in another (an Item removed from an Interface that another
/// Context's Case cites), so the whole Project is checked each time.
///
/// Its revision is 1 when it is made and goes one up each time a Context is added to it or removed from it. It
/// does not move when a Context changes: the Context has a revision of its own. So a change request that adds a
/// Context expects the Project to change. The decision Policy it holds has a revision of its own, and does not move
/// the Project's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    id: ProjectId,
    revision: Revision,
    contexts: BTreeMap<ContextId, Context>,
    policy: Policy,
}

impl Project {
    /// A Project with no Contexts and an empty decision Policy, at its first revision.
    pub fn new(id: ProjectId) -> Project {
        Project {
            id,
            revision: Revision::first(),
            contexts: BTreeMap::new(),
            policy: Policy::new(),
        }
    }

    /// A Project as a store kept it, with its revision and its Policy. Every rule of the Project is checked on it,
    /// as on a change.
    pub(crate) fn restore(
        id: ProjectId,
        revision: Revision,
        contexts: Vec<Context>,
        policy: Policy,
    ) -> Result<Project, ProjectError> {
        let mut map = BTreeMap::new();
        for c in contexts {
            if map.contains_key(&c.id()) {
                return Err(ProjectError::DuplicateId(c.id()));
            }
            map.insert(c.id(), c);
        }
        check(&map)?;
        Ok(Project {
            id,
            revision,
            contexts: map,
            policy,
        })
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
        self.keep(next)?;
        self.revision = self.revision.next();
        Ok(())
    }

    /// Sets the Contexts a Context uses.
    pub fn set_uses(
        &mut self,
        context: ContextId,
        uses: BTreeSet<ContextId>,
    ) -> Result<(), ProjectError> {
        self.change(context, |c| c.set_uses(uses))
    }

    /// Bases a Context on its Contracts, of which it must have one. There is no way back.
    pub fn base_on_contract(&mut self, context: ContextId) -> Result<(), ProjectError> {
        self.change(context, Context::base_on_contract)
    }

    /// Makes or revises a Contract of a Context.
    pub fn put_contract(
        &mut self,
        context: ContextId,
        id: ContractId,
        title: Title,
        clauses: Vec<(ClauseId, ClauseContent)>,
    ) -> Result<(), ProjectError> {
        self.change(context, |c| c.put_contract(id, title, clauses))
    }

    /// Makes or revises an Interface of a Context.
    pub fn put_interface(
        &mut self,
        context: ContextId,
        id: InterfaceId,
        source: Source,
        items: Vec<ItemFound>,
    ) -> Result<(), ProjectError> {
        self.change(context, |c| c.put_interface(id, source, items))
    }

    /// Makes or revises a Harness of a Context.
    pub fn put_harness(
        &mut self,
        context: ContextId,
        id: HarnessId,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), ProjectError> {
        self.change(context, |c| c.put_harness(id, source, cases))
    }

    /// Applies `f` to a copy of the Context and keeps the result when the Project still holds.
    fn change(
        &mut self,
        context: ContextId,
        f: impl FnOnce(&mut Context) -> Result<(), ContextError>,
    ) -> Result<(), ProjectError> {
        let mut next = self.contexts.clone();
        let c = next
            .get_mut(&context)
            .ok_or(ProjectError::UnknownContext(context))?;
        f(c)?;
        self.keep(next)
    }

    /// Takes `next` as the Contexts when it breaks no rule of the Project.
    fn keep(&mut self, next: BTreeMap<ContextId, Context>) -> Result<(), ProjectError> {
        check(&next)?;
        self.contexts = next;
        Ok(())
    }
}

/// The rules of the Project over `contexts`.
fn check(contexts: &BTreeMap<ContextId, Context>) -> Result<(), ProjectError> {
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
    check_ids(contexts)?;
    for c in contexts.values() {
        check_cites(c, contexts)?;
    }
    Ok(())
}

/// Every id of an asset or of a definition in an asset is given once in the whole Project.
fn check_ids(contexts: &BTreeMap<ContextId, Context>) -> Result<(), ProjectError> {
    let mut seen = BTreeSet::new();
    let mut once = |id: Uuid| {
        if seen.insert(id) {
            Ok(())
        } else {
            Err(ProjectError::IdTaken(id))
        }
    };
    for c in contexts.values() {
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

/// Every Item and Clause a Case of `context` cites is in `context` or in a Context it uses.
fn check_cites(
    context: &Context,
    contexts: &BTreeMap<ContextId, Context>,
) -> Result<(), ProjectError> {
    let reach: Vec<&Context> = std::iter::once(context)
        .chain(context.uses().iter().filter_map(|u| contexts.get(u)))
        .collect();
    let has_item = |item: &ItemId| reach.iter().any(|c| c.interface_of(*item).is_some());
    let has_clause = |clause: &ClauseId| reach.iter().any(|c| c.contract_of(*clause).is_some());
    for case in context.harnesses().flat_map(|h| h.cases()) {
        if let Some(item) = case.cites().items().keys().find(|i| !has_item(i)) {
            return Err(ProjectError::ItemNotReached {
                context: context.id(),
                case: case.id(),
                item: *item,
            });
        }
        if let Some(clause) = case.cites().clauses().keys().find(|k| !has_clause(k)) {
            return Err(ProjectError::ClauseNotReached {
                context: context.id(),
                case: case.id(),
                clause: *clause,
            });
        }
    }
    Ok(())
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
}
