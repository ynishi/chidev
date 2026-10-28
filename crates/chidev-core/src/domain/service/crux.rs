//! Deciding a Crux against the change request it stops and the Project, and reading what a Crux replaces.
//!
//! A Crux is answered once. The answer is a decision of a decision maker, and the option it chooses may ask more of
//! targets, which the change request the Crux was raised in records when they are inside its frame. That change
//! request is another aggregate, so the decision is made here, over both.

use std::collections::BTreeSet;

use super::change_request::{outside_frame, revisions_now};
use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestError};
use crate::domain::entity::crux::{Crux, CruxDraft};
use crate::domain::entity::project::Project;
use crate::domain::vo::answer::{Answer, Basis, Beyond};
use crate::domain::vo::crux::Decision;
use crate::domain::vo::id::{AnswerId, ChangeRequestId, CruxId};
use crate::domain::vo::target::{ExpectKind, Target};

/// Gives `crux` its answer, once, and records in `cr` what the option chosen asks of targets, each at the revision it
/// has in `project` now, with the answer that asked for it. `cr` is the change request `crux` was raised in.
/// `replaced` is the Crux `crux` supersedes, as the caller found it (none when `crux` supersedes none, or the Crux it
/// names is not there). `others` are the change requests there are, for the ones the `beyond` of the answer names.
///
/// A decision that replaces another takes back what the one replaced asked: when `replaced` was raised in `cr` too,
/// the expectations recorded from its answer are removed from `cr` before those of this decision are recorded, so
/// that the new decision may ask of a target the other kind than the old one did. A withdrawal replaces nothing (it
/// ends the question, it does not give another decision), so it takes nothing back. A Crux replaced that was raised in
/// another change request leaves that change request's expectations as they are: they are its history, and what this
/// decision asks is recorded in `cr`, the change request this Crux was raised in.
///
/// The frame of `cr` is what it was opened with, and a decision never widens it. The targets of the option chosen
/// that are outside it ([`outside_frame`], read after the take-back) are not recorded in `cr`, and the answer must say
/// where they go (`AnswerOutside`): `Beyond::TakenBy` change requests, each not `cr`, not resolved and found among
/// `others`, that between them frame every target outside, or `Beyond::Declined`. The change requests named are
/// read whenever `TakenBy` is given, whether or not anything is outside. A withdrawal asks and adds nothing, so a
/// `beyond` on it is refused (`BeyondOnWithdrawal`).
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
    others: &[ChangeRequest],
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
    let beyond = answer.beyond().cloned();
    if withdrawn && beyond.is_some() {
        return Err(ChangeRequestError::BeyondOnWithdrawal);
    }
    let mut answered = crux.clone();
    answered.answer(answer)?;
    if !cr.is_resolved() {
        // The request is changed on a copy, so that the take-back and the record are one step: either both stand or
        // neither does.
        let mut next = cr.clone();
        if !withdrawn && let Some(replaced_answer) = taken_back(cr, replaced) {
            next.take_back_decided_expects(replaced_answer)?;
        }
        // A decision that chose no option asks nothing. What it asks to be revised or retired outside the frame is
        // not asked of this request.
        let mut outside = Vec::new();
        if let Some(option) = answered.chosen_option() {
            let revised = option
                .expects()
                .iter()
                .filter(|(_, kind)| **kind == ExpectKind::Target)
                .map(|(target, _)| *target);
            outside = outside_frame(&next, project, revised);
            if !outside.is_empty() && beyond.is_none() {
                return Err(ChangeRequestError::AnswerOutside {
                    crux: crux.id(),
                    option: option.choice().clone(),
                    targets: outside,
                });
            }
        }
        if let Some(Beyond::TakenBy(named)) = &beyond {
            check_taken_by(named, cr, others, project, &outside)?;
        }
        if let Some(option) = answered.chosen_option() {
            let now = revisions_now(project, option.expects().keys().copied());
            let leave_out: BTreeSet<Target> = outside.into_iter().collect();
            next.record_decided_expects(option, &now, answer_id, &leave_out)?;
        }
        *cr = next;
    }
    *crux = answered;
    Ok(())
}

/// Whether the change requests `named`, among `others`, may take what an answer given in `cr` adds outside it, which
/// is `outside`: each is there, is not `cr`, is not resolved, and between them they frame every target outside.
fn check_taken_by(
    named: &[ChangeRequestId],
    cr: &ChangeRequest,
    others: &[ChangeRequest],
    project: &Project,
    outside: &[Target],
) -> Result<(), ChangeRequestError> {
    let mut taking = Vec::with_capacity(named.len());
    for id in named.iter().filter(|id| **id != cr.id()) {
        taking.push(
            others
                .iter()
                .find(|c| c.id() == *id)
                .ok_or(ChangeRequestError::ReferenceUnknown(*id))?,
        );
    }
    if taking.len() < named.len() {
        return Err(ChangeRequestError::BeyondItself);
    }
    if let Some(done) = taking.iter().find(|c| c.is_resolved()) {
        return Err(ChangeRequestError::BeyondResolved(done.id()));
    }
    for target in outside {
        let framed = taking
            .iter()
            .any(|c| outside_frame(c, project, [*target]).is_empty());
        if !framed {
            return Err(ChangeRequestError::BeyondUnframed { target: *target });
        }
    }
    Ok(())
}

/// The answer whose asks no longer stand once a Crux that supersedes `replaced` is decided in `cr`: that of `replaced`
/// when it was raised in `cr` and has an answer. A Crux replaced that was raised in another change request has no
/// asks in `cr`, and an open one has none to take back. This is the one statement of the selection: [`decide`] takes
/// that answer's asks back, and [`raise`] reads the options of the new Crux against what stands without them.
pub fn taken_back(cr: &ChangeRequest, replaced: Option<&Crux>) -> Option<AnswerId> {
    replaced
        .filter(|q| q.raised_in() == cr.id())
        .and_then(Crux::answer_given)
        .map(Answer::id)
}

/// Raises a Crux in `cr`, the change request it stops, as `draft` says. `replaced` is the Crux `draft` supersedes, as
/// the caller found it (none when `draft` supersedes none, or the Crux it names is not there). `cruxes` are the
/// Cruxes there are, to see whether the decision superseded is replaced already.
///
/// It is read in this order, and the first rule broken is the refusal: the change request is resolved
/// (`AlreadyResolved`); the Crux superseded does not exist, has no decision to replace, or is replaced already
/// ([`check_supersedes`]); the Crux's own rules ([`Crux::raise`]); then each option against `cr`
/// ([`ChangeRequest::check_option`]), leaving out what the decision [`taken_back`] asked, since it no longer stands
/// once this Crux is decided.
///
/// Nothing is changed when it is refused: the change request is only read.
pub fn raise(
    id: CruxId,
    draft: CruxDraft,
    cr: &ChangeRequest,
    replaced: Option<&Crux>,
    cruxes: &[Crux],
) -> Result<Crux, ChangeRequestError> {
    debug_assert_eq!(
        draft.raised_in,
        cr.id(),
        "the Crux is raised in its change request"
    );
    debug_assert!(
        replaced.is_none_or(|q| Some(q.id()) == draft.supersedes),
        "the Crux replaced is the one it supersedes"
    );
    if cr.is_resolved() {
        return Err(ChangeRequestError::AlreadyResolved);
    }
    if let Some(superseded) = draft.supersedes {
        check_supersedes(superseded, replaced, cruxes)?;
    }
    let crux = Crux::raise(id, draft)?;
    let leaving_out = taken_back(cr, replaced);
    for option in crux.options() {
        cr.check_option(option, leaving_out)?;
    }
    Ok(crux)
}

/// The Crux among `cruxes` that replaces the decision of `replaced`: one that supersedes it and has not been
/// withdrawn, whether it is open or decided otherwise. A Crux that was withdrawn replaced nothing, so the decision
/// stands again and may be superseded anew. One decision is replaced by one Crux, but a store written before that was
/// a rule may hold two; the one with the smallest id, which is the one raised first, is the answer, so that the same
/// store always gives the same one.
pub fn superseded_by(replaced: CruxId, cruxes: &[Crux]) -> Option<&Crux> {
    cruxes
        .iter()
        .filter(|q| q.supersedes() == Some(replaced))
        .filter(|q| !matches!(q.decision(), Some(Decision::Withdrawn)))
        .min_by_key(|q| q.id())
}

/// The Crux whose decision stands for the question `crux` was raised in: `crux` itself when none replaces it, else
/// the end of the chain of [`superseded_by`]. It may be open, when the last Crux of the chain is not decided yet.
pub fn standing<'a>(crux: &'a Crux, cruxes: &'a [Crux]) -> &'a Crux {
    let mut seen = vec![crux.id()];
    let mut at = crux;
    while let Some(next) = superseded_by(at.id(), cruxes) {
        if seen.contains(&next.id()) {
            break;
        }
        seen.push(next.id());
        at = next;
    }
    at
}

/// Whether a Crux may supersede the Crux `replaced`, which the caller looked up in the store (`found`; none: there is
/// none), among the Cruxes there are (`cruxes`). It must exist and have a decision to replace. A withdrawal is a
/// decision, but there is nothing in it to replace, so it is not one. What was decided is not rewritten; a different
/// decision is a new Crux that points at the one it replaces, and only one does: a decision another Crux replaces
/// already is refused (`SupersededAlready`, naming that Crux, [`superseded_by`]); the Crux to supersede then is the
/// one that stands ([`standing`]). The rule is read here, when a Crux is raised, and not when a store is read.
pub fn check_supersedes(
    replaced: CruxId,
    found: Option<&Crux>,
    cruxes: &[Crux],
) -> Result<(), ChangeRequestError> {
    let crux = found.ok_or(ChangeRequestError::SupersedesUnknown(replaced))?;
    match crux.decision() {
        Some(Decision::Withdrawn) | None => {
            return Err(ChangeRequestError::SupersedesUnanswered(replaced));
        }
        Some(_) => {}
    }
    match superseded_by(replaced, cruxes) {
        Some(by) => Err(ChangeRequestError::SupersededAlready {
            crux: replaced,
            by: by.id(),
        }),
        None => Ok(()),
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

    // The tests that do not read the change requests there are, or the Cruxes there are, call these: the same
    // functions with none.
    fn decide(
        crux: &mut Crux,
        cr: &mut ChangeRequest,
        answer: Answer,
        project: &Project,
        replaced: Option<&Crux>,
    ) -> Result<(), ChangeRequestError> {
        super::decide(crux, cr, answer, project, replaced, &[])
    }

    fn raise(
        id: CruxId,
        draft: CruxDraft,
        cr: &ChangeRequest,
        replaced: Option<&Crux>,
    ) -> Result<Crux, ChangeRequestError> {
        super::raise(id, draft, cr, replaced, &[])
    }

    fn check_supersedes(replaced: CruxId, found: Option<&Crux>) -> Result<(), ChangeRequestError> {
        super::check_supersedes(replaced, found, &[])
    }

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

    /// A change request that asks the Contract 300 to be revised: its frame holds the Clauses 30 and 31.
    fn cr() -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            ChangeRequestDraft {
                title: Title::try_from("How is a half rounded?").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: crate::domain::service::change_request::expectations_now(
                    &project(),
                    [(contract(300), ExpectKind::Target)],
                ),
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
            Err(ChangeRequestError::UnspecifiedNotAllowed {
                crux: q.id(),
                kind: CruxKind::Model
            })
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
                    [
                        (clause(31), ExpectKind::Keep),
                        (contract(300), ExpectKind::Target),
                    ],
                ),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let mut q = crux_in(&c, 500, CruxKind::Ambiguity);
        assert_eq!(
            decide(&mut q, &mut c, chosen(2), &p, None),
            Err(ChangeRequestError::ExpectationConflict {
                option: Choice::try_from("to even").unwrap(),
                target: clause(31)
            })
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
            Err(ChangeRequestError::ExpectationConflict {
                option: Choice::try_from("this").unwrap(),
                target: clause(30)
            })
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
                    [
                        (clause(31), ExpectKind::Keep),
                        (contract(300), ExpectKind::Target),
                    ],
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
            Err(ChangeRequestError::ExpectationConflict {
                option: Choice::try_from("this").unwrap(),
                target: clause(31)
            })
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
                expects: crate::domain::service::change_request::expectations_now(
                    &p,
                    [(contract(300), ExpectKind::Target)],
                ),
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

    /// What a Crux is raised with in `cr`: the options given, and the Crux it replaces.
    fn raising(cr: &ChangeRequest, options: Vec<CruxOption>, replaces: Option<&Crux>) -> CruxDraft {
        CruxDraft {
            raised_in: cr.id(),
            findings: Vec::new(),
            kind: CruxKind::Ambiguity,
            question: prose("How is a half rounded?"),
            options,
            recommended: None,
            importance: None,
            supersedes: replaces.map(Crux::id),
        }
    }

    /// An option (the id `n`, the choice `choice`) that asks `asks`.
    fn option_asking(n: u128, choice: &str, asks: &[(Target, ExpectKind)]) -> CruxOption {
        CruxOption::new(
            option(n),
            Choice::try_from(choice).unwrap(),
            None,
            asks.iter().copied().collect(),
            None,
        )
    }

    fn new_id() -> CruxId {
        CruxId::from_uuid(test_uuid(600))
    }

    #[test]
    fn a_crux_is_raised_in_an_open_change_request_and_the_request_is_only_read() {
        let c = cr();
        let before = c.clone();
        let draft = raising(
            &c,
            vec![
                option_asking(1, "up", &[]),
                option_asking(2, "to even", &[(clause(30), ExpectKind::Target)]),
            ],
            None,
        );
        let q = raise(new_id(), draft, &c, None).unwrap();
        assert!(q.is_open());
        assert_eq!(q.id(), new_id());
        assert_eq!(q.options().len(), 2);
        assert_eq!(c, before);
    }

    #[test]
    fn a_resolved_change_request_takes_no_crux_even_one_that_is_otherwise_refused() {
        let mut c = cr();
        c.resolve(Resolution::new(
            ResolutionKind::WontDo,
            prose("not needed"),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(7),
        ))
        .unwrap();
        let ghost = Crux::raise(
            CruxId::from_uuid(test_uuid(99)),
            raising(&c, vec![option_asking(1, "up", &[])], None),
        )
        .unwrap();
        for (options, replaces) in [
            (vec![option_asking(1, "up", &[])], None),
            (Vec::new(), None),
            (vec![option_asking(1, "up", &[])], Some(&ghost)),
        ] {
            let draft = raising(&c, options, replaces);
            assert_eq!(
                raise(new_id(), draft, &c, None),
                Err(ChangeRequestError::AlreadyResolved)
            );
        }
    }

    #[test]
    fn a_crux_that_supersedes_one_that_has_no_decision_to_replace_is_refused() {
        let p = project();
        let mut c = cr();
        let ghost = CruxId::from_uuid(test_uuid(99));
        let mut draft = raising(&c, vec![option_asking(1, "up", &[])], None);
        draft.supersedes = Some(ghost);
        assert_eq!(
            raise(new_id(), draft, &c, None),
            Err(ChangeRequestError::SupersedesUnknown(ghost))
        );
        let open = crux_asking(&c, 501, &[], None);
        let draft = raising(&c, vec![option_asking(1, "up", &[])], Some(&open));
        assert_eq!(
            raise(new_id(), draft, &c, Some(&open)),
            Err(ChangeRequestError::SupersedesUnanswered(open.id()))
        );
        let mut withdrawn = crux_asking(&c, 502, &[], None);
        decide(
            &mut withdrawn,
            &mut c,
            answer_on(Decision::Withdrawn, arbitrary()),
            &p,
            None,
        )
        .unwrap();
        let draft = raising(&c, vec![option_asking(1, "up", &[])], Some(&withdrawn));
        assert_eq!(
            raise(new_id(), draft, &c, Some(&withdrawn)),
            Err(ChangeRequestError::SupersedesUnanswered(withdrawn.id()))
        );
    }

    #[test]
    fn a_crux_is_raised_with_an_option_and_no_choice_twice() {
        let c = cr();
        assert_eq!(
            raise(new_id(), raising(&c, Vec::new(), None), &c, None),
            Err(ChangeRequestError::NoOption)
        );
        let twice = raising(
            &c,
            vec![option_asking(1, "up", &[]), option_asking(2, "up", &[])],
            None,
        );
        assert_eq!(
            raise(new_id(), twice, &c, None),
            Err(ChangeRequestError::DuplicateOption(
                Choice::try_from("up").unwrap()
            ))
        );
    }

    #[test]
    fn an_option_that_asks_the_other_kind_of_a_target_is_refused_with_the_option_and_the_target() {
        let p = project();
        // The request asks the Clause 31 to be kept.
        let c = ChangeRequest::open(
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
        let draft = raising(
            &c,
            vec![
                option_asking(1, "fine", &[(clause(31), ExpectKind::Keep)]),
                option_asking(2, "clashes", &[(clause(31), ExpectKind::Target)]),
            ],
            None,
        );
        assert_eq!(
            raise(new_id(), draft, &c, None),
            Err(ChangeRequestError::ExpectationConflict {
                option: Choice::try_from("clashes").unwrap(),
                target: clause(31)
            })
        );
    }

    #[test]
    fn what_the_decision_replaced_asked_is_not_a_clash_for_the_crux_that_supersedes_it() {
        let p = project();
        let mut c = cr();
        let mut a = crux_asking(&c, 501, &[(clause(30), ExpectKind::Target)], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        let keeps = || {
            vec![option_asking(
                1,
                "keep it",
                &[(clause(30), ExpectKind::Keep)],
            )]
        };
        // Superseding `a`: what `a` asked no longer stands, so keeping the Clause is no clash.
        let q = raise(new_id(), raising(&c, keeps(), Some(&a)), &c, Some(&a)).unwrap();
        assert_eq!(q.supersedes(), Some(a.id()));
        // Raised without superseding it, the same option is the clash it is.
        assert_eq!(
            raise(new_id(), raising(&c, keeps(), None), &c, None),
            Err(ChangeRequestError::ExpectationConflict {
                option: Choice::try_from("keep it").unwrap(),
                target: clause(30)
            })
        );
        // What the request itself asks is never left out.
        let mut own = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            ChangeRequestDraft {
                title: Title::try_from("T").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: crate::domain::service::change_request::expectations_now(
                    &p,
                    [
                        (clause(30), ExpectKind::Target),
                        (contract(300), ExpectKind::Target),
                    ],
                ),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let mut a2 = crux_asking(&own, 502, &[(clause(31), ExpectKind::Target)], None);
        decide(&mut a2, &mut own, chosen_with(902), &p, None).unwrap();
        let draft = raising(&own, keeps(), Some(&a2));
        assert_eq!(
            raise(new_id(), draft, &own, Some(&a2)),
            Err(ChangeRequestError::ExpectationConflict {
                option: Choice::try_from("keep it").unwrap(),
                target: clause(30)
            })
        );
    }

    #[test]
    fn the_decision_taken_back_is_that_of_the_crux_replaced_when_it_was_raised_here_and_answered() {
        let p = project();
        let mut c = cr();
        assert_eq!(taken_back(&c, None), None);
        let mut a = crux_asking(&c, 501, &[], None);
        // Open: there is no decision to take back.
        assert_eq!(taken_back(&c, Some(&a)), None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        assert_eq!(
            taken_back(&c, Some(&a)),
            Some(AnswerId::from_uuid(test_uuid(901)))
        );
        // A Crux of another change request asked nothing here.
        let elsewhere = ChangeRequest::open(
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
        assert_eq!(taken_back(&elsewhere, Some(&a)), None);
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

    // Replacing a decision once.

    #[test]
    fn a_decision_is_replaced_by_one_crux_unless_that_one_was_withdrawn() {
        let p = project();
        let mut c = cr();
        let mut a = crux_asking(&c, 501, &[], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        let mut b = crux_asking(&c, 502, &[], Some(&a));
        // `b` is open, and replaces `a` already.
        let cruxes = [a.clone(), b.clone()];
        assert_eq!(superseded_by(a.id(), &cruxes).map(Crux::id), Some(b.id()));
        assert_eq!(superseded_by(b.id(), &cruxes), None);
        let refused = ChangeRequestError::SupersededAlready {
            crux: a.id(),
            by: b.id(),
        };
        assert_eq!(
            super::check_supersedes(a.id(), Some(&a), &cruxes),
            Err(refused.clone())
        );
        let draft = raising(&c, vec![option_asking(1, "up", &[])], Some(&a));
        assert_eq!(
            super::raise(new_id(), draft, &c, Some(&a), &cruxes),
            Err(refused)
        );
        // Decided otherwise, it still replaces it.
        decide(&mut b, &mut c, chosen_with(902), &p, Some(&a)).unwrap();
        let cruxes = [a.clone(), b.clone()];
        assert_eq!(superseded_by(a.id(), &cruxes).map(Crux::id), Some(b.id()));
        // A Crux that was withdrawn replaced nothing: the decision stands, and may be superseded anew.
        let mut c = cr();
        let mut a = crux_asking(&c, 501, &[], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        let mut w = crux_asking(&c, 502, &[], Some(&a));
        decide(
            &mut w,
            &mut c,
            answer_on(Decision::Withdrawn, arbitrary()),
            &p,
            Some(&a),
        )
        .unwrap();
        let cruxes = [a.clone(), w];
        assert_eq!(superseded_by(a.id(), &cruxes), None);
        assert_eq!(super::check_supersedes(a.id(), Some(&a), &cruxes), Ok(()));
        let draft = raising(&c, vec![option_asking(1, "up", &[])], Some(&a));
        assert!(super::raise(new_id(), draft, &c, Some(&a), &cruxes).is_ok());
    }

    #[test]
    fn the_decision_that_stands_is_the_end_of_the_chain_and_may_be_a_crux_not_decided_yet() {
        let p = project();
        let mut c = cr();
        let mut a = crux_asking(&c, 501, &[], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        let mut b = crux_asking(&c, 502, &[], Some(&a));
        decide(&mut b, &mut c, chosen_with(902), &p, Some(&a)).unwrap();
        let open = crux_asking(&c, 503, &[], Some(&b));
        let cruxes = [a.clone(), b.clone(), open.clone()];
        for from in [&a, &b, &open] {
            assert_eq!(standing(from, &cruxes).id(), open.id());
        }
        assert!(standing(&open, &cruxes).is_open());
        // Without the last link, the chain ends at `b`; a Crux nothing replaces stands for itself.
        let cruxes = [a.clone(), b.clone()];
        assert_eq!(standing(&a, &cruxes).id(), b.id());
        assert_eq!(standing(&b, &[]).id(), b.id());
    }

    #[test]
    fn a_store_that_holds_two_cruxes_replacing_one_decision_gives_the_one_raised_first() {
        let p = project();
        let mut c = cr();
        let mut a = crux_asking(&c, 501, &[], None);
        decide(&mut a, &mut c, chosen_with(901), &p, None).unwrap();
        let late = crux_asking(&c, 509, &[], Some(&a));
        let early = crux_asking(&c, 503, &[], Some(&a));
        for cruxes in [
            [a.clone(), late.clone(), early.clone()],
            [early.clone(), a.clone(), late.clone()],
        ] {
            assert_eq!(
                superseded_by(a.id(), &cruxes).map(Crux::id),
                Some(early.id())
            );
            assert_eq!(standing(&a, &cruxes).id(), early.id());
        }
    }

    // What an answer adds outside its change request.

    /// The Project of the tests above, with a second Contract (301) of one Clause (32) that a request framing the
    /// first Contract does not frame.
    fn project_with_a_second_contract() -> Project {
        let mut p = project();
        p.put_contract(
            ContextId::from_uuid(test_uuid(1)),
            ContractId::from_uuid(test_uuid(301)),
            Title::try_from("U").unwrap(),
            vec![(
                ClauseId::from_uuid(test_uuid(32)),
                ClauseContent::new(
                    ClauseLabel::try_from("C3").unwrap(),
                    ClauseText::try_from("a sum is whole").unwrap(),
                ),
            )],
        )
        .unwrap();
        p
    }

    fn contract(n: u128) -> Target {
        Target::Contract(ContractId::from_uuid(test_uuid(n)))
    }

    /// The change request `n` that asks `framed` to be revised, as `p` has them now.
    fn request_framing(p: &Project, n: u128, title: &str, framed: &[Target]) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(n)),
            ChangeRequestDraft {
                title: Title::try_from(title).unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: crate::domain::service::change_request::expectations_now(
                    p,
                    framed.iter().map(|t| (*t, ExpectKind::Target)),
                ),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        )
    }

    fn taken_by(requests: &[&ChangeRequest]) -> Beyond {
        Beyond::taken_by(requests.iter().map(|c| c.id()).collect()).unwrap()
    }

    #[test]
    fn an_option_inside_the_frame_is_recorded_and_says_nothing_beyond() {
        let p = project_with_a_second_contract();
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        let mut q = crux_asking(&c, 501, &[(clause(31), ExpectKind::Target)], None);
        decide(&mut q, &mut c, chosen_with(901), &p, None).unwrap();
        assert_eq!(c.decided_expects().len(), 1);
        assert_eq!(c.decided_expects()[0].target, clause(31));
        assert_eq!(q.answer_given().and_then(Answer::beyond), None);
    }

    #[test]
    fn an_option_that_names_a_target_outside_the_frame_needs_the_answer_to_say_where_it_goes() {
        let p = project_with_a_second_contract();
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        let before = c.clone();
        let mut q = crux_asking(
            &c,
            501,
            &[
                (clause(31), ExpectKind::Target),
                (clause(32), ExpectKind::Target),
            ],
            None,
        );
        assert_eq!(
            decide(&mut q, &mut c, chosen_with(901), &p, None),
            Err(ChangeRequestError::AnswerOutside {
                crux: q.id(),
                option: Choice::try_from("this").unwrap(),
                targets: vec![clause(32)]
            })
        );
        assert!(q.is_open());
        assert_eq!(c, before);
        // A keep protects, it does not widen: one outside the frame is recorded, and is not outside.
        let mut keeps = crux_asking(&c, 502, &[(clause(32), ExpectKind::Keep)], None);
        decide(&mut keeps, &mut c, chosen_with(902), &p, None).unwrap();
        assert_eq!(c.decided_expects().len(), 1);
        assert_eq!(c.decided_expects()[0].kind, ExpectKind::Keep);
    }

    #[test]
    fn what_lies_outside_goes_to_the_requests_that_frame_it_and_is_not_recorded_in_this_one() {
        let p = project_with_a_second_contract();
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        let other = request_framing(&p, 2, "Other", &[clause(32)]);
        let mut q = crux_asking(
            &c,
            501,
            &[
                (clause(31), ExpectKind::Target),
                (clause(32), ExpectKind::Target),
            ],
            None,
        );
        let beyond = taken_by(&[&other]);
        let all = [c.clone(), other.clone()];
        super::decide(
            &mut q,
            &mut c,
            chosen_with(901).with_beyond(beyond.clone()),
            &p,
            None,
            &all,
        )
        .unwrap();
        assert!(!q.is_open());
        assert_eq!(q.answer_given().and_then(Answer::beyond), Some(&beyond));
        // The target inside is recorded, the one outside is not, and the frame did not grow by it.
        let recorded: Vec<Target> = c.decided_expects().iter().map(|d| d.target).collect();
        assert_eq!(recorded, [clause(31)]);
        assert!(!crate::domain::service::change_request::frame(&c).contains(&clause(32)));
    }

    #[test]
    fn what_lies_outside_may_be_declined_and_is_not_recorded_either() {
        let p = project_with_a_second_contract();
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        let mut q = crux_asking(&c, 501, &[(clause(32), ExpectKind::Target)], None);
        let declined = Beyond::declined(prose("not now"));
        super::decide(
            &mut q,
            &mut c,
            chosen_with(901).with_beyond(declined.clone()),
            &p,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(q.answer_given().and_then(Answer::beyond), Some(&declined));
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn the_requests_an_answer_names_are_there_not_this_one_not_resolved_and_frame_what_is_outside()
    {
        let p = project_with_a_second_contract();
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        let mut q = crux_asking(
            &c,
            501,
            &[
                (clause(31), ExpectKind::Target),
                (clause(32), ExpectKind::Target),
            ],
            None,
        );
        let before = (c.clone(), q.clone());
        let elsewhere = request_framing(&p, 2, "Elsewhere", &[clause(30)]);
        let mut done = request_framing(&p, 3, "Done", &[clause(32)]);
        done.resolve(Resolution::new(
            ResolutionKind::Done,
            prose("done"),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(7),
        ))
        .unwrap();
        let framing = request_framing(&p, 4, "Framing", &[contract(301)]);
        let ghost = ChangeRequestId::from_uuid(test_uuid(99));
        let all = [c.clone(), elsewhere.clone(), done.clone(), framing.clone()];
        for (named, refused) in [
            (
                vec![elsewhere.id()],
                ChangeRequestError::BeyondUnframed { target: clause(32) },
            ),
            (vec![c.id()], ChangeRequestError::BeyondItself),
            (
                vec![done.id()],
                ChangeRequestError::BeyondResolved(done.id()),
            ),
            // One that frames it does not make up for one that is refused.
            (
                vec![framing.id(), done.id()],
                ChangeRequestError::BeyondResolved(done.id()),
            ),
            (vec![ghost], ChangeRequestError::ReferenceUnknown(ghost)),
        ] {
            let answer = chosen_with(901).with_beyond(Beyond::taken_by(named).unwrap());
            assert_eq!(
                super::decide(&mut q, &mut c, answer, &p, None, &all),
                Err(refused)
            );
            assert_eq!((c.clone(), q.clone()), before, "nothing was changed");
        }
        // Two requests that frame a target between them, each one, are enough.
        let one = request_framing(&p, 5, "One", &[clause(32)]);
        let all = [c.clone(), elsewhere.clone(), one.clone()];
        let answer = chosen_with(901).with_beyond(taken_by(&[&elsewhere, &one]));
        super::decide(&mut q, &mut c, answer, &p, None, &all).unwrap();
        // They are read when nothing is outside too: a worded answer may reach outside in its words.
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        let mut q = crux_asking(&c, 502, &[(clause(31), ExpectKind::Target)], None);
        let answer = chosen_with(901).with_beyond(taken_by(&[&c]));
        let all = [c.clone()];
        assert_eq!(
            super::decide(&mut q, &mut c, answer, &p, None, &all),
            Err(ChangeRequestError::BeyondItself)
        );
    }

    #[test]
    fn a_withdrawal_says_nothing_beyond_whatever_the_state_of_its_request() {
        let p = project_with_a_second_contract();
        let declined = Beyond::declined(prose("nothing"));
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        let mut q = crux_asking(&c, 501, &[], None);
        let withdrawal =
            || answer_on(Decision::Withdrawn, arbitrary()).with_beyond(declined.clone());
        assert_eq!(
            decide(&mut q, &mut c, withdrawal(), &p, None),
            Err(ChangeRequestError::BeyondOnWithdrawal)
        );
        assert!(q.is_open());
        c.resolve(Resolution::new(
            ResolutionKind::WontDo,
            prose("not needed"),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(7),
        ))
        .unwrap();
        assert_eq!(
            decide(&mut q, &mut c, withdrawal(), &p, None),
            Err(ChangeRequestError::BeyondOnWithdrawal)
        );
        assert!(q.is_open());
    }

    #[test]
    fn what_the_decision_replaced_asked_is_taken_back_before_the_frame_is_read() {
        let p = project_with_a_second_contract();
        let mut c = request_framing(&p, 1, "T", &[contract(300)]);
        // A request widened by an earlier decision, as a store written before a decision stopped doing that has it.
        let mut a = crux_asking(&c, 501, &[(clause(32), ExpectKind::Target)], None);
        a.answer(chosen_with(901)).unwrap();
        let now = revisions_now(&p, [clause(32)]);
        c.record_decided_expects(
            a.chosen_option().unwrap(),
            &now,
            AnswerId::from_uuid(test_uuid(901)),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(crate::domain::service::change_request::frame(&c).contains(&clause(32)));
        // The same ask again, as a decision that replaces it: the take-back leaves it outside the frame.
        let mut b = crux_asking(&c, 502, &[(clause(32), ExpectKind::Target)], Some(&a));
        assert_eq!(
            decide(&mut b, &mut c, chosen_with(902), &p, Some(&a)),
            Err(ChangeRequestError::AnswerOutside {
                crux: b.id(),
                option: Choice::try_from("this").unwrap(),
                targets: vec![clause(32)]
            })
        );
    }
}
