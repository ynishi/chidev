//! What a change set moved: the targets whose revision differs between the Project before it and the one it leaves.

use std::collections::BTreeMap;

use crate::domain::entity::project::Project;
use crate::domain::vo::footprint::Move;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

/// Every target of `project` with the revision it has and the containers it is in (the Project first).
fn tree(project: &Project) -> BTreeMap<Target, (Revision, Vec<Target>)> {
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

/// The targets that moved from `before` to `after`: the ones made, the ones whose revision went up, and the ones
/// that are gone, each with the containers it was in (as `after` has it, or as `before` had it for one that is
/// gone), in the order of the targets. A container whose revision moved because something in it was added or
/// removed is a move of its own.
pub fn moves_between(before: &Project, after: &Project) -> Vec<Move> {
    let (was, now) = (tree(before), tree(after));
    let mut moves = Vec::new();
    for (target, (revision, within)) in &now {
        if was.get(target).map(|(r, _)| r) != Some(revision) {
            moves.push(Move::new(*target, within.clone()));
        }
    }
    for (target, (_, within)) in &was {
        if !now.contains_key(target) {
            moves.push(Move::new(*target, within.clone()));
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
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::id::{ClauseId, ContextId, ContractId, ProjectId, test_uuid};
    use crate::domain::vo::name::ContextName;
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
