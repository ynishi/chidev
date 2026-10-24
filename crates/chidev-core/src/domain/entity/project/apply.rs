//! `Project::apply`: a change set applied on a copy of the Contexts, the rules of the Project checked once on the
//! state it ends in.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use super::{
    Project, ProjectError, Taken, check_structure, cite_problems, contract_records, dropped_leaves,
    harness_records, interface_records, refuse_cited,
};
use crate::domain::entity::context::{Context, ContextError};
use crate::domain::entity::harness::{CaseFound, HarnessError};
use crate::domain::vo::case::CaseCites;
use crate::domain::vo::change_set::{AssetPut, AssetRetire, CaseDraft, ChangeSet};
use crate::domain::vo::id::{ClauseId, ContextId, ItemId};
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
    /// The entries are applied in the order the rules need, whatever order they are listed in: the uses of the
    /// Contexts; the Contracts and Interfaces; the basis of the Contexts that are based on their Contracts; the
    /// Harnesses (a citation is recorded at the revision its target has by then); the retirements. The Clauses, Items
    /// and Cases that a put drops from an asset are recorded as retired, as they are by a put alone. Each asset
    /// moves by one revision at most, because it is named once: a change set that names a thing twice is refused.
    ///
    /// When it is refused, nothing changes, and the error names every problem found: a single one as it is, several
    /// as [`ProjectError::Many`]. Problems with the entries themselves (an unknown Context, a Contract that breaks
    /// a rule of its own) are found first; the rules of the Project are only checked when the entries all applied.
    pub fn apply(&mut self, set: ChangeSet) -> Result<(), ProjectError> {
        let repeated = repeats(&set);
        if !repeated.is_empty() {
            return Err(refused(repeated));
        }
        let ChangeSet { mut puts, retires } = set;
        puts.sort_by_key(stage);
        let mut problems = Vec::new();
        let mut next = self.contexts.clone();
        let mut touched = BTreeSet::new();
        for put in puts {
            let (context, done) = put_one(&mut next, put);
            touched.insert(context);
            if let Err(e) = done {
                problems.push(e);
            }
        }
        let dropped: Vec<Retired> = touched
            .iter()
            .filter_map(|id| Some(dropped_leaves(self.contexts.get(id)?, next.get(id)?)))
            .flatten()
            .collect();

        let mut revision = self.revision;
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
        }
        if !problems.is_empty() {
            return Err(refused(problems));
        }
        self.contexts = next;
        self.retired = retired;
        self.revision = revision;
        Ok(())
    }
}

/// The error of a refused change set: the one problem, or all of them.
fn refused(problems: Vec<ProjectError>) -> ProjectError {
    match problems.as_slice() {
        [only] => only.clone(),
        _ => ProjectError::Many(problems),
    }
}

/// When a put is applied: settings first, then the definitions, then the Harnesses that cite them.
fn stage(put: &AssetPut) -> u8 {
    match put {
        AssetPut::Uses { .. } => 0,
        AssetPut::Contract { .. } | AssetPut::Interface { .. } => 1,
        AssetPut::BaseOnContract { .. } => 2,
        AssetPut::Harness { .. } => 3,
    }
}

/// The Context a put is about, and whether it was put on `next`.
fn put_one(
    next: &mut BTreeMap<ContextId, Context>,
    put: AssetPut,
) -> (ContextId, Result<(), ProjectError>) {
    match put {
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
            source,
            cases,
        } => {
            let found: Result<Vec<CaseFound>, ProjectError> =
                cases.into_iter().map(|d| found_of(next, d)).collect();
            let done = match found {
                Ok(found) => edit(next, context, |c| c.put_harness(id, kind, source, found)),
                Err(e) => Err(e),
            };
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

/// The Case as the Harness takes it, each citation at the revision its target has in `contexts` now. A target that
/// is nowhere gets the first revision; the check of the citations finds it unreached.
fn found_of(
    contexts: &BTreeMap<ContextId, Context>,
    draft: CaseDraft,
) -> Result<CaseFound, ProjectError> {
    let item = |id: &ItemId| {
        contexts
            .values()
            .find_map(|c| c.interface_of(*id)?.item(*id))
            .map_or_else(Revision::first, |it| it.revision())
    };
    let clause = |id: &ClauseId| {
        contexts
            .values()
            .find_map(|c| c.contract_of(*id)?.clause(*id))
            .map_or_else(Revision::first, |cl| cl.revision())
    };
    let cites = CaseCites::new(
        draft.items().iter().map(|i| (*i, item(i))).collect(),
        draft.clauses().iter().map(|k| (*k, clause(k))).collect(),
    )
    .map_err(|e| ProjectError::Context(ContextError::Harness(HarnessError::Cites(e))))?;
    Ok((draft.id(), draft.name().clone(), draft.lines(), cites))
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
            AssetPut::Uses { context, .. } => note("uses", "context", context.as_uuid()),
            AssetPut::BaseOnContract { context } => note("base", "context", context.as_uuid()),
            AssetPut::Contract { id, .. } => note("contract", "contract", id.as_uuid()),
            AssetPut::Interface { id, .. } => note("interface", "interface", id.as_uuid()),
            AssetPut::Harness { id, .. } => note("harness", "harness", id.as_uuid()),
        }
        let context = match put {
            AssetPut::Uses { context, .. }
            | AssetPut::BaseOnContract { context }
            | AssetPut::Contract { context, .. }
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
    use crate::domain::vo::case::CaseName;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::id::{CaseId, ContractId, HarnessId, InterfaceId, ProjectId, test_uuid};
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::signature::Signature;
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

    fn items(ids: &[(u128, &str)]) -> Vec<(ItemId, Signature, LineRange)> {
        ids.iter()
            .enumerate()
            .map(|(k, (n, sig))| {
                (
                    item(*n),
                    Signature::try_from(*sig).unwrap(),
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
        .unwrap()
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
            source: Source::new("fn t() {}\n".to_string()),
            cases,
        }
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
}
