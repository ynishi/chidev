//! Checks on the change requests.

use std::borrow::Cow;

use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::change_request::ChangeRequestState;
use crate::domain::service::change_request::{
    awaiting_decision, keep_moved, out_of_frame, unapplied,
};
use crate::domain::vo::answer::Basis;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::intent::Intent;
use crate::domain::vo::target::{ExpectKind, Target};

/// A change request the targets have not followed.
#[derive(Debug, Clone, Copy)]
pub struct Unapplied;

static UNAPPLIED: Descriptor = Descriptor {
    id: RuleId::from_static("cr.unapplied"),
    summary: Cow::Borrowed("an unresolved change request whose targets have not followed"),
    help: Cow::Borrowed(
        "The change was asked for, but a target it expects to change is still as it was or is gone, or a target \
           it expects to remove is still there. A change counts for the request only when it was made by a change \
           set applied for it (`for`), or for a part of it: a target that moved in a change set applied for \
           another request, or for none, is still owed. Make the change as a change set for the request, then \
           resolve it.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Resolve, Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for Unapplied {
    fn descriptor(&self) -> &'static Descriptor {
        &UNAPPLIED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        unapplied(view.change_requests, view.after)
            .into_iter()
            .map(|u| {
                Finding::new(
                    &UNAPPLIED,
                    Some(Subject::ChangeRequest(u.cr)),
                    format!(
                        "{} target(s) not changed yet, {} gone, {} not removed yet",
                        u.unchanged.len(),
                        u.gone.len(),
                        u.not_removed.len()
                    ),
                )
            })
            .collect()
    }
}

/// A change request whose question has no decision yet.
#[derive(Debug, Clone, Copy)]
pub struct AwaitingDecision;

static AWAITING_DECISION: Descriptor = Descriptor {
    id: RuleId::from_static("cr.awaiting-decision"),
    summary: Cow::Borrowed(
        "an unresolved change request whose crux no decision maker has answered",
    ),
    help: Cow::Borrowed(
        "The change request has a choice that neither the repository nor the decision Policy settles, and waits \
           for a decision maker. What depends on the answer waits; the option chosen may expect more of other \
           targets. Apply the Policy if a line covers it, or ask a person, then decide it (`cr_decide`).",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Resolve, Hook::Check]),
    default_level: Level::Note,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for AwaitingDecision {
    fn descriptor(&self) -> &'static Descriptor {
        &AWAITING_DECISION
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        awaiting_decision(view.change_requests.iter())
            .into_iter()
            .map(|c| {
                let kind = c.crux().map_or("question", |q| q.kind().word());
                Finding::new(
                    &AWAITING_DECISION,
                    Some(Subject::ChangeRequest(c.id())),
                    format!("a {kind} question is not decided yet"),
                )
            })
            .collect()
    }
}

/// An answer that rests on a line of the Policy which has since gone or been reworded.
#[derive(Debug, Clone, Copy)]
pub struct PolicyLineGone;

static POLICY_LINE_GONE: Descriptor = Descriptor {
    id: RuleId::from_static("crux.policy-line-gone"),
    summary: Cow::Borrowed(
        "a decision rests on a policy line that is gone or whose text changed since",
    ),
    help: Cow::Borrowed(
        "The decision was made by applying a line of the decision Policy. The Policy no longer has that line, or \
           the line now says something else than when it was applied. The answer stands (it is never rewritten), \
           but what it was derived from is not what it was. If the decision should differ, open a change request \
           whose Crux supersedes it.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Check]),
    default_level: Level::Note,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for PolicyLineGone {
    fn descriptor(&self) -> &'static Descriptor {
        &POLICY_LINE_GONE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let policy = view.after.policy();
        view.change_requests
            .iter()
            .filter_map(|c| {
                let answer = c.crux().and_then(|q| q.answer())?;
                let Basis::Policy {
                    line,
                    policy_revision,
                } = answer.basis()
                else {
                    return None;
                };
                let message = match policy.line(*line) {
                    None => format!("the policy line {line} it was decided on is gone"),
                    Some(l) if l.changed_at() > *policy_revision => format!(
                        "the policy line {line} was reworded after it was applied (policy {policy_revision}, now {})",
                        policy.revision()
                    ),
                    Some(_) => return None,
                };
                Some(Finding::new(
                    &POLICY_LINE_GONE,
                    Some(Subject::ChangeRequest(c.id())),
                    message,
                ))
            })
            .collect()
    }
}

/// A change request that expects a target to stay as it is, one of which has moved.
#[derive(Debug, Clone, Copy)]
pub struct KeepMoved;

static KEEP_MOVED: Descriptor = Descriptor {
    id: RuleId::from_static("cr.keep-moved"),
    summary: Cow::Borrowed(
        "an unresolved change request that expects a target to stay as it is, and the target moved",
    ),
    help: Cow::Borrowed(
        "The change request says a target stays as it is; the target changed (or went) anyway. It cannot be \
           resolved while that holds. Either the change was meant (open a request that expects it) or the target \
           should be put back.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Resolve, Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for KeepMoved {
    fn descriptor(&self) -> &'static Descriptor {
        &KEEP_MOVED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        keep_moved(view.change_requests, view.after)
            .into_iter()
            .map(|k| {
                Finding::new(
                    &KEEP_MOVED,
                    Some(Subject::ChangeRequest(k.cr)),
                    format!("{} target(s) it expects to keep have moved", k.moved.len()),
                )
            })
            .collect()
    }
}

/// A change request whose work moved targets outside its frame.
#[derive(Debug, Clone, Copy)]
pub struct OutOfFrame;

static OUT_OF_FRAME: Descriptor = Descriptor {
    id: RuleId::from_static("cr.out-of-frame"),
    summary: Cow::Borrowed(
        "an unresolved change request whose change sets moved a target outside the frame it declared",
    ),
    help: Cow::Borrowed(
        "The targets a change request expects to change or remove are its frame, and naming a container frames \
           everything inside it. A change set applied for the request (or for a part of it) moved a target outside \
           the frame of the request, or outside the frame of a request it is a part of. Either the change was \
           meant (declare the target in `change` or `remove`, or apply the change set for the request it belongs \
           to) or it should be taken back.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for OutOfFrame {
    fn descriptor(&self) -> &'static Descriptor {
        &OUT_OF_FRAME
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        out_of_frame(view.change_requests)
            .into_iter()
            .map(|o| {
                Finding::new(
                    &OUT_OF_FRAME,
                    Some(Subject::ChangeRequest(o.cr)),
                    format!("{} target(s) moved outside its frame", o.moved.len()),
                )
            })
            .collect()
    }
}

/// A change request whose intent and expectations tell different things.
#[derive(Debug, Clone, Copy)]
pub struct IntentMismatch;

static INTENT_MISMATCH: Descriptor = Descriptor {
    id: RuleId::from_static("cr.intent-mismatch"),
    summary: Cow::Borrowed(
        "an unresolved change request whose intent and whose expectations disagree",
    ),
    help: Cow::Borrowed(
        "A keep-behavior request expects a Clause to change or go, or a change-behavior request expects nothing \
           to change or go (in itself or in any option of its question). It may be right (rewording a Clause \
           keeps the behaviour), but one of the two is likely wrong: check the intent and the expectations.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Check]),
    default_level: Level::Note,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for IntentMismatch {
    fn descriptor(&self) -> &'static Descriptor {
        &INTENT_MISMATCH
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        // A resolved one is a record.
        view.change_requests
            .iter()
            .filter(|c| c.state() != ChangeRequestState::Resolved)
            .filter_map(|c| {
                let moves = |kind: ExpectKind| kind != ExpectKind::Keep;
                let message = match c.intent() {
                    Intent::KeepBehavior
                        if c.possible_kinds()
                            .any(|(t, k)| matches!(t, Target::Clause(_)) && moves(k)) =>
                    {
                        "it keeps the behaviour, yet expects a clause to change or go"
                    }
                    Intent::ChangeBehavior if !c.possible_kinds().any(|(_, k)| moves(k)) => {
                        "it changes the behaviour, yet expects nothing to change or go"
                    }
                    _ => return None,
                };
                Some(Finding::new(
                    &INTENT_MISMATCH,
                    Some(Subject::ChangeRequest(c.id())),
                    message.to_string(),
                ))
            })
            .collect()
    }
}
