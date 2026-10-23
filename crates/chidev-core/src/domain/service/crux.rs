//! Deciding and resolving a Crux against the Project, and finding decisions the definitions have not followed.

use std::collections::BTreeMap;

use super::cite::revision_of;
use crate::domain::entity::crux::{Crux, CruxError, CruxState};
use crate::domain::entity::project::Project;
use crate::domain::vo::crux::Decision;
use crate::domain::vo::id::CruxId;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

/// The revision each of `targets` has in `project` now; a target no asset has is left out.
pub fn revisions_now(
    project: &Project,
    targets: impl IntoIterator<Item = Target>,
) -> BTreeMap<Target, Revision> {
    targets
        .into_iter()
        .filter_map(|t| revision_of(project, t).map(|r| (t, r)))
        .collect()
}

/// Gives `crux` the person's decision, with each line at the revision its definition has in `project` now.
pub fn decide(crux: &mut Crux, decision: Decision, project: &Project) -> Result<(), CruxError> {
    let now = revisions_now(project, crux.lines().keys().copied());
    crux.decide(decision, &now)
}

/// The lines of `crux` its decision has not reached in `project`: those that asked a change and are still at the
/// revision they had when it was decided, and those whose definition is gone. Empty when the decision asks no
/// change.
fn behind(crux: &Crux, project: &Project) -> (Vec<Target>, Vec<Target>) {
    let mut unchanged = Vec::new();
    let mut gone = Vec::new();
    if crux.asks_change() {
        for (target, at_decision) in crux.lines() {
            match revision_of(project, *target) {
                None => gone.push(*target),
                Some(now) if now == *at_decision => unchanged.push(*target),
                Some(_) => {}
            }
        }
    }
    (unchanged, gone)
}

/// Closes the record of `crux` once its decision is applied: every line it asked to change has a later revision
/// than when it was decided, and none is gone.
pub fn resolve(crux: &mut Crux, project: &Project) -> Result<(), CruxError> {
    let (unchanged, gone) = behind(crux, project);
    if !unchanged.is_empty() || !gone.is_empty() {
        return Err(CruxError::NotApplied(
            unchanged.into_iter().chain(gone).collect(),
        ));
    }
    crux.resolve()
}

/// A decided Crux whose definitions have not followed its decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapplied {
    /// The Crux.
    pub crux: CruxId,
    /// The definitions still at the revision they had when the decision was taken, in order.
    pub unchanged: Vec<Target>,
    /// The definitions no asset of the Project has any more, in order.
    pub gone: Vec<Target>,
}

/// Every unresolved Crux in `cruxes` whose decision asks a change that has not reached all its lines.
pub fn unapplied<'a>(
    cruxes: impl IntoIterator<Item = &'a Crux>,
    project: &Project,
) -> Vec<Unapplied> {
    cruxes
        .into_iter()
        .filter(|c| c.state() == CruxState::Decided)
        .filter_map(|crux| {
            let (unchanged, gone) = behind(crux, project);
            (!unchanged.is_empty() || !gone.is_empty()).then(|| Unapplied {
                crux: crux.id(),
                unchanged,
                gone,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::entity::context::Context;
    use crate::domain::entity::crux::CruxDraft;
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::crux::{CruxKind, CruxOption, Reasoned};
    use crate::domain::vo::id::{ClauseId, ContextId, ContractId, ProjectId, test_uuid};
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::title::Title;

    fn clause() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(30)))
    }

    fn put_contract(p: &mut Project, text: &str) {
        p.put_contract(
            ContextId::from_uuid(test_uuid(1)),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![(
                ClauseId::from_uuid(test_uuid(30)),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from(text).unwrap(),
                ),
            )],
        )
        .unwrap();
    }

    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                ContextId::from_uuid(test_uuid(1)),
                ContextName::try_from("core").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        put_contract(&mut p, "a half rounds up");
        p
    }

    fn choice(s: &str) -> Choice {
        Choice::try_from(s).unwrap()
    }

    fn crux(p: &Project) -> Crux {
        Crux::new(
            CruxId::from_uuid(test_uuid(1)),
            CruxDraft {
                kind: CruxKind::Ambiguity,
                title: Title::try_from("How is a half rounded?").unwrap(),
                situation: None,
                source: None,
                options: vec![
                    CruxOption::new(choice("up"), None, None),
                    CruxOption::new(
                        choice("to even"),
                        Some(Prose::try_from("C1 rounds half to even").unwrap()),
                        None,
                    ),
                ],
                default: Some(Reasoned::new(choice("up"), None)),
                recommended: None,
                lines: revisions_now(p, [clause()]),
            },
        )
        .unwrap()
    }

    fn chosen(c: &str) -> Decision {
        Decision::Chosen(Reasoned::new(choice(c), None))
    }

    #[test]
    fn the_default_or_no_decision_asks_nothing_and_resolves_at_once() {
        let p = project();
        let mut c = crux(&p);
        assert!(unapplied([&c], &p).is_empty());
        decide(&mut c, chosen("up"), &p).unwrap();
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p).unwrap();
        assert_eq!(c.state(), CruxState::Resolved);
    }

    #[test]
    fn a_decision_asking_a_change_resolves_only_once_the_definitions_follow() {
        let mut p = project();
        let mut c = crux(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                crux: c.id(),
                unchanged: vec![clause()],
                gone: Vec::new()
            }]
        );
        assert_eq!(
            resolve(&mut c, &p),
            Err(CruxError::NotApplied(vec![clause()]))
        );
        put_contract(&mut p, "a half rounds to even");
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p).unwrap();
        assert!(unapplied([&c], &p).is_empty());
    }

    #[test]
    fn a_definition_that_is_gone_is_reported() {
        let p = project();
        let mut c = crux(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(
            unapplied([&c], &empty),
            [Unapplied {
                crux: c.id(),
                unchanged: Vec::new(),
                gone: vec![clause()]
            }]
        );
    }
}
