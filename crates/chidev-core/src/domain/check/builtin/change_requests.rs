//! Checks on the change requests.

use std::borrow::Cow;

use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::finding::{Ground, LineState};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::change_request::ChangeRequestState;
use crate::domain::service::change_request::{
    awaiting_decision, keep_moved, out_of_frame, resolved_without_work, targets_untouched,
};
use crate::domain::service::cite::revision_of;
use crate::domain::vo::answer::Basis;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::intent::Intent;
use crate::domain::vo::target::{ExpectKind, Target};

/// A change request resolved as done with no work on it or on its parts.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedWithoutWork;

static RESOLVED_WITHOUT_WORK: Descriptor = Descriptor {
    id: RuleId::from_static("cr.resolved-without-work"),
    summary: Cow::Borrowed("a change request resolved as done that no change set was applied for"),
    help: Cow::Borrowed(
        "The change request was resolved as done, but no change set applied `for` it, or for a part of it, moved \
           anything. The resolution stands (it is a decision, not a reading of the work); the record and the work \
           disagree. If nothing needed to change, resolve it as `invalid` or `wont_do` with the reason; if the \
           work was done under another request, say so in its resolution or reopen it.",
    ),
    hooks: Cow::Borrowed(&[Hook::Resolve, Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for ResolvedWithoutWork {
    fn descriptor(&self) -> &'static Descriptor {
        &RESOLVED_WITHOUT_WORK
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        resolved_without_work(view.change_requests)
            .into_iter()
            .map(|cr| {
                Finding::new(
                    &RESOLVED_WITHOUT_WORK,
                    Some(Subject::ChangeRequest(cr)),
                    "resolved as done, but no work was applied for it".to_string(),
                )
            })
            .collect()
    }
}

/// A change request resolved as done that named a target its work never touched.
#[derive(Debug, Clone, Copy)]
pub struct TargetsUntouched;

static TARGETS_UNTOUCHED: Descriptor = Descriptor {
    id: RuleId::from_static("cr.targets-untouched"),
    summary: Cow::Borrowed(
        "a change request resolved as done, one of whose targets no work of it or of its parts touched",
    ),
    help: Cow::Borrowed(
        "The change request asked for a target to be revised or retired and was resolved as done, but no change \
           set applied for it, or for a part of it, moved that target or anything in it. It may be right (the \
           target needed nothing), but the request and the work say different things: check the targets, or the \
           resolution. A request with no work at all is `cr.resolved-without-work`, not this.",
    ),
    hooks: Cow::Borrowed(&[Hook::Resolve, Hook::Check]),
    default_level: Level::Note,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for TargetsUntouched {
    fn descriptor(&self) -> &'static Descriptor {
        &TARGETS_UNTOUCHED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        targets_untouched(view.change_requests, view.project)
            .into_iter()
            .map(|u| {
                Finding::new(
                    &TARGETS_UNTOUCHED,
                    Some(Subject::ChangeRequest(u.cr)),
                    format!(
                        "resolved as done, but {} target(s) were never touched by its work",
                        u.untouched.len()
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
    hooks: Cow::Borrowed(&[Hook::Resolve, Hook::Check]),
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
        let policy = view.project.policy();
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
                // The ground is the line's own `changed_at`, not the revision of the whole Policy: rewording
                // another line moves that and has nothing to do with this decision.
                let (message, state) = match policy.line(*line) {
                    None => (
                        format!("the policy line {line} it was decided on is gone"),
                        LineState::Gone,
                    ),
                    Some(l) if l.changed_at() > *policy_revision => (
                        format!(
                            "the policy line {line} was reworded after it was applied (policy {policy_revision}, now {})",
                            policy.revision()
                        ),
                        LineState::Reworded(l.changed_at()),
                    ),
                    Some(_) => return None,
                };
                Some(
                    Finding::new(
                        &POLICY_LINE_GONE,
                        Some(Subject::ChangeRequest(c.id())),
                        message,
                    )
                    .grounded(vec![Ground::PolicyLine { line: *line, state }]),
                )
            })
            .collect()
    }
}

/// A change request that keeps a target as it is, one of which has moved.
#[derive(Debug, Clone, Copy)]
pub struct KeepMoved;

static KEEP_MOVED: Descriptor = Descriptor {
    id: RuleId::from_static("cr.keep-moved"),
    summary: Cow::Borrowed(
        "an unresolved change request that keeps a target as it is, and the target moved",
    ),
    help: Cow::Borrowed(
        "The change request says a target stays as it is; the target changed (or went) anyway. Resolving it as \
           done would record a contradiction. Either the change was meant (name the target in a request that asks \
           for it) or the target should be put back.",
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
        keep_moved(view.change_requests, view.project)
            .into_iter()
            .map(|k| {
                // Each target that moved, with the revision it has now (none when it is gone): it moving on
                // again is a new ground.
                let grounds = k
                    .moved
                    .iter()
                    .map(|t| Ground::Moved {
                        target: *t,
                        now: revision_of(view.project, *t),
                    })
                    .collect();
                Finding::new(
                    &KEEP_MOVED,
                    Some(Subject::ChangeRequest(k.cr)),
                    format!("{} target(s) it keeps have moved", k.moved.len()),
                )
                .grounded(grounds)
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
        "an unresolved change request whose change sets moved a target it did not name",
    ),
    help: Cow::Borrowed(
        "The targets a change request names are its frame, and naming a container frames everything inside it. \
           A change set applied for the request moved a target outside that frame. The frame is the request's own: \
           a parent's targets neither widen nor bound its parts. Either the change was meant (name the target in \
           `targets`, or apply the change set for the request it belongs to) or it should be taken back.",
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
                // The ground is the footprint, not the Project as it is now: a put that moves the same target
                // for no request leaves the work of the request, and so its grounds, as they were.
                let grounds = o
                    .by_work
                    .iter()
                    .map(|(target, at)| Ground::Work {
                        target: *target,
                        at: *at,
                    })
                    .collect();
                Finding::new(
                    &OUT_OF_FRAME,
                    Some(Subject::ChangeRequest(o.cr)),
                    format!("{} target(s) moved outside its frame", o.moved.len()),
                )
                .grounded(grounds)
            })
            .collect()
    }
}

/// A change request whose intent and targets tell different things.
#[derive(Debug, Clone, Copy)]
pub struct IntentMismatch;

static INTENT_MISMATCH: Descriptor = Descriptor {
    id: RuleId::from_static("cr.intent-mismatch"),
    summary: Cow::Borrowed("an unresolved change request whose intent and whose targets disagree"),
    help: Cow::Borrowed(
        "A keep-behavior request names a Clause as a target to revise or retire, or a change-behavior request names \
           no target at all (in itself or in any option of its question) and is not a parent of other requests. \
           It may be right (rewording a Clause keeps the behaviour), but one of the two is likely wrong: check the \
           intent and the targets.",
    ),
    hooks: Cow::Borrowed(&[Hook::Resolve, Hook::Check]),
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
                let names = |kind: ExpectKind| kind == ExpectKind::Target;
                // A parent only aggregates: it need not name a target.
                let is_parent = view
                    .change_requests
                    .iter()
                    .any(|p| p.parent() == Some(c.id()));
                let message = match c.intent() {
                    Intent::KeepBehavior
                        if c.possible_kinds()
                            .any(|(t, k)| matches!(t, Target::Clause(_)) && names(k)) =>
                    {
                        "it keeps the behaviour, yet names a clause to revise or retire"
                    }
                    Intent::ChangeBehavior
                        if !is_parent && !c.possible_kinds().any(|(_, k)| names(k)) =>
                    {
                        "it changes the behaviour, yet names no target"
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
