//! Checks on the change requests.

use std::borrow::Cow;

use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::change_request::ChangeRequestState;
use crate::domain::service::change_request::{awaiting_decision, keep_moved, unapplied};
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
           it expects to remove is still there. Make the change, then resolve the change request.",
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
        unapplied(view.change_requests.iter(), view.after)
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
    summary: Cow::Borrowed("an unresolved change request whose question no person has decided"),
    help: Cow::Borrowed(
        "The change request asks a question and waits for the person. Work goes on with the default, if there is \
           one, but the option chosen may expect more of other targets. Decide it (`cr_decide`).",
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
        keep_moved(view.change_requests.iter(), view.after)
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
