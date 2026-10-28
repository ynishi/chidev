//! `Project::apply`: a change set applied on a copy of the Contexts, the rules of the Project checked once on the
//! state it ends in.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::{
    Project, ProjectError, Taken, check_structure, cite_problems, contract_records, dropped_leaves,
    harness_records, interface_records, refuse_cited, refused, sim_shape_problems,
};
use crate::domain::entity::context::{Context, ContextError};
use crate::domain::entity::harness::{CaseFound, Harness};
use crate::domain::vo::case::CaseCites;
use crate::domain::vo::change_set::{AssetPut, AssetRetire, CaseDraft, ChangeSet};
use crate::domain::vo::frame::Frame;
use crate::domain::vo::id::{ClauseId, ContextId, HarnessId, ItemId};
use crate::domain::vo::retired::Retired;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

impl Project {
    /// Applies a change set: all of it, or nothing.
    ///
    /// The rules of the Project (names, uses, ids, what a Case may cite, what a retirement may take away) are about
    /// the state a change ends in, not about the states on the way, so they are checked once, at the end, on a copy.
    /// A change that is coherent as a whole is therefore never refused for the order of its parts: a Harness can be
    /// put without the Cases that cite a Clause that the same change removes, and a Harness can cite a Clause or an
    /// Item that the same change makes.
    ///
    /// The entries are applied in the order the rules need, whatever order they are listed in: the Contexts
    /// added (the Project goes one revision up for each); the uses of the Contexts; the Contracts and Interfaces; the basis of the Contexts that are based on their Contracts; the
    /// Harnesses (a citation is recorded at the revision its target has by then, or, in a set that carries the frame
    /// of a change request, only where the frame holds it: see below); the retirements. The Clauses, Items
    /// and Cases that a put drops from an asset are recorded as retired, as they are by a put alone. Each asset
    /// moves by one revision at most, because it is named once: a change set that names a thing twice is refused.
    /// A put of some Clauses of a Contract ([`AssetPut::Clauses`]) is a put of that Contract, so it is named once
    /// with it: the Contract composes the list and it goes in as any put of the Contract does.
    ///
    /// A set without a frame (a put alone, or a set for no change request) records every citation of every Case of
    /// a Harness it puts at the revision its target has. A set with one claims a Case current only for what the
    /// request is about: a citation is recorded at the revision its target has when the Case is new, the Case did
    /// not cite that target before, the target is inside the frame, or the Case is; otherwise it stays at the
    /// revision the Case had recorded for it, so a stale Case of another part stays stale. Whether the Case's code
    /// changed does not decide it. A set that itself revises a Clause outside its frame therefore leaves the Cases
    /// that cite it stale.
    ///
    /// When it is refused, nothing changes, and the error names every problem found: a single one as it is, several
    /// as [`ProjectError::Many`]. Problems with the entries themselves (an unknown Context, a Contract that breaks
    /// a rule of its own) are found first; the rules of the Project are only checked when the entries all applied.
    ///
    /// The shape of a sim Harness is asked, on the state the set ends in, of the Harnesses the set puts and of the
    /// Context each is in (a Context has at most one sim Harness): to swap one, put the new and retire the old in
    /// one set. A sim Harness the set does not put is not asked, however it was kept.
    pub fn apply(&mut self, set: ChangeSet) -> Result<(), ProjectError> {
        let repeated = Project::repeated(&set);
        if !repeated.is_empty() {
            return Err(refused(repeated));
        }
        let frame = set.frame().cloned();
        let ChangeSet {
            mut puts, retires, ..
        } = set;
        let added = puts
            .iter()
            .filter(|p| matches!(p, AssetPut::Context(_)))
            .count();
        let put_harnesses: BTreeSet<HarnessId> = puts
            .iter()
            .filter_map(|p| match p {
                AssetPut::Harness { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        puts.sort_by_key(stage);
        let mut problems = Vec::new();
        let mut next = self.contexts.clone();
        let mut touched = BTreeSet::new();
        for put in puts {
            let (context, done) = put_one(&mut next, frame.as_ref(), put);
            touched.insert(context);
            match done {
                Ok(()) => {}
                // A put that has several problems of its own gives them all, each one apart.
                Err(ProjectError::Many(all)) => problems.extend(all),
                Err(e) => problems.push(e),
            }
        }
        let dropped: Vec<Retired> = touched
            .iter()
            .filter_map(|id| Some(dropped_leaves(self.contexts.get(id)?, next.get(id)?)))
            .flatten()
            .collect();

        let mut revision = self.revision;
        for _ in 0..added {
            revision = revision.next();
        }
        let mut records = Vec::new();
        let mut gone = Vec::new();
        for retire in retires {
            match retire_one(&mut next, retire, &mut revision) {
                Ok((taken, context)) => {
                    records.extend(taken);
                    gone.extend(context);
                }
                Err(e) => problems.push(e),
            }
        }
        for id in gone {
            let by: Vec<ContextId> = next
                .values()
                .filter(|c| c.uses().contains(&id))
                .map(Context::id)
                .collect();
            if !by.is_empty() {
                problems.push(ProjectError::RetireUsed { context: id, by });
            }
        }

        let mut retired = self.retired.clone();
        retired.extend(dropped);
        retired.extend(records.iter().cloned());
        if problems.is_empty() {
            if let Err(e) = refuse_cited(&next, &records) {
                problems.push(e);
            }
            match check_structure(&next, &retired) {
                Err(e) => problems.push(e),
                Ok(()) => {
                    let going: BTreeSet<Target> = records.iter().map(Retired::target).collect();
                    problems.extend(cite_problems(&next, &going));
                }
            }
            problems.extend(sim_shape_problems(&next, &put_harnesses));
        }
        if !problems.is_empty() {
            return Err(refused(problems));
        }
        self.contexts = next;
        self.retired = retired;
        self.revision = revision;
        Ok(())
    }

    /// The things `set` names more than once (an asset put twice or put and retired, the settings of a Context
    /// given twice, a Context put into and retired), each as [`ProjectError::Repeated`]. It asks nothing of the
    /// Project: [`Project::apply`] refuses a set that has any, before it applies anything, and a caller that has
    /// found other problems with the entries can add these to say every problem at once.
    pub fn repeated(set: &ChangeSet) -> Vec<ProjectError> {
        repeats(set)
    }
}

/// When a put is applied: settings first, then the definitions, then the Harnesses that cite them.
fn stage(put: &AssetPut) -> u8 {
    match put {
        AssetPut::Context(_) => 0,
        AssetPut::Uses { .. } => 1,
        AssetPut::Contract { .. } | AssetPut::Clauses { .. } | AssetPut::Interface { .. } => 2,
        AssetPut::BaseOnContract { .. } => 3,
        AssetPut::Harness { .. } => 4,
    }
}

/// The Context a put is about, and whether it was put on `next`. `frame` is the frame of the change request the set
/// is applied for, if it is.
fn put_one(
    next: &mut BTreeMap<ContextId, Context>,
    frame: Option<&Frame>,
    put: AssetPut,
) -> (ContextId, Result<(), ProjectError>) {
    match put {
        AssetPut::Context(context) => {
            let id = context.id();
            if next.contains_key(&id) {
                return (id, Err(ProjectError::DuplicateId(id)));
            }
            next.insert(id, context);
            (id, Ok(()))
        }
        AssetPut::Uses { context, uses } => (context, edit(next, context, |c| c.set_uses(uses))),
        AssetPut::BaseOnContract { context } => {
            (context, edit(next, context, Context::base_on_contract))
        }
        AssetPut::Contract {
            context,
            id,
            title,
            clauses,
        } => (
            context,
            edit(next, context, |c| c.put_contract(id, title, clauses)),
        ),
        AssetPut::Clauses {
            context,
            id,
            edits,
            remove,
        } => {
            let done = match next.get_mut(&context) {
                None => Err(ProjectError::UnknownContext(context)),
                Some(c) => c
                    .put_clauses(id, &edits, &remove)
                    .map_err(|all| refused(all.into_iter().map(ProjectError::Context).collect())),
            };
            (context, done)
        }
        AssetPut::Interface {
            context,
            id,
            source,
            items,
        } => (
            context,
            edit(next, context, |c| c.put_interface(id, source, items)),
        ),
        AssetPut::Harness {
            context,
            id,
            kind,
            builds,
            source,
            cases,
        } => {
            let before = next.get(&context).and_then(|c| c.harness(id));
            let found: Vec<CaseFound> = cases
                .into_iter()
                .map(|d| found_of(next, frame, (context, id), before, d))
                .collect();
            let done = edit(next, context, |c| {
                c.put_harness(id, kind, builds, source, found)
            });
            (context, done)
        }
    }
}

/// Applies `f` to the Context `context` of `next`.
fn edit(
    next: &mut BTreeMap<ContextId, Context>,
    context: ContextId,
    f: impl FnOnce(&mut Context) -> Result<(), ContextError>,
) -> Result<(), ProjectError> {
    let c = next
        .get_mut(&context)
        .ok_or(ProjectError::UnknownContext(context))?;
    f(c)?;
    Ok(())
}

/// The Case as the Harness takes it, each citation at the revision its target has in `contexts` now, or, when the
/// set has a `frame`, only where it holds it.
///
/// `at` is the Context and the Harness the Case is put in, and `before` the Harness as `contexts` has it before the
/// put. Without a frame every citation is recorded at the revision its target has. With one, a citation is recorded
/// there when the Case is new (it is not in `before`), the Case did not cite that target before, the target is
/// inside the frame, or the Case is; otherwise at the revision the Case had recorded for it, so a citation that was
/// stale stays stale. The containers are worked out here (the Project, the Context and the Contract, Interface or
/// Harness that hold the target, as `contexts` has them); whether the frame holds the target is [`Frame::contains`].
///
/// A target that is nowhere gets the first revision; the check of the citations finds it unreached.
fn found_of(
    contexts: &BTreeMap<ContextId, Context>,
    frame: Option<&Frame>,
    at: (ContextId, HarnessId),
    before: Option<&Harness>,
    draft: CaseDraft,
) -> CaseFound {
    let old = before.and_then(|h| h.case(draft.id()));
    let case_inside = frame.is_some_and(|f| {
        f.contains(
            Target::Case(draft.id()),
            &[
                Target::Project,
                Target::Context(at.0),
                Target::Harness(at.1),
            ],
        )
    });
    // The revision `target` is recorded at: the one it has now, unless the frame keeps the Case's old citation.
    let recorded = |target: Target, now: Revision, was: Option<Revision>, within: &[Target]| match (
        frame, was,
    ) {
        (Some(f), Some(was)) if !case_inside && !f.contains(target, within) => was,
        _ => now,
    };
    let item = |id: &ItemId| {
        let (now, within) = contexts
            .values()
            .find_map(|c| {
                let interface = c.interface_of(*id)?;
                let now = interface.item(*id)?.revision();
                let within = vec![
                    Target::Project,
                    Target::Context(c.id()),
                    Target::Interface(interface.id()),
                ];
                Some((now, within))
            })
            .unwrap_or_else(|| (Revision::first(), Vec::new()));
        let was = old.and_then(|c| c.cites().items().get(id).copied());
        recorded(Target::Item(*id), now, was, &within)
    };
    let clause = |id: &ClauseId| {
        let (now, within) = contexts
            .values()
            .find_map(|c| {
                let contract = c.contract_of(*id)?;
                let now = contract.clause(*id)?.revision();
                let within = vec![
                    Target::Project,
                    Target::Context(c.id()),
                    Target::Contract(contract.id()),
                ];
                Some((now, within))
            })
            .unwrap_or_else(|| (Revision::first(), Vec::new()));
        let was = old.and_then(|c| c.cites().clauses().get(id).copied());
        recorded(Target::Clause(*id), now, was, &within)
    };
    let cites = CaseCites::of(
        draft.items().iter().map(|i| (*i, item(i))).collect(),
        draft.clauses().iter().map(|k| (*k, clause(k))).collect(),
    );
    (draft.id(), draft.name().clone(), draft.lines(), cites)
}

/// Retires one thing out of `next`: the records of it and of its parts, and the Context when it is one. A Context
/// takes the Project to the next of `revision`.
fn retire_one(
    next: &mut BTreeMap<ContextId, Context>,
    retire: AssetRetire,
    revision: &mut Revision,
) -> Result<(Vec<Retired>, Option<ContextId>), ProjectError> {
    match retire {
        AssetRetire::Context(id) => {
            let context = next.remove(&id).ok_or(ProjectError::UnknownContext(id))?;
            *revision = revision.next();
            let at = *revision;
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
            Ok((records, Some(id)))
        }
        AssetRetire::Contract { context, id } => take(next, context, |c| {
            c.retire_contract(id).map(Taken::Contract)
        }),
        AssetRetire::Interface { context, id } => take(next, context, |c| {
            c.retire_interface(id).map(Taken::Interface)
        }),
        AssetRetire::Harness { context, id } => {
            take(next, context, |c| c.retire_harness(id).map(Taken::Harness))
        }
    }
}

/// Takes an asset out of the Context `context` of `next` with `take`, and gives the records of what went.
fn take(
    next: &mut BTreeMap<ContextId, Context>,
    context: ContextId,
    take: impl FnOnce(&mut Context) -> Result<Taken, ContextError>,
) -> Result<(Vec<Retired>, Option<ContextId>), ProjectError> {
    let c = next
        .get_mut(&context)
        .ok_or(ProjectError::UnknownContext(context))?;
    let taken = take(c)?;
    Ok((taken.records(c.name(), c.revision()), None))
}

/// The things a change set names more than once: an asset put twice or put and retired, the settings of a Context
/// given twice, a Context put into and retired.
fn repeats(set: &ChangeSet) -> Vec<ProjectError> {
    let mut seen: BTreeSet<(&'static str, Uuid)> = BTreeSet::new();
    let mut out = Vec::new();
    let mut note = |key: &'static str, what: &'static str, id: Uuid| {
        if !seen.insert((key, id)) {
            out.push(ProjectError::Repeated { what, id });
        }
    };
    let retiring: BTreeSet<ContextId> = set
        .retires
        .iter()
        .filter_map(|r| match r {
            AssetRetire::Context(id) => Some(*id),
            _ => None,
        })
        .collect();
    for put in &set.puts {
        match put {
            AssetPut::Context(c) => note("context", "context", c.id().as_uuid()),
            AssetPut::Uses { context, .. } => note("uses", "context", context.as_uuid()),
            AssetPut::BaseOnContract { context } => note("base", "context", context.as_uuid()),
            // A partial put of a Contract is a put of it: it is named once, with a put of the whole.
            AssetPut::Contract { id, .. } | AssetPut::Clauses { id, .. } => {
                note("contract", "contract", id.as_uuid())
            }
            AssetPut::Interface { id, .. } => note("interface", "interface", id.as_uuid()),
            AssetPut::Harness { id, .. } => note("harness", "harness", id.as_uuid()),
        }
        let context = match put {
            AssetPut::Context(c) => c.id(),
            AssetPut::Uses { context, .. }
            | AssetPut::BaseOnContract { context }
            | AssetPut::Contract { context, .. }
            | AssetPut::Clauses { context, .. }
            | AssetPut::Interface { context, .. }
            | AssetPut::Harness { context, .. } => *context,
        };
        if retiring.contains(&context) {
            note("retired-context", "context", context.as_uuid());
        }
    }
    for retire in &set.retires {
        match retire {
            AssetRetire::Context(id) => note("retire", "context", id.as_uuid()),
            AssetRetire::Contract { context, id } => {
                note("contract", "contract", id.as_uuid());
                if retiring.contains(context) {
                    note("retired-context", "context", context.as_uuid());
                }
            }
            AssetRetire::Interface { context, id } => {
                note("interface", "interface", id.as_uuid());
                if retiring.contains(context) {
                    note("retired-context", "context", context.as_uuid());
                }
            }
            AssetRetire::Harness { context, id } => {
                note("harness", "harness", id.as_uuid());
                if retiring.contains(context) {
                    note("retired-context", "context", context.as_uuid());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entity::contract::ContractError;
    use crate::domain::entity::harness::{CaseNameFault, Harness, SimShapeProblem};
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::clause_edit::{ClauseEdit, ClauseNameProblem, NameField};
    use crate::domain::vo::harness_kind::HarnessKind;
    use crate::domain::vo::id::{CaseId, ContractId, HarnessId, InterfaceId, ProjectId, test_uuid};
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::title::Title;

    fn ctx() -> ContextId {
        ContextId::from_uuid(test_uuid(1))
    }

    fn item(n: u128) -> ItemId {
        ItemId::from_uuid(test_uuid(n))
    }

    fn clause(n: u128) -> ClauseId {
        ClauseId::from_uuid(test_uuid(n))
    }

    fn contract_id() -> ContractId {
        ContractId::from_uuid(test_uuid(102))
    }

    fn interface_id() -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(101))
    }

    fn harness_id() -> HarnessId {
        HarnessId::from_uuid(test_uuid(103))
    }

    fn clauses(ids: &[(u128, &str)]) -> Vec<(ClauseId, ClauseContent)> {
        ids.iter()
            .map(|(n, label)| {
                (
                    clause(*n),
                    ClauseContent::new(
                        ClauseLabel::try_from(*label).unwrap(),
                        ClauseText::try_from("a").unwrap(),
                    ),
                )
            })
            .collect()
    }

    fn items(ids: &[(u128, &str)]) -> Vec<(ItemId, ItemKey, LineRange)> {
        ids.iter()
            .enumerate()
            .map(|(k, (n, key))| {
                (
                    item(*n),
                    ItemKey::try_from(*key).unwrap(),
                    LineRange::new(k + 1, k + 1).unwrap(),
                )
            })
            .collect()
    }

    fn draft(case: u128, items: &[u128], clauses: &[u128]) -> CaseDraft {
        CaseDraft::new(
            CaseId::from_uuid(test_uuid(case)),
            CaseName::try_from(format!("case{case}").as_str()).unwrap(),
            LineRange::new(1, 1).unwrap(),
            items.iter().map(|i| item(*i)).collect(),
            clauses.iter().map(|k| clause(*k)).collect(),
        )
    }

    fn contract(ids: &[(u128, &str)]) -> AssetPut {
        AssetPut::Contract {
            context: ctx(),
            id: contract_id(),
            title: Title::try_from("T").unwrap(),
            clauses: clauses(ids),
        }
    }

    fn interface(ids: &[(u128, &str)]) -> AssetPut {
        AssetPut::Interface {
            context: ctx(),
            id: interface_id(),
            source: Source::new("fn a()\nfn release()\n".to_string()),
            items: items(ids),
        }
    }

    fn harness(cases: Vec<CaseDraft>) -> AssetPut {
        AssetPut::Harness {
            context: ctx(),
            id: harness_id(),
            kind: None,
            builds: BTreeSet::new(),
            source: Source::new("fn t() {}\n".to_string()),
            cases,
        }
    }

    /// A sim Harness `sim` that builds `builds`, its sweep citing the Clauses `clauses`.
    fn sim(builds: &[InterfaceId], clauses: &[u128]) -> AssetPut {
        AssetPut::Harness {
            context: ctx(),
            id: HarnessId::from_uuid(test_uuid(106)),
            kind: Some(HarnessKind::Sim),
            builds: builds.iter().copied().collect(),
            source: Source::new("fn s() {}\n".to_string()),
            cases: vec![sweep(70, clauses)],
        }
    }

    /// A Case named `sweep`, citing the Clauses `clauses` and no Item.
    fn sweep(case: u128, clauses: &[u128]) -> CaseDraft {
        CaseDraft::new(
            CaseId::from_uuid(test_uuid(case)),
            CaseName::try_from("sweep").unwrap(),
            LineRange::new(1, 1).unwrap(),
            BTreeSet::new(),
            clauses.iter().map(|k| clause(*k)).collect(),
        )
    }

    /// A Project with `core`: the Interface has Items 10 (`fn a()`) and 11 (`fn release()`), the Contract Clauses 20
    /// (C1) and 21 (C7), and the Harness two Cases, 50 citing Item 10 and Clause 20, 51 citing Item 11 and Clause 21.
    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                ctx(),
                ContextName::try_from("core").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        p.apply(ChangeSet::new(
            vec![
                contract(&[(20, "C1"), (21, "C7")]),
                interface(&[(10, "fn a()"), (11, "fn release()")]),
                harness(vec![draft(50, &[10], &[20]), draft(51, &[11], &[21])]),
            ],
            Vec::new(),
        ))
        .unwrap();
        p
    }

    fn revisions(p: &Project) -> (u64, u64, u64, u64) {
        let c = p.context(ctx()).unwrap();
        (
            c.revision().number(),
            c.contract(contract_id()).unwrap().revision().number(),
            c.interface(interface_id()).unwrap().revision().number(),
            c.harness(harness_id()).unwrap().revision().number(),
        )
    }

    /// The change that removes Clause 21 and Item 11 and puts the Harness without the Case that cites them.
    fn removal() -> Vec<AssetPut> {
        vec![
            contract(&[(20, "C1")]),
            interface(&[(10, "fn a()")]),
            harness(vec![draft(50, &[10], &[20])]),
        ]
    }

    #[test]
    fn what_cases_cite_and_the_change_removes_goes_in_one_apply_with_the_harness_put_without_them()
    {
        let mut p = project();
        assert_eq!(revisions(&p), (4, 1, 1, 1));
        p.apply(ChangeSet::new(removal(), Vec::new())).unwrap();
        let c = p.context(ctx()).unwrap();
        assert!(
            c.contract(contract_id())
                .unwrap()
                .clause(clause(21))
                .is_none()
        );
        assert!(
            c.interface(interface_id())
                .unwrap()
                .item(item(11))
                .is_none()
        );
        assert!(
            c.harness(harness_id())
                .unwrap()
                .case(CaseId::from_uuid(test_uuid(51)))
                .is_none()
        );
        // each asset moved once, the Context not at all (no asset was added)
        assert_eq!(revisions(&p), (4, 2, 2, 2));
        // what the puts dropped is on record, as a put alone records it
        let gone: Vec<Target> = p.retired().iter().map(Retired::target).collect();
        assert_eq!(
            gone,
            [
                Target::Clause(clause(21)),
                Target::Item(item(11)),
                Target::Case(CaseId::from_uuid(test_uuid(51))),
            ]
        );
    }

    #[test]
    fn the_order_the_entries_are_listed_in_does_not_matter() {
        let mut p = project();
        let mut puts = removal();
        puts.reverse();
        p.apply(ChangeSet::new(puts, Vec::new())).unwrap();
        assert_eq!(revisions(&p), (4, 2, 2, 2));
    }

    #[test]
    fn the_same_removal_as_separate_puts_is_refused_at_the_first_step() {
        let mut p = project();
        let before = p.clone();
        for first in [contract(&[(20, "C1")]), interface(&[(10, "fn a()")])] {
            let err = p
                .apply(ChangeSet::new(vec![first], Vec::new()))
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    ProjectError::ClauseNotReached { .. } | ProjectError::ItemNotReached { .. }
                ),
                "{err:?}"
            );
            assert_eq!(p, before);
        }
        // the Harness first is accepted, then the definitions: the order the single puts force
        p.apply(ChangeSet::new(
            vec![harness(vec![draft(50, &[10], &[20])])],
            Vec::new(),
        ))
        .unwrap();
        p.apply(ChangeSet::new(vec![contract(&[(20, "C1")])], Vec::new()))
            .unwrap();
        p.apply(ChangeSet::new(
            vec![interface(&[(10, "fn a()")])],
            Vec::new(),
        ))
        .unwrap();
    }

    #[test]
    fn a_change_set_that_ends_with_a_dangling_citation_is_refused_naming_each_and_changes_nothing()
    {
        let mut p = project();
        let before = p.clone();
        // the Contract and the Interface drop what Case 51 cites, and the Harness is left as it is
        let err = p
            .apply(ChangeSet::new(
                vec![contract(&[(20, "C1")]), interface(&[(10, "fn a()")])],
                Vec::new(),
            ))
            .unwrap_err();
        let ProjectError::Many(all) = err else {
            panic!("several problems expected");
        };
        let case = CaseId::from_uuid(test_uuid(51));
        assert_eq!(
            all,
            [
                ProjectError::ItemNotReached {
                    context: ctx(),
                    case,
                    item: item(11)
                },
                ProjectError::ClauseNotReached {
                    context: ctx(),
                    case,
                    clause: clause(21)
                },
            ]
        );
        assert_eq!(p, before);
    }

    #[test]
    fn retiring_and_putting_are_one_change_set() {
        let mut p = project();
        // the Harness goes, and the Contract and the Interface drop what only it cited
        p.apply(ChangeSet::new(
            vec![contract(&[(20, "C1")]), interface(&[(10, "fn a()")])],
            vec![AssetRetire::Harness {
                context: ctx(),
                id: harness_id(),
            }],
        ))
        .unwrap();
        let c = p.context(ctx()).unwrap();
        assert!(c.harness(harness_id()).is_none());
        assert_eq!(c.contract(contract_id()).unwrap().clauses().len(), 1);
        let gone: Vec<Target> = p.retired().iter().map(Retired::target).collect();
        assert_eq!(
            gone,
            [
                Target::Clause(clause(21)),
                Target::Item(item(11)),
                Target::Harness(harness_id()),
                Target::Case(CaseId::from_uuid(test_uuid(50))),
                Target::Case(CaseId::from_uuid(test_uuid(51))),
            ]
        );
    }

    #[test]
    fn a_retirement_that_a_case_which_stays_stops_is_refused_with_the_case_named() {
        let mut p = project();
        let before = p.clone();
        let err = p
            .apply(ChangeSet::new(
                Vec::new(),
                vec![AssetRetire::Interface {
                    context: ctx(),
                    id: interface_id(),
                }],
            ))
            .unwrap_err();
        assert!(
            matches!(&err, ProjectError::RetireCited { cases, .. } if cases.len() == 2),
            "{err:?}"
        );
        assert_eq!(p, before);
    }

    #[test]
    fn a_new_harness_cites_what_the_same_change_makes_at_the_revision_it_has_there() {
        let mut p = project();
        let extra_contract = AssetPut::Contract {
            context: ctx(),
            id: ContractId::from_uuid(test_uuid(104)),
            title: Title::try_from("N").unwrap(),
            clauses: clauses(&[(40, "C9")]),
        };
        let extra_harness = AssetPut::Harness {
            context: ctx(),
            id: HarnessId::from_uuid(test_uuid(105)),
            kind: None,
            builds: BTreeSet::new(),
            source: Source::new("fn n() {}\n".to_string()),
            cases: vec![draft(60, &[10], &[40])],
        };
        p.apply(ChangeSet::new(
            vec![extra_harness, extra_contract],
            Vec::new(),
        ))
        .unwrap();
        let h = p
            .context(ctx())
            .unwrap()
            .harness(HarnessId::from_uuid(test_uuid(105)))
            .unwrap();
        let cites = h.cases()[0].cites();
        assert_eq!(cites.clauses().get(&clause(40)), Some(&Revision::first()));
        // two assets added to the Context
        assert_eq!(p.context(ctx()).unwrap().revision().number(), 6);
    }

    #[test]
    fn a_sim_harness_builds_an_interface_the_same_change_makes_and_loses_one_the_change_retires() {
        let mut p = project();
        // a sim cites a Clause and builds the Interface; it cites no Item, so no Case of it is on any Pin's basis
        p.apply(ChangeSet::new(
            vec![sim(&[interface_id()], &[20])],
            Vec::new(),
        ))
        .unwrap();
        let h = p
            .context(ctx())
            .unwrap()
            .harness(HarnessId::from_uuid(test_uuid(106)))
            .unwrap();
        assert_eq!(h.kind(), HarnessKind::Sim);
        assert_eq!(h.builds(), &BTreeSet::from([interface_id()]));
        assert!(h.cases()[0].cites().items().is_empty());

        // an Interface the Project does not have is refused, named
        let ghost = InterfaceId::from_uuid(test_uuid(900));
        let mut q = project();
        let err = q
            .apply(ChangeSet::new(vec![sim(&[ghost], &[20])], Vec::new()))
            .unwrap_err();
        assert_eq!(
            err,
            ProjectError::BuildNotReached {
                context: ctx(),
                harness: HarnessId::from_uuid(test_uuid(106)),
                interface: ghost
            }
        );

        // an Interface made in the same change is reached, whatever the order
        let mut r = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        r.add_context(
            Context::new(
                ctx(),
                ContextName::try_from("core").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        r.apply(ChangeSet::new(
            vec![
                sim(&[interface_id()], &[20]),
                interface(&[(10, "fn a()")]),
                contract(&[(20, "C1")]),
            ],
            Vec::new(),
        ))
        .unwrap();

        // retiring the Interface while the sim builds it is refused, and nothing changes; retiring both is not
        let before = p.clone();
        let retire = AssetRetire::Interface {
            context: ctx(),
            id: interface_id(),
        };
        let err = p
            .apply(ChangeSet::new(
                Vec::new(),
                vec![
                    retire.clone(),
                    AssetRetire::Harness {
                        context: ctx(),
                        id: harness_id(),
                    },
                ],
            ))
            .unwrap_err();
        assert!(
            matches!(err, ProjectError::BuildNotReached { .. }),
            "{err:?}"
        );
        assert_eq!(p, before);
        p.apply(ChangeSet::new(
            Vec::new(),
            vec![
                retire,
                AssetRetire::Harness {
                    context: ctx(),
                    id: harness_id(),
                },
                AssetRetire::Harness {
                    context: ctx(),
                    id: HarnessId::from_uuid(test_uuid(106)),
                },
            ],
        ))
        .unwrap();
    }

    #[test]
    fn a_thing_named_twice_is_refused() {
        let mut p = project();
        let before = p.clone();
        let twice = p
            .apply(ChangeSet::new(
                vec![contract(&[(20, "C1")]), contract(&[(20, "C1")])],
                Vec::new(),
            ))
            .unwrap_err();
        assert_eq!(
            twice,
            ProjectError::Repeated {
                what: "contract",
                id: contract_id().as_uuid()
            }
        );
        let both = p
            .apply(ChangeSet::new(
                vec![interface(&[(10, "fn a()")])],
                vec![AssetRetire::Interface {
                    context: ctx(),
                    id: interface_id(),
                }],
            ))
            .unwrap_err();
        assert!(matches!(
            both,
            ProjectError::Repeated {
                what: "interface",
                ..
            }
        ));
        assert_eq!(p, before);
    }

    /// An edit of the Clause `label` to say `text`, with `new` as the id it takes if it makes a Clause.
    fn clause_edit(label: &str, text: &str, replaces: Option<&str>, new: u128) -> ClauseEdit {
        ClauseEdit::new(
            ClauseLabel::try_from(label).unwrap(),
            ClauseText::try_from(text).unwrap(),
            replaces.map(|r| ClauseLabel::try_from(r).unwrap()),
            clause(new),
        )
    }

    /// A partial put of the Contract of `project()`.
    fn clauses_put(edits: Vec<ClauseEdit>, remove: &[&str]) -> AssetPut {
        AssetPut::Clauses {
            context: ctx(),
            id: contract_id(),
            edits,
            remove: remove
                .iter()
                .map(|l| ClauseLabel::try_from(*l).unwrap())
                .collect(),
        }
    }

    #[test]
    fn a_clauses_put_changes_the_clauses_it_names_and_keeps_the_rest_and_the_title() {
        let mut p = project();
        p.apply(ChangeSet::new(
            vec![clauses_put(
                vec![
                    clause_edit("C1", "changed", None, 80),
                    clause_edit("C2", "new", None, 90),
                ],
                &[],
            )],
            Vec::new(),
        ))
        .unwrap();
        let k = p.context(ctx()).unwrap().contract(contract_id()).unwrap();
        assert_eq!(k.title().as_str(), "T");
        let seen: Vec<_> = k
            .clauses()
            .iter()
            .map(|c| {
                (
                    c.id(),
                    c.label().to_string(),
                    c.text().as_str().to_string(),
                    c.revision().number(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (clause(20), "C1".to_string(), "changed".to_string(), 2),
                (clause(21), "C7".to_string(), "a".to_string(), 1),
                (clause(90), "C2".to_string(), "new".to_string(), 1),
            ]
        );
        assert_eq!(k.revision().number(), 2);
    }

    #[test]
    fn a_clauses_put_can_remove_a_clause_no_case_cites() {
        let mut p = project();
        p.apply(ChangeSet::new(
            vec![
                clauses_put(vec![clause_edit("C2", "new", None, 90)], &[]),
                harness(vec![draft(50, &[10], &[20])]),
            ],
            Vec::new(),
        ))
        .unwrap();
        p.apply(ChangeSet::new(
            vec![clauses_put(Vec::new(), &["C7"])],
            Vec::new(),
        ))
        .unwrap();
        let k = p.context(ctx()).unwrap().contract(contract_id()).unwrap();
        let labels: Vec<_> = k.clauses().iter().map(|c| c.label().to_string()).collect();
        assert_eq!(labels, ["C1", "C2"]);
        assert!(p.retired_of(Target::Clause(clause(21))).is_some());
    }

    #[test]
    fn a_clauses_put_with_a_put_of_the_same_contract_or_another_clauses_put_is_named_twice() {
        let mut p = project();
        let before = p.clone();
        let repeated = ProjectError::Repeated {
            what: "contract",
            id: contract_id().as_uuid(),
        };
        let edits = || vec![clause_edit("C1", "changed", None, 80)];
        for puts in [
            vec![
                contract(&[(20, "C1"), (21, "C7")]),
                clauses_put(edits(), &[]),
            ],
            vec![
                clauses_put(edits(), &[]),
                contract(&[(20, "C1"), (21, "C7")]),
            ],
            vec![clauses_put(edits(), &[]), clauses_put(edits(), &[])],
        ] {
            assert_eq!(
                p.apply(ChangeSet::new(puts, Vec::new())).unwrap_err(),
                repeated
            );
        }
        assert_eq!(p, before);
    }

    #[test]
    fn a_clauses_put_of_a_contract_the_context_does_not_have_or_with_wrong_names_changes_nothing() {
        let mut p = project();
        let before = p.clone();
        let missing = AssetPut::Clauses {
            context: ctx(),
            id: ContractId::from_uuid(test_uuid(999)),
            edits: Vec::new(),
            remove: Vec::new(),
        };
        assert!(matches!(
            p.apply(ChangeSet::new(vec![missing], Vec::new())),
            Err(ProjectError::Context(ContextError::NoSuchAsset {
                kind: "contract",
                ..
            }))
        ));
        let wrong = clauses_put(vec![clause_edit("C3", "x", Some("C9"), 90)], &["C8"]);
        let err = p
            .apply(ChangeSet::new(vec![wrong], Vec::new()))
            .unwrap_err();
        let c = |l: &str| ClauseLabel::try_from(l).unwrap();
        let said = |problem| {
            ProjectError::Context(ContextError::Contract(ContractError::ClauseName {
                contract: contract_id(),
                problem,
            }))
        };
        assert_eq!(
            err,
            ProjectError::Many(vec![
                said(ClauseNameProblem::Unknown {
                    label: c("C9"),
                    field: NameField::Replaces
                }),
                said(ClauseNameProblem::Unknown {
                    label: c("C8"),
                    field: NameField::Remove
                }),
            ])
        );
        assert_eq!(p, before);
    }

    #[test]
    fn a_context_is_retired_with_what_it_owns_and_the_project_moves_once_for_it() {
        let mut p = project();
        let at = p.revision().number();
        p.apply(ChangeSet::new(
            Vec::new(),
            vec![AssetRetire::Context(ctx())],
        ))
        .unwrap();
        assert!(p.context(ctx()).is_none());
        assert_eq!(p.revision().number(), at + 1);
        assert!(p.retired_of(Target::Context(ctx())).is_some());
        assert!(
            p.retired_of(Target::Case(CaseId::from_uuid(test_uuid(51))))
                .is_some()
        );
    }

    #[test]
    fn an_unknown_context_is_refused() {
        let mut p = project();
        let missing = ContextId::from_uuid(test_uuid(9));
        let err = p
            .apply(ChangeSet::new(
                vec![AssetPut::BaseOnContract { context: missing }],
                Vec::new(),
            ))
            .unwrap_err();
        assert_eq!(err, ProjectError::UnknownContext(missing));
    }

    fn sim_id(n: u128) -> HarnessId {
        HarnessId::from_uuid(test_uuid(n))
    }

    /// A Case named `name`, citing the Clauses `clauses` and no Item.
    fn named(case: u128, name: &str, clauses: &[u128]) -> CaseDraft {
        CaseDraft::new(
            CaseId::from_uuid(test_uuid(case)),
            CaseName::try_from(name).unwrap(),
            LineRange::new(1, 1).unwrap(),
            BTreeSet::new(),
            clauses.iter().map(|k| clause(*k)).collect(),
        )
    }

    /// A sim Harness `id` that builds `builds`, with these Cases.
    fn sim_put(id: u128, builds: &[InterfaceId], cases: Vec<CaseDraft>) -> AssetPut {
        AssetPut::Harness {
            context: ctx(),
            id: sim_id(id),
            kind: Some(HarnessKind::Sim),
            builds: builds.iter().copied().collect(),
            source: Source::new("fn s() {}\n".to_string()),
            cases,
        }
    }

    /// The problems of shape a refusal is made of.
    fn shape_of(err: ProjectError) -> Vec<SimShapeProblem> {
        let all = match err {
            ProjectError::Many(all) => all,
            one => vec![one],
        };
        all.into_iter()
            .map(|e| match e {
                ProjectError::SimShape(p) => p,
                other => panic!("not a problem of shape: {other:?}"),
            })
            .collect()
    }

    /// `p` with these sim Harnesses besides the ones it has, as a store keeps them: read, not put.
    fn with_kept(p: &Project, kept: Vec<Harness>) -> Project {
        let mut all: Vec<Harness> = p.context(ctx()).unwrap().harnesses().cloned().collect();
        all.extend(kept);
        p.with_harnesses_kept(ctx(), all)
    }

    /// A sim Harness `id` as a store kept it, whatever its shape: it builds Interface 101, and its Cases are
    /// `(id, name, Clauses cited)`.
    fn kept_sim(id: u128, cases: &[(u128, &str, &[u128])]) -> Harness {
        Harness::of_kind(
            HarnessKind::Sim,
            sim_id(id),
            BTreeSet::from([interface_id()]),
            Source::new("fn s() {}\n".to_string()),
            cases
                .iter()
                .map(|(n, name, cited)| {
                    (
                        CaseId::from_uuid(test_uuid(*n)),
                        CaseName::try_from(*name).unwrap(),
                        LineRange::new(1, 1).unwrap(),
                        CaseCites::of(
                            BTreeMap::new(),
                            cited
                                .iter()
                                .map(|k| (clause(*k), Revision::first()))
                                .collect(),
                        ),
                    )
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn a_sim_harness_put_in_a_shape_the_protocol_does_not_run_is_refused_by_the_rule_it_breaks() {
        let id = sim_id(106);
        let refused = |cases: Vec<CaseDraft>, builds: &[InterfaceId]| {
            let mut p = project();
            let before = p.clone();
            let err = p
                .apply(ChangeSet::new(
                    vec![sim_put(106, builds, cases)],
                    Vec::new(),
                ))
                .unwrap_err();
            assert_eq!(p, before, "a refused set changes nothing");
            shape_of(err)
        };
        let built = [interface_id()];
        assert_eq!(
            refused(vec![named(70, "regress-a", &[20])], &built),
            [SimShapeProblem::SweepOnce {
                harness: id,
                count: 0,
                extra: None
            }]
        );
        assert_eq!(
            refused(
                vec![named(70, "sweep", &[20]), named(71, "sweep", &[20])],
                &built
            ),
            [SimShapeProblem::SweepOnce {
                harness: id,
                count: 2,
                extra: Some(CaseId::from_uuid(test_uuid(71)))
            }]
        );
        assert_eq!(
            refused(
                vec![named(70, "sweep", &[20]), named(71, "Regress-A", &[20])],
                &built
            ),
            [SimShapeProblem::CaseName {
                harness: id,
                case: CaseId::from_uuid(test_uuid(71)),
                fault: CaseNameFault::NeitherSweepNorRegress
            }]
        );
        assert_eq!(
            refused(
                vec![
                    named(70, "sweep", &[20]),
                    named(71, "regress-a", &[20]),
                    named(72, "regress-a", &[20])
                ],
                &built
            ),
            [SimShapeProblem::CaseName {
                harness: id,
                case: CaseId::from_uuid(test_uuid(72)),
                fault: CaseNameFault::RegressNamedTwice
            }]
        );
        assert_eq!(
            refused(
                vec![named(70, "sweep", &[20]), named(71, "regress-a", &[])],
                &built
            ),
            [SimShapeProblem::CaseCitesNone {
                harness: id,
                case: CaseId::from_uuid(test_uuid(71))
            }]
        );
        assert_eq!(
            refused(vec![named(70, "sweep", &[20])], &[]),
            [SimShapeProblem::BuildsNone { harness: id }]
        );
        // several at once come back as the problems of one refusal, each once
        let many = refused(vec![named(70, "regress-a", &[])], &[]);
        assert_eq!(many.len(), 3, "{many:?}");
    }

    #[test]
    fn a_sim_case_that_cites_a_clause_of_a_context_it_uses_is_refused_though_it_reaches_it() {
        let mut p = project();
        let base = ContextId::from_uuid(test_uuid(2));
        p.apply(ChangeSet::new(
            vec![
                AssetPut::Context(
                    Context::new(
                        base,
                        ContextName::try_from("base").unwrap(),
                        BTreeSet::new(),
                    )
                    .unwrap(),
                ),
                AssetPut::Uses {
                    context: ctx(),
                    uses: BTreeSet::from([base]),
                },
                AssetPut::Contract {
                    context: base,
                    id: ContractId::from_uuid(test_uuid(110)),
                    title: Title::try_from("B").unwrap(),
                    clauses: clauses(&[(90, "C1")]),
                },
            ],
            Vec::new(),
        ))
        .unwrap();
        let err = p
            .apply(ChangeSet::new(
                vec![sim_put(
                    106,
                    &[interface_id()],
                    vec![named(70, "sweep", &[20, 90])],
                )],
                Vec::new(),
            ))
            .unwrap_err();
        assert_eq!(
            shape_of(err),
            [SimShapeProblem::ClauseForeign {
                harness: sim_id(106),
                case: CaseId::from_uuid(test_uuid(70)),
                clause: clause(90)
            }]
        );
    }

    #[test]
    fn a_sim_case_that_cites_a_clause_its_context_does_not_reach_is_refused_once_as_not_reached() {
        let mut p = project();
        let other = ContextId::from_uuid(test_uuid(3));
        p.apply(ChangeSet::new(
            vec![
                AssetPut::Context(
                    Context::new(
                        other,
                        ContextName::try_from("other").unwrap(),
                        BTreeSet::new(),
                    )
                    .unwrap(),
                ),
                AssetPut::Contract {
                    context: other,
                    id: ContractId::from_uuid(test_uuid(111)),
                    title: Title::try_from("O").unwrap(),
                    clauses: clauses(&[(91, "C1")]),
                },
            ],
            Vec::new(),
        ))
        .unwrap();
        let err = p
            .apply(ChangeSet::new(
                vec![sim_put(
                    106,
                    &[interface_id()],
                    vec![named(70, "sweep", &[20, 91])],
                )],
                Vec::new(),
            ))
            .unwrap_err();
        assert!(
            matches!(err, ProjectError::ClauseNotReached { clause: cited, .. } if cited == clause(91)),
            "{err:?}"
        );
    }

    #[test]
    fn a_sim_harness_and_the_contract_it_cites_are_put_in_one_set_and_are_judged_on_the_end_state()
    {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                ctx(),
                ContextName::try_from("core").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        p.apply(ChangeSet::new(
            vec![
                sim_put(106, &[interface_id()], vec![named(70, "sweep", &[20])]),
                contract(&[(20, "C1")]),
                interface(&[(10, "fn a()")]),
            ],
            Vec::new(),
        ))
        .unwrap();
        assert_eq!(
            p.context(ctx())
                .unwrap()
                .harness(sim_id(106))
                .unwrap()
                .kind(),
            HarnessKind::Sim
        );
    }

    #[test]
    fn a_sim_harness_is_swapped_for_another_in_one_set_and_not_by_a_put_alone() {
        let mut p = project();
        let built = [interface_id()];
        p.apply(ChangeSet::new(
            vec![sim_put(106, &built, vec![named(70, "sweep", &[20])])],
            Vec::new(),
        ))
        .unwrap();
        // the new one alone: the Context would have two
        let before = p.clone();
        let err = p
            .apply(ChangeSet::new(
                vec![sim_put(107, &built, vec![named(71, "sweep", &[20])])],
                Vec::new(),
            ))
            .unwrap_err();
        assert_eq!(
            shape_of(err),
            [SimShapeProblem::OnePerContext {
                context: ctx(),
                harnesses: vec![sim_id(106), sim_id(107)]
            }]
        );
        assert_eq!(p, before);
        // the new one and the retirement of the old one, in one set: judged on the state it ends in
        p.apply(ChangeSet::new(
            vec![sim_put(107, &built, vec![named(71, "sweep", &[20])])],
            vec![AssetRetire::Harness {
                context: ctx(),
                id: sim_id(106),
            }],
        ))
        .unwrap();
        let c = p.context(ctx()).unwrap();
        assert!(c.harness(sim_id(106)).is_none() && c.harness(sim_id(107)).is_some());
    }

    #[test]
    fn a_shape_kept_from_before_stops_the_put_of_that_harness_and_no_other_change() {
        let built = [interface_id()];
        let kept = with_kept(&project(), vec![kept_sim(106, &[(70, "stray", &[20])])]);
        // a change that does not put it goes through
        let mut p = kept.clone();
        p.apply(ChangeSet::new(
            vec![contract(&[(20, "C1"), (21, "C7"), (22, "C8")])],
            Vec::new(),
        ))
        .unwrap();
        let mut p = kept.clone();
        p.apply(ChangeSet::new(
            Vec::new(),
            vec![AssetRetire::Harness {
                context: ctx(),
                id: harness_id(),
            }],
        ))
        .unwrap();
        // a put of it is held to the shape, as a put of a new one is
        let mut p = kept.clone();
        let err = p
            .apply(ChangeSet::new(
                vec![sim_put(106, &built, vec![named(70, "stray", &[20])])],
                Vec::new(),
            ))
            .unwrap_err();
        assert_eq!(shape_of(err).len(), 2);
        assert_eq!(p, kept);
        // and the put in the shape the protocol runs mends it
        p.apply(ChangeSet::new(
            vec![sim_put(106, &built, vec![named(70, "sweep", &[20])])],
            Vec::new(),
        ))
        .unwrap();
    }

    #[test]
    fn of_two_sim_harnesses_kept_in_a_context_one_is_mended_only_while_the_other_is_retired() {
        let built = [interface_id()];
        let kept = with_kept(
            &project(),
            vec![
                kept_sim(106, &[(70, "stray", &[20])]),
                kept_sim(107, &[(71, "stray", &[20])]),
            ],
        );
        let mended = || sim_put(106, &built, vec![named(70, "sweep", &[20])]);
        let mut p = kept.clone();
        let err = p
            .apply(ChangeSet::new(vec![mended()], Vec::new()))
            .unwrap_err();
        assert_eq!(
            shape_of(err),
            [SimShapeProblem::OnePerContext {
                context: ctx(),
                harnesses: vec![sim_id(106), sim_id(107)]
            }]
        );
        assert_eq!(p, kept);
        p.apply(ChangeSet::new(
            vec![mended()],
            vec![AssetRetire::Harness {
                context: ctx(),
                id: sim_id(107),
            }],
        ))
        .unwrap();
        assert!(p.context(ctx()).unwrap().harness(sim_id(107)).is_none());
    }

    fn rev(n: u64) -> Revision {
        (1..n).fold(Revision::first(), |r, _| r.next())
    }

    /// The Contract of `project()` with the Clauses in `reworded` given another sentence.
    fn contract_reworded(reworded: &[u128]) -> AssetPut {
        AssetPut::Contract {
            context: ctx(),
            id: contract_id(),
            title: Title::try_from("T").unwrap(),
            clauses: [(20, "C1"), (21, "C7")]
                .iter()
                .map(|&(n, label)| {
                    let text = if reworded.contains(&n) { "b" } else { "a" };
                    (
                        clause(n),
                        ClauseContent::new(
                            ClauseLabel::try_from(label).unwrap(),
                            ClauseText::try_from(text).unwrap(),
                        ),
                    )
                })
                .collect(),
        }
    }

    /// `puts` as a change set for a change request whose frame names `named`.
    fn for_frame(puts: Vec<AssetPut>, named: &[Target]) -> ChangeSet {
        ChangeSet::new(puts, Vec::new()).for_frame(Frame::new(named.iter().copied().collect()))
    }

    /// The Cases of `project()`, put again as they are: 50 cites Item 10 and Clause 20, 51 Item 11 and Clause 21.
    fn the_cases() -> Vec<CaseDraft> {
        vec![draft(50, &[10], &[20]), draft(51, &[11], &[21])]
    }

    fn case_of(p: &Project, n: u128) -> &crate::domain::entity::harness::Case {
        p.context(ctx())
            .unwrap()
            .harness(harness_id())
            .unwrap()
            .case(CaseId::from_uuid(test_uuid(n)))
            .unwrap()
    }

    fn cited_clause(p: &Project, case: u128, cited: u128) -> Option<Revision> {
        case_of(p, case)
            .cites()
            .clauses()
            .get(&clause(cited))
            .copied()
    }

    fn cited_item(p: &Project, case: u128, cited: u128) -> Option<Revision> {
        case_of(p, case).cites().items().get(&item(cited)).copied()
    }

    /// `project()` with the Clauses `reworded` revised and the Harness not put.
    fn stale(reworded: &[u128]) -> Project {
        let mut p = project();
        p.apply(ChangeSet::new(
            vec![contract_reworded(reworded)],
            Vec::new(),
        ))
        .unwrap();
        p
    }

    #[test]
    fn a_put_for_a_frame_leaves_a_case_citing_a_stale_clause_outside_it_as_it_is() {
        let mut p = stale(&[21]);
        let a = Target::Clause(clause(20));
        let (.., harness_was) = revisions(&p);
        p.apply(for_frame(vec![harness(the_cases())], &[a]))
            .unwrap();
        // Case 51 cites Clause 21, which the frame does not hold: it keeps the revision it cited, and its own, and
        // nothing of the Harness moved
        assert_eq!(cited_clause(&p, 51, 21), Some(rev(1)));
        assert_eq!(case_of(&p, 51).revision(), rev(1));
        assert_eq!(case_of(&p, 50).revision(), rev(1));
        assert_eq!(revisions(&p).3, harness_was);
        // the Clause itself is at the revision the earlier set gave it, so the citation is stale
        let c = p.context(ctx()).unwrap();
        let revised = c.contract(contract_id()).unwrap().clause(clause(21));
        assert_eq!(revised.unwrap().revision(), rev(2));
    }

    #[test]
    fn a_put_for_a_frame_brings_up_the_cases_citing_what_the_frame_holds_and_no_other() {
        let mut p = stale(&[20, 21]);
        let a = Target::Clause(clause(20));
        p.apply(for_frame(vec![harness(the_cases())], &[a]))
            .unwrap();
        // Case 50 cites Clause 20: brought up, and it moves with it
        assert_eq!(cited_clause(&p, 50, 20), Some(rev(2)));
        assert_eq!(case_of(&p, 50).revision(), rev(2));
        // Case 51 cites Clause 21: still stale, and it did not move
        assert_eq!(cited_clause(&p, 51, 21), Some(rev(1)));
        assert_eq!(case_of(&p, 51).revision(), rev(1));
        // the other part's put brings up its own
        let b = Target::Clause(clause(21));
        p.apply(for_frame(vec![harness(the_cases())], &[b]))
            .unwrap();
        assert_eq!(cited_clause(&p, 51, 21), Some(rev(2)));
        assert_eq!(case_of(&p, 51).revision(), rev(2));
        assert_eq!(cited_clause(&p, 50, 20), Some(rev(2)));
        assert_eq!(case_of(&p, 50).revision(), rev(2));
    }

    #[test]
    fn a_case_citing_both_is_brought_up_on_the_framed_citation_only() {
        let mut cases = the_cases();
        cases.push(draft(52, &[10], &[20, 21]));
        let mut p = project();
        p.apply(ChangeSet::new(vec![harness(cases.clone())], Vec::new()))
            .unwrap();
        p.apply(ChangeSet::new(
            vec![contract_reworded(&[20, 21])],
            Vec::new(),
        ))
        .unwrap();
        let (a, b) = (Target::Clause(clause(20)), Target::Clause(clause(21)));
        p.apply(for_frame(vec![harness(cases.clone())], &[a]))
            .unwrap();
        assert_eq!(cited_clause(&p, 52, 20), Some(rev(2)));
        assert_eq!(cited_clause(&p, 52, 21), Some(rev(1)));
        assert_eq!(case_of(&p, 52).revision(), rev(2));
        p.apply(for_frame(vec![harness(cases)], &[b])).unwrap();
        assert_eq!(cited_clause(&p, 52, 20), Some(rev(2)));
        assert_eq!(cited_clause(&p, 52, 21), Some(rev(2)));
        assert_eq!(case_of(&p, 52).revision(), rev(3));
    }

    #[test]
    fn a_stale_item_outside_the_frame_stays_stale_too() {
        let mut p = project();
        // Item 11 is given another declaration; nothing puts the Harness
        p.apply(ChangeSet::new(
            vec![AssetPut::Interface {
                context: ctx(),
                id: interface_id(),
                source: Source::new("fn a()\nfn release() -> u8\n".to_string()),
                items: items(&[(10, "fn a()"), (11, "fn release()")]),
            }],
            Vec::new(),
        ))
        .unwrap();
        let a = Target::Clause(clause(20));
        p.apply(for_frame(vec![harness(the_cases())], &[a]))
            .unwrap();
        assert_eq!(cited_item(&p, 51, 11), Some(rev(1)));
        assert_eq!(case_of(&p, 51).revision(), rev(1));
        // naming the Interface holds the Item
        let i = Target::Interface(interface_id());
        p.apply(for_frame(vec![harness(the_cases())], &[i]))
            .unwrap();
        assert_eq!(cited_item(&p, 51, 11), Some(rev(2)));
        assert_eq!(case_of(&p, 51).revision(), rev(2));
    }

    #[test]
    fn a_citation_the_case_did_not_have_and_a_new_case_are_current() {
        let mut p = stale(&[21]);
        let a = Target::Clause(clause(20));
        let mut cases = the_cases();
        // Case 50 now cites Clause 21 as well, and Case 52 is new and cites it
        cases[0] = draft(50, &[10], &[20, 21]);
        cases.push(draft(52, &[10], &[21]));
        p.apply(for_frame(vec![harness(cases)], &[a])).unwrap();
        assert_eq!(cited_clause(&p, 50, 21), Some(rev(2)));
        assert_eq!(cited_clause(&p, 52, 21), Some(rev(2)));
        assert_eq!(case_of(&p, 52).revision(), Revision::first());
        // the Case that cited it before stays stale
        assert_eq!(cited_clause(&p, 51, 21), Some(rev(1)));
    }

    #[test]
    fn a_frame_that_holds_the_case_brings_up_all_it_cites() {
        let named = [
            Target::Case(CaseId::from_uuid(test_uuid(51))),
            Target::Harness(harness_id()),
            Target::Context(ctx()),
            Target::Project,
            Target::Contract(contract_id()),
        ];
        for target in named {
            let mut p = stale(&[20, 21]);
            p.apply(for_frame(vec![harness(the_cases())], &[target]))
                .unwrap();
            assert_eq!(cited_clause(&p, 51, 21), Some(rev(2)), "{target:?}");
            let case_50 = if target == Target::Case(CaseId::from_uuid(test_uuid(51))) {
                // a Case named brings up its own citations alone
                rev(1)
            } else {
                rev(2)
            };
            assert_eq!(cited_clause(&p, 50, 20), Some(case_50), "{target:?}");
        }
    }

    #[test]
    fn a_put_for_no_request_brings_every_citation_up() {
        let mut p = stale(&[20, 21]);
        p.apply(ChangeSet::new(vec![harness(the_cases())], Vec::new()))
            .unwrap();
        assert_eq!(cited_clause(&p, 50, 20), Some(rev(2)));
        assert_eq!(cited_clause(&p, 51, 21), Some(rev(2)));
        // a set for a request whose frame names nothing is not a set for no request: it holds nothing
        let mut q = stale(&[20, 21]);
        q.apply(for_frame(vec![harness(the_cases())], &[])).unwrap();
        assert_eq!(cited_clause(&q, 50, 20), Some(rev(1)));
        assert_eq!(cited_clause(&q, 51, 21), Some(rev(1)));
    }

    #[test]
    fn a_set_that_revises_a_clause_outside_its_frame_leaves_the_cases_citing_it_stale() {
        let mut p = project();
        let a = Target::Clause(clause(20));
        p.apply(for_frame(
            vec![contract_reworded(&[21]), harness(the_cases())],
            &[a],
        ))
        .unwrap();
        let c = p.context(ctx()).unwrap();
        let revised = c.contract(contract_id()).unwrap().clause(clause(21));
        assert_eq!(revised.unwrap().revision(), rev(2));
        // Case 51 cites it at the revision it had: stale right after the set
        assert_eq!(cited_clause(&p, 51, 21), Some(rev(1)));
        assert_eq!(case_of(&p, 51).revision(), rev(1));
    }
}
