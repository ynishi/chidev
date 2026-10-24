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

/// Gives the Crux of `cr` the person's decision, with each definition the option chosen asks to change at the
/// revision it has in `project` now.
pub fn decide(
    cr: &mut ChangeRequest,
    decision: Decision,
    project: &Project,
) -> Result<(), ChangeRequestError> {
    let targets = cr
        .crux()
        .into_iter()
        .flat_map(|q| q.options())
        .flat_map(|o| o.changes().iter().copied());
    let now = revisions_now(project, targets);
    cr.decide(decision, &now)
}

/// The definitions `cr` asks to change that the change has not reached in `project`: the lines, and the changes
/// the decision asked, still at the revision they were recorded at, and those whose definition is gone. A
/// definition both a line and a decided change counts once.
fn behind(cr: &ChangeRequest, project: &Project) -> (Vec<Target>, Vec<Target>) {
    let mut unchanged = Vec::new();
    let mut gone = Vec::new();
    for (target, recorded) in cr.lines().iter().chain(cr.decided_changes()) {
        match revision_of(project, *target) {
            None if !gone.contains(target) => gone.push(*target),
            Some(now) if now == *recorded && !unchanged.contains(target) => unchanged.push(*target),
            _ => {}
        }
    }
    (unchanged, gone)
}

/// Closes the record of `cr` once the change is applied: its Crux, if it has one, is decided; every line and every
/// change the decision asked has a later revision than the one it recorded; and none is gone.
pub fn resolve(cr: &mut ChangeRequest, project: &Project) -> Result<(), ChangeRequestError> {
    if !cr.awaits_decision() {
        let (unchanged, gone) = behind(cr, project);
        if !unchanged.is_empty() || !gone.is_empty() {
            return Err(ChangeRequestError::NotApplied { unchanged, gone });
        }
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

/// Every unresolved change request in `crs` with a line, or a change its decision asked, the definitions have not
/// followed. One whose Crux is not decided yet is here for its lines, and also in [`awaiting_decision`].
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

/// Every unresolved change request in `crs` whose Crux is not decided yet: a person is awaited.
pub fn awaiting_decision<'a>(
    crs: impl IntoIterator<Item = &'a ChangeRequest>,
) -> Vec<&'a ChangeRequest> {
    crs.into_iter().filter(|c| c.awaits_decision()).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::entity::change_request::{ChangeRequestDraft, Crux};
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

    /// The clause the question is about: which error a refusal is.
    fn detail() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(31)))
    }

    /// The contract with its two clauses: the settled change (`text`) and the detail (`error`).
    fn put_both(p: &mut Project, text: &str, error: &str) {
        let one = |n: u128, label: &str, text: &str| {
            (
                ClauseId::from_uuid(test_uuid(n)),
                ClauseContent::new(
                    ClauseLabel::try_from(label).unwrap(),
                    ClauseText::try_from(text).unwrap(),
                ),
            )
        };
        p.put_contract(
            ContextId::from_uuid(test_uuid(1)),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![one(30, "C1", text), one(31, "C2", error)],
        )
        .unwrap();
    }

    fn put_contract(p: &mut Project, text: &str) {
        put_both(p, text, "a refusal is InvalidDiscount");
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

    fn cr(p: &Project, crux: Option<Crux>) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            ChangeRequestDraft {
                title: Title::try_from("How is a half rounded?").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                lines: revisions_now(p, [clause()]),
                crux,
                addresses: Vec::new(),
            },
        )
    }

    /// A settled change (C1, its line) and a question about a detail (which error type; C2 for a new one).
    fn person(p: &Project) -> ChangeRequest {
        let crux = Crux::new(
            CruxKind::Ambiguity,
            vec![
                CruxOption::new(choice("up"), None, BTreeSet::new(), None),
                CruxOption::new(
                    choice("to even"),
                    Some(Prose::try_from("C2 names a new error").unwrap()),
                    BTreeSet::from([detail()]),
                    None,
                ),
            ],
            Some(Reasoned::new(choice("up"), None)),
            None,
        )
        .unwrap();
        cr(p, Some(crux))
    }

    fn chosen(c: &str) -> Decision {
        Decision::Chosen(Reasoned::new(choice(c), None))
    }

    #[test]
    fn the_default_still_requires_the_lines() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("up"), &p).unwrap();
        let unchanged = || Unapplied {
            cr: c.id(),
            unchanged: vec![clause()],
            gone: Vec::new(),
        };
        assert_eq!(unapplied([&c], &p), [unchanged()]);
        assert_eq!(
            resolve(&mut c, &p),
            Err(ChangeRequestError::NotApplied {
                unchanged: vec![clause()],
                gone: Vec::new()
            })
        );
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn a_change_the_chosen_option_asks_is_required_besides_the_lines() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                cr: c.id(),
                unchanged: vec![clause(), detail()],
                gone: Vec::new()
            }]
        );
        // The line alone is not enough.
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        assert_eq!(
            resolve(&mut c, &p),
            Err(ChangeRequestError::NotApplied {
                unchanged: vec![detail()],
                gone: Vec::new()
            })
        );
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidRate",
        );
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p).unwrap();
    }

    #[test]
    fn a_new_decision_replaces_what_the_earlier_one_asked() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        decide(&mut c, chosen("up"), &p).unwrap();
        // Only the line is behind now; the detail the first decision asked is not.
        assert_eq!(unapplied([&c], &p)[0].unchanged, [clause()]);
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        resolve(&mut c, &p).unwrap();
    }

    #[test]
    fn a_change_request_with_no_crux_waits_for_its_lines_to_change() {
        let mut p = project();
        let mut c = cr(&p, None);
        assert_eq!(unapplied([&c], &p).len(), 1);
        assert!(resolve(&mut c, &p).is_err());
        put_contract(&mut p, "a half rounds up, to whole cents");
        resolve(&mut c, &p).unwrap();
        assert!(unapplied([&c], &p).is_empty());
    }

    #[test]
    fn an_undecided_crux_is_not_resolved_even_when_the_lines_changed_and_is_awaited() {
        let mut p = project();
        let c = person(&p);
        assert_eq!(awaiting_decision([&c]), [&c]);
        // Its lines are behind too: it is reported for both.
        assert_eq!(unapplied([&c], &p).len(), 1);
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        assert!(unapplied([&c], &p).is_empty());
        assert_eq!(awaiting_decision([&c]), [&c]);
        let mut c = c;
        assert_eq!(resolve(&mut c, &p), Err(ChangeRequestError::Undecided));
        decide(&mut c, chosen("up"), &p).unwrap();
        assert!(awaiting_decision([&c]).is_empty());
        resolve(&mut c, &p).unwrap();
        assert!(awaiting_decision([&c]).is_empty());
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
                gone: vec![clause(), detail()]
            }]
        );
    }
}
