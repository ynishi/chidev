//! What a change set moved: the targets whose revision differs between the Project before it and the one it leaves.

use std::collections::{BTreeMap, BTreeSet};

use crate::domain::entity::project::Project;
use crate::domain::vo::footprint::Move;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

/// Every target of `project` with the revision it has and the containers it is in (the Project first).
fn tree(project: &Project) -> Tree {
    let mut out = BTreeMap::new();
    out.insert(Target::Project, (project.revision(), Vec::new()));
    for context in project.contexts() {
        let here = vec![Target::Project];
        out.insert(Target::Context(context.id()), (context.revision(), here));
        let in_context = vec![Target::Project, Target::Context(context.id())];
        let asset = |target: Target| {
            let mut within = in_context.clone();
            within.push(target);
            within
        };
        for contract in context.contracts() {
            let at = Target::Contract(contract.id());
            out.insert(at, (contract.revision(), in_context.clone()));
            for clause in contract.clauses() {
                out.insert(Target::Clause(clause.id()), (clause.revision(), asset(at)));
            }
        }
        for interface in context.interfaces() {
            let at = Target::Interface(interface.id());
            out.insert(at, (interface.revision(), in_context.clone()));
            for item in interface.items() {
                out.insert(Target::Item(item.id()), (item.revision(), asset(at)));
            }
        }
        for harness in context.harnesses() {
            let at = Target::Harness(harness.id());
            out.insert(at, (harness.revision(), in_context.clone()));
            for case in harness.cases() {
                out.insert(Target::Case(case.id()), (case.revision(), asset(at)));
            }
        }
    }
    out
}

/// Every target of a Project with the revision it has and the containers it is in, as `tree` gives it.
type Tree = BTreeMap<Target, (Revision, Vec<Target>)>;

/// What each Case of `project` cites: the Items and Clauses.
fn citations(project: &Project) -> BTreeMap<Target, Vec<Target>> {
    project
        .contexts()
        .flat_map(|c| c.harnesses())
        .flat_map(|h| h.cases())
        .map(|case| (Target::Case(case.id()), case.cites().targets().collect()))
        .collect()
}

/// The targets that moved from `before` to `after`: the ones made, the ones whose revision went up, and the ones
/// that are gone, each with the containers it was in (as `after` has it, or as `before` had it for one that is
/// gone), in the order of the targets. A container whose revision moved because something in it was added or
/// removed is a move of its own.
///
/// A Case that moved also carries what it cites: in `after` for one that was made, in `before` for one that is
/// gone, and in both together for one that was changed, so that a change set that retires a Clause and moves the
/// Cases off it still counts them as citing it.
pub fn moves_between(before: &Project, after: &Project) -> Vec<Move> {
    let (was, now) = (tree(before), tree(after));
    let (was_cites, now_cites) = (citations(before), citations(after));
    // What the Case `m` moved cites on each side given, with the containers of each as that side has them.
    let with_cites = |m: Move, sides: &[(&Tree, &BTreeMap<Target, Vec<Target>>)]| {
        let mut cited = BTreeSet::new();
        let mut within = BTreeSet::new();
        for (tree, cites) in sides {
            for t in cites.get(&m.target()).into_iter().flatten() {
                cited.insert(*t);
                if let Some((_, w)) = tree.get(t) {
                    within.extend(w.iter().copied());
                }
            }
        }
        m.with_cites(cited.into_iter().collect(), within.into_iter().collect())
    };
    let mut moves = Vec::new();
    for (target, (revision, within)) in &now {
        if was.get(target).map(|(r, _)| r) != Some(revision) {
            let sides: &[_] = if was.contains_key(target) {
                &[(&was, &was_cites), (&now, &now_cites)]
            } else {
                &[(&now, &now_cites)]
            };
            moves.push(with_cites(Move::new(*target, within.clone()), sides));
        }
    }
    for (target, (_, within)) in &was {
        if !now.contains_key(target) {
            moves.push(with_cites(
                Move::new(*target, within.clone()),
                &[(&was, &was_cites)],
            ));
        }
    }
    moves.sort_by_key(Move::target);
    moves
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::entity::context::Context;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::id::{
        CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId, ProjectId,
        test_uuid,
    };
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::title::Title;

    fn core() -> ContextId {
        ContextId::from_uuid(test_uuid(1))
    }

    fn contract_id() -> ContractId {
        ContractId::from_uuid(test_uuid(300))
    }

    fn clause_id(n: u128) -> ClauseId {
        ClauseId::from_uuid(test_uuid(n))
    }

    fn put(p: &mut Project, clauses: &[(u128, &str, &str)]) {
        p.put_contract(
            core(),
            contract_id(),
            Title::try_from("T").unwrap(),
            clauses
                .iter()
                .map(|(n, label, text)| {
                    (
                        clause_id(*n),
                        ClauseContent::new(
                            ClauseLabel::try_from(*label).unwrap(),
                            ClauseText::try_from(*text).unwrap(),
                        ),
                    )
                })
                .collect(),
        )
        .unwrap();
    }

    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                core(),
                ContextName::try_from("core").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        put(&mut p, &[(30, "C1", "one"), (31, "C2", "two")]);
        p
    }

    #[test]
    fn nothing_moves_when_the_project_is_the_same() {
        let p = project();
        assert!(moves_between(&p, &p.clone()).is_empty());
    }

    #[test]
    fn a_revised_clause_moves_with_its_contract_and_no_container_above() {
        let before = project();
        let mut after = before.clone();
        put(&mut after, &[(30, "C1", "one, revised"), (31, "C2", "two")]);
        let moved: Vec<Target> = moves_between(&before, &after)
            .iter()
            .map(Move::target)
            .collect();
        assert_eq!(
            moved,
            [
                Target::Contract(contract_id()),
                Target::Clause(clause_id(30))
            ]
        );
    }

    #[test]
    fn a_clause_that_is_dropped_moves_with_the_containers_it_was_in() {
        let before = project();
        let mut after = before.clone();
        put(&mut after, &[(30, "C1", "one")]);
        let moves = moves_between(&before, &after);
        let dropped = moves
            .iter()
            .find(|m| m.target() == Target::Clause(clause_id(31)))
            .unwrap();
        assert_eq!(
            dropped.within(),
            [
                Target::Project,
                Target::Context(core()),
                Target::Contract(contract_id())
            ]
        );
        assert!(dropped.is_inside(Target::Context(core())));
        assert!(
            moves
                .iter()
                .any(|m| m.target() == Target::Contract(contract_id()))
        );
    }

    fn item_id() -> ItemId {
        ItemId::from_uuid(test_uuid(10))
    }

    fn harness_id() -> HarnessId {
        HarnessId::from_uuid(test_uuid(400))
    }

    /// The Harness with the Cases given, each its number and the Clauses (numbers) it cites besides the Item.
    fn put_cases(p: &mut Project, body: &str, cases: &[(u128, &[u128])]) {
        p.put_harness(
            core(),
            harness_id(),
            Source::new(format!("{body}\n")),
            cases
                .iter()
                .map(|(n, clauses)| {
                    (
                        CaseId::from_uuid(test_uuid(*n)),
                        CaseName::try_from("t").unwrap(),
                        LineRange::new(1, 1).unwrap(),
                        CaseCites::new(
                            BTreeMap::from([(item_id(), Revision::first())]),
                            clauses
                                .iter()
                                .map(|c| (clause_id(*c), Revision::first()))
                                .collect(),
                        )
                        .unwrap(),
                    )
                })
                .collect(),
        )
        .unwrap();
    }

    fn with_an_interface() -> Project {
        let mut p = project();
        p.put_interface(
            core(),
            InterfaceId::from_uuid(test_uuid(100)),
            Source::new("fn a()\n".to_string()),
            vec![(
                item_id(),
                ItemKey::try_from("fn a").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        p
    }

    fn move_of(moves: &[Move], target: Target) -> &Move {
        moves.iter().find(|m| m.target() == target).unwrap()
    }

    #[test]
    fn a_moved_case_carries_what_it_cited_before_and_after_the_change_set_and_nothing_else_does() {
        let mut before = with_an_interface();
        put_cases(&mut before, "fn t() {}", &[(40, &[30]), (41, &[31])]);
        let mut after = before.clone();
        // 40 now cites C2 (it cited C1), 41 is gone, 42 is new and cites C1
        put_cases(&mut after, "fn t() { 1 }", &[(40, &[31]), (42, &[30])]);
        let moves = moves_between(&before, &after);
        let (item, c1, c2) = (
            Target::Item(item_id()),
            Target::Clause(clause_id(30)),
            Target::Clause(clause_id(31)),
        );
        let (core, contract) = (Target::Context(core()), Target::Contract(contract_id()));
        let case = |n: u128| Target::Case(CaseId::from_uuid(test_uuid(n)));
        // changed: before and after together
        let changed = move_of(&moves, case(40));
        assert_eq!(changed.cites(), [c1, c2, item]);
        assert!(changed.cites_inside(contract) && changed.cites_inside(core));
        assert!(changed.cites_inside(Target::Interface(InterfaceId::from_uuid(test_uuid(100)))));
        // retired: before
        let gone = move_of(&moves, case(41));
        assert_eq!(gone.cites(), [c2, item]);
        assert!(!gone.cites_inside(c1));
        // added: after
        let made = move_of(&moves, case(42));
        assert_eq!(made.cites(), [c1, item]);
        assert!(!made.cites_inside(c2));
        // a Harness, a Contract, a Clause: no citations
        for other in [Target::Harness(harness_id()), contract] {
            assert!(
                moves
                    .iter()
                    .filter(|m| m.target() == other)
                    .all(|m| m.cites().is_empty() && m.cites_within().is_empty())
            );
        }
    }

    #[test]
    fn a_new_asset_moves_with_the_context_that_holds_it() {
        let mut before = project();
        before.retire_contract(core(), contract_id()).unwrap();
        let mut after = before.clone();
        after
            .put_contract(
                core(),
                ContractId::from_uuid(test_uuid(301)),
                Title::try_from("U").unwrap(),
                vec![(
                    clause_id(40),
                    ClauseContent::new(
                        ClauseLabel::try_from("C1").unwrap(),
                        ClauseText::try_from("new").unwrap(),
                    ),
                )],
            )
            .unwrap();
        let moved: Vec<Target> = moves_between(&before, &after)
            .iter()
            .map(Move::target)
            .collect();
        assert!(moved.contains(&Target::Context(core())));
        assert!(moved.contains(&Target::Contract(ContractId::from_uuid(test_uuid(301)))));
        assert!(moved.contains(&Target::Clause(clause_id(40))));
    }
}
