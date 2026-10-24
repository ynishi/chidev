//! Deciding and resolving a change request against the Project, and finding the ones the targets have not
//! followed.
//!
//! An expectation is met by comparing the revision its target has now (`revision_of`) with the one it recorded:
//! a change when the revision is past it, a removal when the target is gone, a keep while it is the same.

use std::collections::BTreeMap;

use super::cite::revision_of;
use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestError, ChangeRequestState,
};
use crate::domain::entity::project::Project;
use crate::domain::vo::crux::Decision;
use crate::domain::vo::id::ChangeRequestId;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::{ExpectKind, Expectation, Target};

/// The revision each of `targets` has in `project` now; a target the Project does not have is left out.
pub fn revisions_now(
    project: &Project,
    targets: impl IntoIterator<Item = Target>,
) -> BTreeMap<Target, Revision> {
    targets
        .into_iter()
        .filter_map(|t| revision_of(project, t).map(|r| (t, r)))
        .collect()
}

/// The expectations `wanted` as a change request records them: each target with what is expected of it and the
/// revision it has in `project` now. A target the Project does not have is left out; the caller reports it.
pub fn expectations_now(
    project: &Project,
    wanted: impl IntoIterator<Item = (Target, ExpectKind)>,
) -> BTreeMap<Target, Expectation> {
    wanted
        .into_iter()
        .filter_map(|(t, kind)| {
            revision_of(project, t).map(|recorded| (t, Expectation { kind, recorded }))
        })
        .collect()
}

/// Gives the Crux of `cr` the person's decision, with each target the option chosen expects something of at the
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
        .flat_map(|o| o.expects().keys().copied());
    let now = revisions_now(project, targets);
    cr.decide(decision, &now)
}

/// Where the targets of one change request stand against a Project.
#[derive(Debug, Default)]
struct Standing {
    /// To change, and still at the recorded revision.
    unchanged: Vec<Target>,
    /// To change, and gone.
    gone: Vec<Target>,
    /// To remove, and still there.
    not_removed: Vec<Target>,
    /// To keep, and not at the recorded revision (or gone).
    moved: Vec<Target>,
}

/// Reads every expectation of `cr` (its own and those its decision asked) against `project`. A target in the
/// lists once.
fn standing(cr: &ChangeRequest, project: &Project) -> Standing {
    let mut s = Standing::default();
    for (target, expectation) in cr.expectations() {
        let now = revision_of(project, target);
        if expectation.is_met_by(now) {
            continue;
        }
        let list = match (expectation.kind, now) {
            (ExpectKind::Change, Some(_)) => &mut s.unchanged,
            (ExpectKind::Change, None) => &mut s.gone,
            (ExpectKind::Remove, _) => &mut s.not_removed,
            (ExpectKind::Keep, _) => &mut s.moved,
        };
        if !list.contains(&target) {
            list.push(target);
        }
    }
    s
}

/// The change requests, in `crs`, that are a part of `parent` (their parent is it) and are not resolved.
pub fn open_parts(parent: ChangeRequestId, crs: &[ChangeRequest]) -> Vec<ChangeRequestId> {
    crs.iter()
        .filter(|c| c.parent() == Some(parent) && c.state() != ChangeRequestState::Resolved)
        .map(|c| c.id())
        .collect()
}

/// Closes the record of `cr` once the change is applied: its Crux, if it has one, is decided; every expectation to
/// change or remove, its own and those of the decision, is met; no expectation to keep has been broken; and every
/// change request in `all` that is a part of it is resolved. A change request is a record of its own, so `all`
/// is every change request there is (`cr` itself may be among them).
pub fn resolve(
    cr: &mut ChangeRequest,
    project: &Project,
    all: &[ChangeRequest],
) -> Result<(), ChangeRequestError> {
    if !cr.awaits_decision() {
        let s = standing(cr, project);
        if !s.unchanged.is_empty() || !s.gone.is_empty() || !s.not_removed.is_empty() {
            return Err(ChangeRequestError::NotApplied {
                unchanged: s.unchanged,
                gone: s.gone,
                not_removed: s.not_removed,
            });
        }
        if !s.moved.is_empty() {
            return Err(ChangeRequestError::KeepMoved { moved: s.moved });
        }
        let children = open_parts(cr.id(), all);
        if !children.is_empty() {
            return Err(ChangeRequestError::ChildrenNotResolved { children });
        }
    }
    cr.resolve()
}

/// An unresolved change request the targets have not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapplied {
    /// The change request.
    pub cr: ChangeRequestId,
    /// The targets to change that are still at the revision it recorded, in order.
    pub unchanged: Vec<Target>,
    /// The targets to change that the Project does not have any more, in order.
    pub gone: Vec<Target>,
    /// The targets to remove that are still in the Project, in order.
    pub not_removed: Vec<Target>,
}

/// Every unresolved change request in `crs` with an expectation to change or remove, its own or one its decision
/// asked, that the Project does not meet. One whose Crux is not decided yet is here for its own expectations, and
/// also in [`awaiting_decision`]. An expectation to keep that was broken is not here: see [`keep_moved`].
pub fn unapplied<'a>(
    crs: impl IntoIterator<Item = &'a ChangeRequest>,
    project: &Project,
) -> Vec<Unapplied> {
    crs.into_iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved)
        .filter_map(|cr| {
            let s = standing(cr, project);
            (!s.unchanged.is_empty() || !s.gone.is_empty() || !s.not_removed.is_empty()).then(
                || Unapplied {
                    cr: cr.id(),
                    unchanged: s.unchanged,
                    gone: s.gone,
                    not_removed: s.not_removed,
                },
            )
        })
        .collect()
}

/// An unresolved change request one of whose expectations to keep was broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepMoved {
    /// The change request.
    pub cr: ChangeRequestId,
    /// The targets it expected to keep that are not at the revision it recorded (or are gone), in order.
    pub moved: Vec<Target>,
}

/// Every unresolved change request in `crs` that expects a target to stay as it is and the target has moved on
/// in `project`. A resolved one is a record: what changes after it is another change request's business.
pub fn keep_moved<'a>(
    crs: impl IntoIterator<Item = &'a ChangeRequest>,
    project: &Project,
) -> Vec<KeepMoved> {
    crs.into_iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved)
        .filter_map(|cr| {
            let moved = standing(cr, project).moved;
            (!moved.is_empty()).then(|| KeepMoved { cr: cr.id(), moved })
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
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::crux::{CruxKind, CruxOption, Reasoned};
    use crate::domain::vo::id::{
        CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId, ProjectId,
        test_uuid,
    };
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::signature::Signature;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::title::Title;

    fn clause() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(30)))
    }

    /// The clause the question is about: which error a refusal is.
    fn detail() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(31)))
    }

    fn core() -> ContextId {
        ContextId::from_uuid(test_uuid(1))
    }

    fn billing() -> ContextId {
        ContextId::from_uuid(test_uuid(2))
    }

    fn contract() -> Target {
        Target::Contract(ContractId::from_uuid(test_uuid(300)))
    }

    fn interface() -> Target {
        Target::Interface(InterfaceId::from_uuid(test_uuid(100)))
    }

    fn item() -> Target {
        Target::Item(ItemId::from_uuid(test_uuid(10)))
    }

    fn harness() -> Target {
        Target::Harness(HarnessId::from_uuid(test_uuid(400)))
    }

    fn case() -> Target {
        Target::Case(CaseId::from_uuid(test_uuid(40)))
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
            core(),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![one(30, "C1", text), one(31, "C2", error)],
        )
        .unwrap();
    }

    fn put_contract(p: &mut Project, text: &str) {
        put_both(p, text, "a refusal is InvalidDiscount");
    }

    /// The contract with its first clause alone.
    fn put_first_only(p: &mut Project, text: &str) {
        p.put_contract(
            core(),
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

    fn put_interface(p: &mut Project, id: u128, item_id: u128, sig: &str) {
        p.put_interface(
            core(),
            InterfaceId::from_uuid(test_uuid(id)),
            Source::new(format!("{sig}\n")),
            vec![(
                ItemId::from_uuid(test_uuid(item_id)),
                Signature::try_from(sig).unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
    }

    fn put_harness(p: &mut Project, body: &str) {
        p.put_harness(
            core(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new(format!("{body}\n")),
            vec![(
                CaseId::from_uuid(test_uuid(40)),
                CaseName::try_from("t").unwrap(),
                LineRange::new(1, 1).unwrap(),
                CaseCites::new(
                    BTreeMap::from([(ItemId::from_uuid(test_uuid(10)), Revision::first())]),
                    BTreeMap::new(),
                )
                .unwrap(),
            )],
        )
        .unwrap();
    }

    fn context(id: ContextId, name: &str) -> Context {
        Context::new(id, ContextName::try_from(name).unwrap(), BTreeSet::new()).unwrap()
    }

    /// A Project with the Context `core`: an Interface with one Item, the Contract with two Clauses, and a Harness
    /// with one Case.
    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(context(core(), "core")).unwrap();
        put_interface(&mut p, 100, 10, "fn a()");
        put_contract(&mut p, "a half rounds up");
        put_harness(&mut p, "fn t() {}");
        p
    }

    fn choice(s: &str) -> Choice {
        Choice::try_from(s).unwrap()
    }

    fn draft(p: &Project, intent: Intent, wanted: &[(Target, ExpectKind)]) -> ChangeRequestDraft {
        ChangeRequestDraft {
            title: Title::try_from("How is a half rounded?").unwrap(),
            situation: None,
            intent,
            parent: None,
            issuer: None,
            expects: expectations_now(p, wanted.iter().copied()),
            crux: None,
            addresses: Vec::new(),
        }
    }

    fn open(
        p: &Project,
        n: u128,
        intent: Intent,
        wanted: &[(Target, ExpectKind)],
    ) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(n)),
            draft(p, intent, wanted),
        )
    }

    fn cr(p: &Project, crux: Option<Crux>) -> ChangeRequest {
        let mut d = draft(p, Intent::ChangeBehavior, &[(clause(), ExpectKind::Change)]);
        d.crux = crux;
        ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(1)), d)
    }

    /// A settled change (C1, its own expectation) and a question about a detail (which error type; C2 for a new
    /// one).
    fn person(p: &Project) -> ChangeRequest {
        let crux = Crux::new(
            CruxKind::Ambiguity,
            vec![
                CruxOption::new(choice("up"), None, BTreeMap::new(), None),
                CruxOption::new(
                    choice("to even"),
                    Some(Prose::try_from("C2 names a new error").unwrap()),
                    BTreeMap::from([(detail(), ExpectKind::Change)]),
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
    fn the_default_still_requires_the_own_expectations() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("up"), &p).unwrap();
        let unchanged = || Unapplied {
            cr: c.id(),
            unchanged: vec![clause()],
            gone: Vec::new(),
            not_removed: Vec::new(),
        };
        assert_eq!(unapplied([&c], &p), [unchanged()]);
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::NotApplied {
                unchanged: vec![clause()],
                gone: Vec::new(),
                not_removed: Vec::new()
            })
        );
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn what_the_chosen_option_expects_is_required_besides_the_own_expectations() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                cr: c.id(),
                unchanged: vec![clause(), detail()],
                gone: Vec::new(),
                not_removed: Vec::new()
            }]
        );
        // The own expectation alone is not enough.
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::NotApplied {
                unchanged: vec![detail()],
                gone: Vec::new(),
                not_removed: Vec::new()
            })
        );
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidRate",
        );
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_new_decision_replaces_what_the_earlier_one_asked() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        decide(&mut c, chosen("up"), &p).unwrap();
        // Only the own expectation is behind now; the detail the first decision asked is not.
        assert_eq!(unapplied([&c], &p)[0].unchanged, [clause()]);
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_change_request_with_no_crux_waits_for_its_expectations_to_be_met() {
        let mut p = project();
        let mut c = cr(&p, None);
        assert_eq!(unapplied([&c], &p).len(), 1);
        assert!(resolve(&mut c, &p, &[]).is_err());
        put_contract(&mut p, "a half rounds up, to whole cents");
        resolve(&mut c, &p, &[]).unwrap();
        assert!(unapplied([&c], &p).is_empty());
    }

    #[test]
    fn an_undecided_crux_is_not_resolved_even_when_the_expectations_are_met_and_is_awaited() {
        let mut p = project();
        let c = person(&p);
        assert_eq!(awaiting_decision([&c]), [&c]);
        // Its own expectation is behind too: it is reported for both.
        assert_eq!(unapplied([&c], &p).len(), 1);
        put_both(
            &mut p,
            "a discount above 50% is refused",
            "a refusal is InvalidDiscount",
        );
        assert!(unapplied([&c], &p).is_empty());
        assert_eq!(awaiting_decision([&c]), [&c]);
        let mut c = c;
        assert_eq!(resolve(&mut c, &p, &[]), Err(ChangeRequestError::Undecided));
        decide(&mut c, chosen("up"), &p).unwrap();
        assert!(awaiting_decision([&c]).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        assert!(awaiting_decision([&c]).is_empty());
    }

    #[test]
    fn a_target_to_change_that_is_gone_is_reported() {
        let p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(
            unapplied([&c], &empty),
            [Unapplied {
                cr: c.id(),
                unchanged: Vec::new(),
                gone: vec![clause(), detail()],
                not_removed: Vec::new()
            }]
        );
    }

    // M1: a change request that keeps the behaviour resolves, without a warning.

    #[test]
    fn a_keep_behavior_request_keeps_a_clause_and_changes_an_item_and_resolves() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(clause(), ExpectKind::Keep), (item(), ExpectKind::Change)],
        );
        assert_eq!(unapplied([&c], &p).len(), 1);
        assert!(keep_moved([&c], &p).is_empty());
        put_interface(&mut p, 100, 10, "fn a(x: u8)");
        assert!(unapplied([&c], &p).is_empty());
        assert!(keep_moved([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    // M2: adding is a change of what holds the new thing.

    #[test]
    fn a_clause_is_added_as_a_change_of_its_contract() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(contract(), ExpectKind::Change)],
        );
        assert_eq!(unapplied([&c], &p).len(), 1);
        p.put_contract(
            core(),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![
                (
                    ClauseId::from_uuid(test_uuid(30)),
                    ClauseContent::new(
                        ClauseLabel::try_from("C1").unwrap(),
                        ClauseText::try_from("a half rounds up").unwrap(),
                    ),
                ),
                (
                    ClauseId::from_uuid(test_uuid(31)),
                    ClauseContent::new(
                        ClauseLabel::try_from("C2").unwrap(),
                        ClauseText::try_from("a refusal is InvalidDiscount").unwrap(),
                    ),
                ),
                (
                    ClauseId::from_uuid(test_uuid(32)),
                    ClauseContent::new(
                        ClauseLabel::try_from("C3").unwrap(),
                        ClauseText::try_from("a total of no lines is zero").unwrap(),
                    ),
                ),
            ],
        )
        .unwrap();
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn an_interface_is_added_as_a_change_of_its_context() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(Target::Context(core()), ExpectKind::Change)],
        );
        // A change inside an asset does not move the Context.
        put_interface(&mut p, 100, 10, "fn a(x: u8)");
        put_contract(&mut p, "another promise");
        assert_eq!(unapplied([&c], &p).len(), 1);
        // A new asset does.
        put_interface(&mut p, 101, 11, "fn b()");
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_context_is_added_as_a_change_of_the_project() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(Target::Project, ExpectKind::Change)],
        );
        // A change inside a Context does not move the Project.
        put_interface(&mut p, 101, 11, "fn b()");
        assert_eq!(unapplied([&c], &p).len(), 1);
        p.add_context(context(billing(), "billing")).unwrap();
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_new_use_or_basis_moves_the_context() {
        let mut p = project();
        p.add_context(context(billing(), "billing")).unwrap();
        let c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(Target::Context(core()), ExpectKind::Change)],
        );
        p.set_uses(core(), BTreeSet::from([billing()])).unwrap();
        assert!(unapplied([&c], &p).is_empty());
        let c = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(Target::Context(core()), ExpectKind::Change)],
        );
        // The same uses again move nothing.
        p.set_uses(core(), BTreeSet::from([billing()])).unwrap();
        assert_eq!(unapplied([&c], &p).len(), 1);
        p.base_on_contract(core()).unwrap();
        assert!(unapplied([&c], &p).is_empty());
    }

    // M3: removing resolves once the target is gone.

    #[test]
    fn a_clause_is_removed_when_it_is_no_longer_in_its_contract() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(detail(), ExpectKind::Remove)],
        );
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                cr: c.id(),
                unchanged: Vec::new(),
                gone: Vec::new(),
                not_removed: vec![detail()]
            }]
        );
        // Changing it is not removing it.
        put_both(&mut p, "a half rounds up", "a refusal is InvalidRate");
        assert!(resolve(&mut c, &p, &[]).is_err());
        put_first_only(&mut p, "a half rounds up");
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_context_is_removed_when_the_project_no_longer_has_it() {
        let mut with_two = project();
        with_two.add_context(context(billing(), "billing")).unwrap();
        let mut c = open(
            &with_two,
            1,
            Intent::ChangeBehavior,
            &[(Target::Context(billing()), ExpectKind::Remove)],
        );
        assert_eq!(unapplied([&c], &with_two).len(), 1);
        let without = project();
        assert!(unapplied([&c], &without).is_empty());
        resolve(&mut c, &without, &[]).unwrap();
    }

    // M4: a Case can be changed.

    #[test]
    fn a_case_is_changed_when_its_code_changes() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(case(), ExpectKind::Change)],
        );
        assert_eq!(unapplied([&c], &p).len(), 1);
        put_harness(&mut p, "fn t() { assert!(true); }");
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        // The container and the leaf are told apart.
        let h = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[
                (harness(), ExpectKind::Change),
                (interface(), ExpectKind::Keep),
            ],
        );
        assert_eq!(unapplied([&h], &p).len(), 1);
        put_harness(&mut p, "fn t() { assert!(false); }");
        assert!(unapplied([&h], &p).is_empty());
    }

    // Keep.

    #[test]
    fn a_keep_that_moved_refuses_the_resolve_and_is_reported_apart() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(clause(), ExpectKind::Keep), (item(), ExpectKind::Change)],
        );
        put_interface(&mut p, 100, 10, "fn a(x: u8)");
        put_contract(&mut p, "a half rounds to even");
        assert!(unapplied([&c], &p).is_empty());
        assert_eq!(
            keep_moved([&c], &p),
            [KeepMoved {
                cr: c.id(),
                moved: vec![clause()]
            }]
        );
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::KeepMoved {
                moved: vec![clause()]
            })
        );
        assert_eq!(c.state(), ChangeRequestState::Open);
        // Gone is moved too.
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(keep_moved([&c], &empty)[0].moved, [clause()]);
    }

    #[test]
    fn a_resolved_request_is_a_record_and_reports_nothing() {
        let mut p = project();
        let mut c = open(&p, 1, Intent::KeepBehavior, &[(clause(), ExpectKind::Keep)]);
        resolve(&mut c, &p, &[]).unwrap();
        put_contract(&mut p, "a half rounds to even");
        assert!(keep_moved([&c], &p).is_empty());
        assert!(unapplied([&c], &p).is_empty());
    }

    // Options.

    #[test]
    fn an_option_that_removes_a_target_requires_it_gone() {
        let mut p = project();
        let crux = Crux::new(
            CruxKind::Conflict,
            vec![
                CruxOption::new(
                    choice("drop C2"),
                    None,
                    BTreeMap::from([(detail(), ExpectKind::Remove)]),
                    None,
                ),
                CruxOption::new(
                    choice("keep both"),
                    None,
                    BTreeMap::from([(detail(), ExpectKind::Keep)]),
                    None,
                ),
            ],
            None,
            None,
        )
        .unwrap();
        let mut c = cr(&p, Some(crux));
        decide(&mut c, chosen("drop C2"), &p).unwrap();
        put_contract(&mut p, "a half rounds to even");
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::NotApplied {
                unchanged: Vec::new(),
                gone: Vec::new(),
                not_removed: vec![detail()]
            })
        );
        put_first_only(&mut p, "a half rounds to even");
        resolve(&mut c, &p, &[]).unwrap();
        // Choosing the other option instead asks to keep it.
        let fresh = project();
        let mut other = cr(&fresh, c.crux().cloned());
        decide(&mut other, chosen("keep both"), &fresh).unwrap();
        assert_eq!(other.decided_expects()[&detail()].kind, ExpectKind::Keep);
    }

    // M6: a parent waits for its parts.

    #[test]
    fn a_parent_is_not_resolved_while_a_part_is_not() {
        let p = project();
        let mut parent = open(&p, 1, Intent::ChangeBehavior, &[]);
        let mut d = draft(&p, Intent::ChangeBehavior, &[]);
        d.title = Title::try_from("part").unwrap();
        d.parent = Some(parent.id());
        let mut part = ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(2)), d);
        let all = [parent.clone(), part.clone()];
        assert_eq!(open_parts(parent.id(), &all), [part.id()]);
        assert_eq!(
            resolve(&mut parent, &p, &all),
            Err(ChangeRequestError::ChildrenNotResolved {
                children: vec![part.id()]
            })
        );
        assert_eq!(parent.state(), ChangeRequestState::Open);
        resolve(&mut part, &p, &all).unwrap();
        let all = [parent.clone(), part];
        assert!(open_parts(parent.id(), &all).is_empty());
        resolve(&mut parent, &p, &all).unwrap();
        assert_eq!(parent.state(), ChangeRequestState::Resolved);
    }
}
