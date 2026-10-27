//! Deciding a Crux against the change request it stops and the Project, and reading what a Crux replaces.
//!
//! A Crux is answered once. The answer is a decision of a decision maker, and the option it chooses may ask more of
//! targets, which the change request the Crux was raised in records. That change request is another aggregate, so
//! the decision is made here, over both.

use super::change_request::revisions_now;
use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestError};
use crate::domain::entity::crux::Crux;
use crate::domain::entity::project::Project;
use crate::domain::vo::answer::{Answer, Basis};
use crate::domain::vo::crux::Decision;
use crate::domain::vo::id::CruxId;

/// Gives `crux` its answer, once, and records in `cr` what the option chosen asks of targets, each at the revision it
/// has in `project` now, with the answer that asked for it. `cr` is the change request `crux` was raised in.
/// `replaced` is the Crux `crux` supersedes, as the caller found it (none when `crux` supersedes none, or the Crux it
/// names is not there).
///
/// A decision that replaces another takes back what the one replaced asked: when `replaced` was raised in `cr` too,
/// the expectations recorded from its answer are removed from `cr` before those of this decision are recorded, so
/// that the new decision may ask of a target the other kind than the old one did. A withdrawal replaces nothing (it
/// ends the question, it does not give another decision), so it takes nothing back. A Crux replaced that was raised in
/// another change request leaves that change request's expectations as they are: they are its history, and what this
/// decision asks is recorded in `cr`, the change request this Crux was raised in.
///
/// An answer on the basis of a Policy line must cite a line the Policy of `project` has, at the revision it has now
/// (the line applied is the line as it reads at the decision). A Crux that is answered already takes no other
/// answer.
///
/// A resolved change request is a closed record: the question it was raised in may still be withdrawn, which asks
/// nothing of any target, and any other decision is refused (`AlreadyResolved`) until the change request is
/// reopened. A target the option asks to be kept, or revised, with the other kind than `cr` or an earlier decision
/// (not one taken back) asks it, is refused (see [`ChangeRequest::record_decided_expects`]).
///
/// Nothing is changed when it is refused.
pub fn decide(
    crux: &mut Crux,
    cr: &mut ChangeRequest,
    answer: Answer,
    project: &Project,
    replaced: Option<&Crux>,
) -> Result<(), ChangeRequestError> {
    debug_assert_eq!(
        crux.raised_in(),
        cr.id(),
        "the Crux is decided in its change request"
    );
    debug_assert!(
        replaced.is_none_or(|q| Some(q.id()) == crux.supersedes()),
        "the Crux replaced is the one it supersedes"
    );
    // A decision is never rewritten, whatever the basis of the new one and the state of the change request: that is
    // said first.
    if !crux.is_open() {
        return Err(ChangeRequestError::AlreadyAnswered(crux.id()));
    }
    if let Basis::Policy {
        line,
        policy_revision,
    } = answer.basis()
    {
        let policy = project.policy();
        if policy.line(*line).is_none() {
            return Err(ChangeRequestError::UnknownPolicyLine(*line));
        }
        if *policy_revision != policy.revision() {
            return Err(ChangeRequestError::PolicyRevisionMismatch {
                recorded: *policy_revision,
                now: policy.revision(),
            });
        }
    }
    let withdrawn = matches!(answer.decision(), Decision::Withdrawn);
    if cr.is_resolved() && !withdrawn {
        return Err(ChangeRequestError::AlreadyResolved);
    }
    let answer_id = answer.id();
    let mut answered = crux.clone();
    answered.answer(answer)?;
    if !cr.is_resolved() {
        // The request is changed on a copy, so that the take-back and the record are one step: either both stand or
        // neither does.
        let mut next = cr.clone();
        if !withdrawn
            && let Some(replaced_answer) = replaced
                .filter(|q| q.raised_in() == cr.id())
                .and_then(Crux::answer_given)
        {
            next.take_back_decided_expects(replaced_answer.id())?;
        }
        let asked = answered.decided_expects();
        let now = revisions_now(project, asked.keys().copied());
        next.record_decided_expects(&asked, &now, answer_id)?;
        *cr = next;
    }
    *crux = answered;
    Ok(())
}

/// Whether a Crux may supersede the Crux `replaced`, which the caller looked up in the store (`found`; none: there is
/// none). It must exist and have a decision to replace. A withdrawal is a decision, but there is nothing in it to
/// replace, so it is not one. What was decided is not rewritten; a different decision is a new Crux that points at
/// the one it replaces.
pub fn check_supersedes(replaced: CruxId, found: Option<&Crux>) -> Result<(), ChangeRequestError> {
    let crux = found.ok_or(ChangeRequestError::SupersedesUnknown(replaced))?;
    match crux.decision() {
        Some(Decision::Withdrawn) | None => Err(ChangeRequestError::SupersedesUnanswered(replaced)),
        Some(_) => Ok(()),
    }
}

/// The Cruxes among `cruxes` that have no decision yet: a decision maker is awaited, and the change request each was
/// raised in waits, resolved or not.
pub fn awaiting_decision<'a>(cruxes: impl IntoIterator<Item = &'a Crux>) -> Vec<&'a Crux> {
    cruxes.into_iter().filter(|q| q.is_open()).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::entity::change_request::ChangeRequestDraft;
    use crate::domain::entity::crux::CruxDraft;
    use crate::domain::vo::answer::DecisionMaker;
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::crux::{CruxKind, CruxOption, Recommended};
    use crate::domain::vo::id::{
        AnswerId, ChangeRequestId, ClauseId, ContextId, ContractId, CruxOptionId, ProjectId,
        test_uuid,
    };
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::policy::PolicyLineId;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::resolution::{Reopening, Resolution, ResolutionKind};
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::target::{ExpectKind, Target};
    use crate::domain::vo::timestamp::Timestamp;
    use crate::domain::vo::title::Title;

    use crate::domain::entity::context::Context;

    fn prose(s: &str) -> Prose {
        Prose::try_from(s).unwrap()
    }

    fn option(n: u128) -> CruxOptionId {
        CruxOptionId::from_uuid(test_uuid(1000 + n))
    }

    fn clause(n: u128) -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(n)))
    }

    /// A Project with a Contract of two Clauses (30 and 31), so that both have a revision.
    fn project() -> Project {
        let core = ContextId::from_uuid(test_uuid(1));
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                core,
                ContextName::try_from("core").unwrap(),
                Default::default(),
            )
            .unwrap(),
        )
        .unwrap();
        let one = |n: u128, label: &str| {
            (
                ClauseId::from_uuid(test_uuid(n)),
                ClauseContent::new(
                    ClauseLabel::try_from(label).unwrap(),
                    ClauseText::try_from("a half rounds up").unwrap(),
                ),
            )
        };
        p.put_contract(
            core,
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![one(30, "C1"), one(31, "C2")],
        )
        .unwrap();
        p
    }

    fn cr() -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            ChangeRequestDraft {
                title: Title::try_from("How is a half rounded?").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: BTreeMap::new(),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        )
    }

    /// `up` (1), and `to even` (2) which asks for the Clause 31 to be revised and the Clause 30 to be kept.
    fn crux_in(cr: &ChangeRequest, id: u128, kind: CruxKind) -> Crux {
        Crux::raise(
            CruxId::from_uuid(test_uuid(id)),
            CruxDraft {
                raised_in: cr.id(),
                findings: Vec::new(),
                kind,
                question: prose("How is a half rounded?"),
                options: vec![
                    CruxOption::new(
                        option(1),
                        Choice::try_from("up").unwrap(),
                        None,
                        BTreeMap::new(),
                        None,
                    ),
                    CruxOption::new(
                        option(2),
                        Choice::try_from("to even").unwrap(),
                        None,
                        BTreeMap::from([
                            (clause(31), ExpectKind::Target),
                            (clause(30), ExpectKind::Keep),
                        ]),
                        None,
                    ),
                ],
                recommended: Some(Recommended::new(option(1), None, None)),
                importance: None,
                supersedes: None,
            },
        )
        .unwrap()
    }

    fn answer_on(decision: Decision, basis: Basis) -> Answer {
        Answer::new(
            AnswerId::from_uuid(test_uuid(900)),
            decision,
            basis,
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(5),
        )
    }

    fn arbitrary() -> Basis {
        Basis::Arbitrary {
            reason: prose("nothing derives it"),
        }
    }

    fn chosen(n: u128) -> Answer {
        answer_on(
            Decision::Chosen {
                option: option(n),
                reason: None,
            },
            arbitrary(),
        )
    }

    fn line_id(text: &str) -> PolicyLineId {
        PolicyLineId::try_from(text).unwrap()
    }

    #[test]
    fn a_decision_on_a_policy_basis_must_cite_a_line_the_policy_has() {
        let mut p = project();
        let mut c = cr();
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        let on = |line: &str, revision| {
            answer_on(
                Decision::Chosen {
                    option: option(1),
                    reason: None,
                },
                Basis::Policy {
                    line: line_id(line),
                    policy_revision: revision,
                },
            )
        };
        // The policy has no lines yet.
        assert_eq!(
            decide(&mut q, &mut c, on("P1", p.policy().revision()), &p, None),
            Err(ChangeRequestError::UnknownPolicyLine(line_id("P1")))
        );
        assert!(q.is_open());
        p.put_policy(vec![(
            line_id("P1"),
            prose("take the recommendation from 80"),
        )])
        .unwrap();
        // A line it does not have, and a revision it is not at, are refused.
        assert_eq!(
            decide(&mut q, &mut c, on("P2", p.policy().revision()), &p, None),
            Err(ChangeRequestError::UnknownPolicyLine(line_id("P2")))
        );
        assert_eq!(
            decide(&mut q, &mut c, on("P1", Revision::first()), &p, None),
            Err(ChangeRequestError::PolicyRevisionMismatch {
                recorded: Revision::first(),
                now: p.policy().revision()
            })
        );
        assert!(q.is_open());
        // The line it has, at the revision it has: taken, and the revision is recorded.
        let basis = p.policy().basis_on(line_id("P1")).unwrap();
        decide(
            &mut q,
            &mut c,
            answer_on(
                Decision::Chosen {
                    option: option(1),
                    reason: None,
                },
                basis.clone(),
            ),
            &p,
            None,
        )
        .unwrap();
        assert_eq!(q.answer_given().map(Answer::basis), Some(&basis));
    }

    #[test]
    fn the_option_chosen_is_recorded_in_the_change_request_at_the_revision_now() {
        let p = project();
        let mut c = cr();
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        decide(&mut q, &mut c, chosen(2), &p, None).unwrap();
        assert!(!q.is_open());
        let recorded = c.decided_expects();
        assert_eq!(recorded.len(), 2);
        let kind_of = |t| recorded.iter().find(|d| d.target == t).map(|d| d.kind);
        assert_eq!(kind_of(clause(31)), Some(ExpectKind::Target));
        assert_eq!(kind_of(clause(30)), Some(ExpectKind::Keep));
        // An option that asks nothing records nothing.
        let mut c = cr();
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        decide(&mut q, &mut c, chosen(1), &p, None).unwrap();
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn a_second_decision_is_refused_and_what_the_first_named_still_stands() {
        let p = project();
        let mut c = cr();
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        decide(&mut q, &mut c, chosen(2), &p, None).unwrap();
        let asked = c.decided_expects().to_vec();
        assert_eq!(
            decide(&mut q, &mut c, chosen(1), &p, None),
            Err(ChangeRequestError::AlreadyAnswered(q.id()))
        );
        assert_eq!(c.decided_expects(), asked);
        assert_eq!(q.decision(), Some(chosen(2).decision()));
    }

    #[test]
    fn a_decision_that_the_crux_or_the_change_request_refuses_changes_neither() {
        let p = project();
        let mut c = cr();
        let mut q = crux_in(&c, 500, CruxKind::Model);
        let free = answer_on(Decision::Unspecified { keep: Vec::new() }, arbitrary());
        assert_eq!(
            decide(&mut q, &mut c, free, &p, None),
            Err(ChangeRequestError::UnspecifiedNotAllowed("model"))
        );
        assert_eq!(
            decide(&mut q, &mut c, chosen(9), &p, None),
            Err(ChangeRequestError::NotAnOption(option(9)))
        );
        assert!(q.is_open() && c.decided_expects().is_empty());
        // The change request asks the Clause 31 to be kept already; `to even` asks it to be revised.
        let mut c = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            ChangeRequestDraft {
                title: Title::try_from("T").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: crate::domain::service::change_request::expectations_now(
                    &p,
                    [(clause(31), ExpectKind::Keep)],
                ),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        assert_eq!(
            decide(&mut q, &mut c, chosen(2), &p, None),
            Err(ChangeRequestError::ExpectationConflict(clause(31)))
        );
        assert!(q.is_open() && c.decided_expects().is_empty());
    }

    #[test]
    fn a_resolved_change_request_takes_a_withdrawal_and_no_other_decision() {
        let p = project();
        let mut c = cr();
        // A request that is not done may end with its question open.
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        c.resolve(Resolution::new(
            ResolutionKind::WontDo,
            prose("not needed"),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(7),
        ))
        .unwrap();
        assert_eq!(
            decide(&mut q, &mut c, chosen(2), &p, None),
            Err(ChangeRequestError::AlreadyResolved)
        );
        assert!(q.is_open());
        decide(
            &mut q,
            &mut c,
            answer_on(Decision::Withdrawn, arbitrary()),
            &p,
            None,
        )
        .unwrap();
        assert!(!q.is_open());
        assert!(c.decided_expects().is_empty());
        // Reopened, it takes any decision again.
        c.reopen(Reopening::new(
            prose("it came back"),
            DecisionMaker::person("bob").unwrap(),
            Timestamp::from_millis(8),
        ))
        .unwrap();
        let mut again = crux_in(&c, 501, CruxKind::Ambiguity);
        decide(&mut again, &mut c, chosen(2), &p, None).unwrap();
        assert_eq!(c.decided_expects().len(), 2);
    }

    #[test]
    fn a_decided_crux_says_it_is_decided_before_it_says_its_change_request_is_resolved() {
        let p = project();
        let mut c = cr();
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        decide(&mut q, &mut c, chosen(1), &p, None).unwrap();
        c.resolve(Resolution::new(
            ResolutionKind::Done,
            prose("done"),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(7),
        ))
        .unwrap();
        for again in [chosen(2), answer_on(Decision::Withdrawn, arbitrary())] {
            assert_eq!(
                decide(&mut q, &mut c, again, &p, None),
                Err(ChangeRequestError::AlreadyAnswered(q.id()))
            );
        }
    }

    #[test]
    fn a_crux_may_supersede_only_a_crux_that_exists_and_has_a_decision_to_replace() {
        let p = project();
        let mut c = cr();
        let open = crux_in(&c, 501, CruxKind::Ambiguity);
        let mut chosen_one = crux_in(&c, 502, CruxKind::Ambiguity);
        let mut withdrawn = crux_in(&c, 503, CruxKind::Ambiguity);
        decide(
            &mut withdrawn,
            &mut c,
            answer_on(Decision::Withdrawn, arbitrary()),
            &p,
            None,
        )
        .unwrap();
        decide(&mut chosen_one, &mut c, chosen(1), &p, None).unwrap();
        // It does not exist.
        let ghost = CruxId::from_uuid(test_uuid(99));
        assert_eq!(
            check_supersedes(ghost, None),
            Err(ChangeRequestError::SupersedesUnknown(ghost))
        );
        // It has no decision, or its decision is a withdrawal.
        assert_eq!(
            check_supersedes(open.id(), Some(&open)),
            Err(ChangeRequestError::SupersedesUnanswered(open.id()))
        );
        assert_eq!(
            check_supersedes(withdrawn.id(), Some(&withdrawn)),
            Err(ChangeRequestError::SupersedesUnanswered(withdrawn.id()))
        );
        // It has a decision.
        assert_eq!(check_supersedes(chosen_one.id(), Some(&chosen_one)), Ok(()));
    }

    /// A Crux raised in `cr` with one option (1) that asks `asks`, and the Crux `replaces` if it has one.
    fn crux_asking(
        cr: &ChangeRequest,
        id: u128,
        asks: &[(Target, ExpectKind)],
        replaces: Option<&Crux>,
    ) -> Crux {
        Crux::raise(
            CruxId::from_uuid(test_uuid(id)),
            CruxDraft {
                raised_in: cr.id(),
                findings: Vec::new(),
                kind: CruxKind::Ambiguity,
                question: prose("How is a half rounded?"),
                options: vec![CruxOption::new(
                    option(1),
                    Choice::try_from("this").unwrap(),
                    None,
                    asks.iter().copied().collect(),
                    None,
                )],
                recommended: None,
                importance: None,
                supersedes: replaces.map(Crux::id),
            },
        )
        .unwrap()
    }

    /// An answer with the id `id` that chooses the option 1.
    fn chosen_with(id: u128) -> Answer {
        Answer::new(
            AnswerId::from_uuid(test_uuid(id)),
            Decision::Chosen {
                option: option(1),
                reason: None,
            },
            arbitrary(),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(5),
        )
    }

    #[test]
    fn a_crux_that_supersedes_a_decision_of_the_same_request_takes_back_what_it_asked() {
        let p = project();
        let mut c = cr();
        let mut a = crux_asking(&c, 501, &[(clause(30), ExpectKind::Target)], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        assert_eq!(c.decided_expects()[0].kind, ExpectKind::Target);
        // Another decision, which stands.
        let mut other = crux_asking(&c, 503, &[(clause(31), ExpectKind::Keep)], None);
        decide(&mut other, &mut c, chosen_with(903), &p, None).unwrap();
        // The Crux that supersedes `a` keeps what `a` asked to be revised: not a clash, as `a` no longer stands.
        let mut b = crux_asking(&c, 502, &[(clause(30), ExpectKind::Keep)], Some(&a));
        decide(&mut b, &mut c, chosen_with(902), &p, Some(&a)).unwrap();
        let held = c.decided_expects();
        assert_eq!(held.len(), 2);
        let of = |t| held.iter().find(|d| d.target == t).unwrap();
        assert_eq!(of(clause(30)).kind, ExpectKind::Keep);
        assert_eq!(of(clause(30)).answer, AnswerId::from_uuid(test_uuid(902)));
        assert_eq!(of(clause(31)).answer, AnswerId::from_uuid(test_uuid(903)));
        // Without the Crux it replaces, the same decision is the clash it was.
        let mut c2 = cr();
        let mut a2 = crux_asking(&c2, 501, &[(clause(30), ExpectKind::Target)], None);
        decide(&mut a2, &mut c2, chosen_with(901), &p, None).unwrap();
        let mut b2 = crux_asking(&c2, 502, &[(clause(30), ExpectKind::Keep)], Some(&a2));
        assert_eq!(
            decide(&mut b2, &mut c2, chosen_with(902), &p, None),
            Err(ChangeRequestError::ExpectationConflict(clause(30)))
        );
    }

    #[test]
    fn a_crux_that_supersedes_one_of_two_decisions_that_keep_a_target_leaves_the_keep_of_the_other()
    {
        let p = project();
        let mut c = cr();
        let mut a = crux_asking(&c, 501, &[(clause(30), ExpectKind::Keep)], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        // An independent Crux asks the same keep: it is recorded as its own entry.
        let mut b = crux_asking(&c, 502, &[(clause(30), ExpectKind::Keep)], None);
        decide(&mut b, &mut c, chosen_with(902), &p, None).unwrap();
        assert_eq!(c.decided_expects().len(), 2);
        // A third supersedes the first and asks something else.
        let mut third = crux_asking(&c, 503, &[(clause(31), ExpectKind::Target)], Some(&a));
        decide(&mut third, &mut c, chosen_with(903), &p, Some(&a)).unwrap();
        let held = c.decided_expects();
        assert_eq!(held.len(), 2);
        let keep = held.iter().find(|d| d.target == clause(30)).unwrap();
        assert_eq!(keep.kind, ExpectKind::Keep);
        assert_eq!(keep.answer, AnswerId::from_uuid(test_uuid(902)));
        assert_eq!(c.kind_asked(&clause(30), None), Some(ExpectKind::Keep));
    }

    #[test]
    fn a_supersession_is_one_step_and_a_withdrawal_replaces_nothing() {
        let p = project();
        // The new decision clashes with what the request itself asks of C2 (31): it is refused, and what `a` asked is
        // not taken back.
        let mut c = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            ChangeRequestDraft {
                title: Title::try_from("T").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: crate::domain::service::change_request::expectations_now(
                    &p,
                    [(clause(31), ExpectKind::Keep)],
                ),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let mut a = crux_asking(&c, 501, &[(clause(30), ExpectKind::Target)], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        let held = c.decided_expects().to_vec();
        let mut b = crux_asking(&c, 502, &[(clause(31), ExpectKind::Target)], Some(&a));
        assert_eq!(
            decide(&mut b, &mut c, chosen_with(902), &p, Some(&a)),
            Err(ChangeRequestError::ExpectationConflict(clause(31)))
        );
        assert!(b.is_open());
        assert_eq!(c.decided_expects(), held);
        assert_eq!(held.len(), 1);
        // A withdrawal ends the question and gives no other decision: what `a` asked stands.
        let mut w = crux_asking(&c, 503, &[], Some(&a));
        decide(
            &mut w,
            &mut c,
            answer_on(Decision::Withdrawn, arbitrary()),
            &p,
            Some(&a),
        )
        .unwrap();
        assert_eq!(c.decided_expects(), held);
    }

    #[test]
    fn a_crux_that_supersedes_a_decision_of_another_request_leaves_that_request_alone() {
        let p = project();
        let mut elsewhere = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(2)),
            ChangeRequestDraft {
                title: Title::try_from("Elsewhere").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: BTreeMap::new(),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let mut a = crux_asking(&elsewhere, 501, &[(clause(30), ExpectKind::Target)], None);
        decide(&mut a, &mut elsewhere, chosen_with(901), &p, None).unwrap();
        let history = elsewhere.clone();
        let mut here = cr();
        // The request here asks nothing of C1 yet; the replaced decision is another request's.
        let mut b = crux_asking(&here, 502, &[(clause(30), ExpectKind::Keep)], Some(&a));
        decide(&mut b, &mut here, chosen_with(902), &p, Some(&a)).unwrap();
        assert_eq!(here.decided_expects()[0].kind, ExpectKind::Keep);
        assert_eq!(elsewhere, history);
    }

    #[test]
    fn the_cruxes_awaiting_a_decision_are_the_open_ones() {
        let p = project();
        let mut c = cr();
        let mut answered = crux_in(&c, 501, CruxKind::Ambiguity);
        let open = crux_in(&c, 502, CruxKind::Conflict);
        decide(&mut answered, &mut c, chosen(1), &p, None).unwrap();
        let all = [answered, open.clone()];
        assert_eq!(awaiting_decision(&all), [&open]);
    }
}
