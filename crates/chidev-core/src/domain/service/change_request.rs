//! Deciding and resolving a change request against the Project, and finding the ones the definitions have not
//! followed.

use std::collections::BTreeMap;

use super::cite::revision_of;
use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestError, ChangeRequestState,
};
use crate::domain::entity::project::Project;
use crate::domain::vo::crux::Decision;
use crate::domain::vo::id::ChangeRequestId;
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

/// Gives the Crux of `cr` the person's decision, with each line at the revision its definition has in `project`
/// now.
pub fn decide(
    cr: &mut ChangeRequest,
    decision: Decision,
    project: &Project,
) -> Result<(), ChangeRequestError> {
    let now = revisions_now(project, cr.lines().keys().copied());
    cr.decide(decision, &now)
}

/// The lines of `cr` the change has not reached in `project`: those still at the revision they had when it was
/// opened or its Crux decided, and those whose definition is gone. Empty when it asks no change.
fn behind(cr: &ChangeRequest, project: &Project) -> (Vec<Target>, Vec<Target>) {
    let mut unchanged = Vec::new();
    let mut gone = Vec::new();
    if cr.asks_change() {
        for (target, recorded) in cr.lines() {
            match revision_of(project, *target) {
                None => gone.push(*target),
                Some(now) if now == *recorded => unchanged.push(*target),
                Some(_) => {}
            }
        }
    }
    (unchanged, gone)
}

/// Closes the record of `cr` once the change is applied: every line it asks to change has a later revision than
/// the one it recorded, and none is gone.
pub fn resolve(cr: &mut ChangeRequest, project: &Project) -> Result<(), ChangeRequestError> {
    let (unchanged, gone) = behind(cr, project);
    if !unchanged.is_empty() || !gone.is_empty() {
        return Err(ChangeRequestError::NotApplied(
            unchanged.into_iter().chain(gone).collect(),
        ));
    }
    cr.resolve()
}

/// An unresolved change request the definitions have not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapplied {
    /// The change request.
    pub cr: ChangeRequestId,
    /// The definitions still at the revision it recorded, in order.
    pub unchanged: Vec<Target>,
    /// The definitions no asset of the Project has any more, in order.
    pub gone: Vec<Target>,
}

/// Every unresolved change request in `crs` that asks a change which has not reached all its lines. One whose Crux
/// is not decided yet asks nothing: work goes on with the default.
pub fn unapplied<'a>(
    crs: impl IntoIterator<Item = &'a ChangeRequest>,
    project: &Project,
) -> Vec<Unapplied> {
    crs.into_iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved)
        .filter_map(|cr| {
            let (unchanged, gone) = behind(cr, project);
            (!unchanged.is_empty() || !gone.is_empty()).then(|| Unapplied {
                cr: cr.id(),
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
    use crate::domain::entity::change_request::{ChangeRequestDraft, Crux, Route};
    use crate::domain::entity::context::Context;
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::crux::{CruxKind, CruxOption, Reasoned};
    use crate::domain::vo::id::{ClauseId, ContextId, ContractId, ProjectId, test_uuid};
    use crate::domain::vo::intent::Intent;
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

    fn cr(p: &Project, route: Route) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            ChangeRequestDraft {
                title: Title::try_from("How is a half rounded?").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                lines: revisions_now(p, [clause()]),
                route,
                addresses: Vec::new(),
            },
        )
    }

    fn person(p: &Project) -> ChangeRequest {
        let crux = Crux::new(
            CruxKind::Ambiguity,
            vec![
                CruxOption::new(choice("up"), None, None),
                CruxOption::new(
                    choice("to even"),
                    Some(Prose::try_from("C1 rounds half to even").unwrap()),
                    None,
                ),
            ],
            Some(Reasoned::new(choice("up"), None)),
            None,
        )
        .unwrap();
        cr(p, Route::Person(crux))
    }

    fn chosen(c: &str) -> Decision {
        Decision::Chosen(Reasoned::new(choice(c), None))
    }

    #[test]
    fn the_default_or_no_decision_asks_nothing_and_resolves_at_once() {
        let p = project();
        let mut c = person(&p);
        assert!(unapplied([&c], &p).is_empty());
        decide(&mut c, chosen("up"), &p).unwrap();
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn a_decision_asking_a_change_resolves_only_once_the_definitions_follow() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                cr: c.id(),
                unchanged: vec![clause()],
                gone: Vec::new()
            }]
        );
        assert_eq!(
            resolve(&mut c, &p),
            Err(ChangeRequestError::NotApplied(vec![clause()]))
        );
        put_contract(&mut p, "a half rounds to even");
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p).unwrap();
    }

    #[test]
    fn a_change_request_the_machine_resolves_waits_for_its_lines_to_change() {
        let mut p = project();
        let mut c = cr(&p, Route::Machine);
        assert_eq!(unapplied([&c], &p).len(), 1);
        assert!(resolve(&mut c, &p).is_err());
        put_contract(&mut p, "a half rounds up, to whole cents");
        resolve(&mut c, &p).unwrap();
        assert!(unapplied([&c], &p).is_empty());
    }

    #[test]
    fn a_definition_that_is_gone_is_reported() {
        let p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(
            unapplied([&c], &empty),
            [Unapplied {
                cr: c.id(),
                unchanged: Vec::new(),
                gone: vec![clause()]
            }]
        );
    }
}
