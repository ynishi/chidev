//! Checks on the change requests.

use std::borrow::Cow;

use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::change_request::ChangeRequestState;
use crate::domain::service::change_request::unapplied;
use crate::domain::service::cite::revision_of;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::intent::Intent;

/// A change request the definitions have not followed.
#[derive(Debug, Clone, Copy)]
pub struct Unapplied;

static UNAPPLIED: Descriptor = Descriptor {
    id: RuleId::from_static("cr.unapplied"),
    summary: Cow::Borrowed("an unresolved change request whose definitions have not changed"),
    help: Cow::Borrowed(
        "The change was asked for and decided, but a definition it names is still as it was, or is gone. Make the \
           change, then resolve the change request.",
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
                        "{} definition(s) not changed yet, {} gone",
                        u.unchanged.len(),
                        u.gone.len()
                    ),
                )
            })
            .collect()
    }
}

/// A change request that promised to keep the behaviour, one of whose definitions changed.
#[derive(Debug, Clone, Copy)]
pub struct KeepBehaviorChanged;

static KEEP_BEHAVIOR_CHANGED: Descriptor = Descriptor {
    id: RuleId::from_static("cr.keep-behavior-changed"),
    summary: Cow::Borrowed(
        "a keep-behavior change request one of whose definitions moved past the revision it recorded",
    ),
    help: Cow::Borrowed(
        "The change request says the behaviour stays; a definition it is about changed anyway. Either the change \
           was meant (open a change-behavior request) or the definition should be put back.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Resolve, Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for KeepBehaviorChanged {
    fn descriptor(&self) -> &'static Descriptor {
        &KEEP_BEHAVIOR_CHANGED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        // A resolved one is a record; what changes after it is another change request's business.
        view.change_requests
            .iter()
            .filter(|c| {
                c.intent() == Intent::KeepBehavior && c.state() != ChangeRequestState::Resolved
            })
            .filter_map(|c| {
                let moved = c
                    .lines()
                    .iter()
                    .filter(|(t, recorded)| {
                        revision_of(view.after, **t).is_some_and(|now| now > **recorded)
                    })
                    .count();
                (moved > 0).then(|| {
                    Finding::new(
                        &KEEP_BEHAVIOR_CHANGED,
                        Some(Subject::ChangeRequest(c.id())),
                        format!("{moved} definition(s) it promised to keep changed"),
                    )
                })
            })
            .collect()
    }
}

/// A change request resolved while a part of it is not.
#[derive(Debug, Clone, Copy)]
pub struct ParentResolvedEarly;

static PARENT_RESOLVED_EARLY: Descriptor = Descriptor {
    id: RuleId::from_static("cr.parent-resolved-early"),
    summary: Cow::Borrowed("a resolved change request with a part that is not resolved"),
    help: Cow::Borrowed(
        "The broader request was closed before its parts. Resolve the parts first, so that the record shows the \
           whole change was applied.",
    ),
    hooks: Cow::Borrowed(&[Hook::Resolve, Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for ParentResolvedEarly {
    fn descriptor(&self) -> &'static Descriptor {
        &PARENT_RESOLVED_EARLY
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        view.change_requests
            .iter()
            .filter(|p| p.state() == ChangeRequestState::Resolved)
            .filter_map(|p| {
                let open = view
                    .change_requests
                    .iter()
                    .filter(|c| {
                        c.parent() == Some(p.id()) && c.state() != ChangeRequestState::Resolved
                    })
                    .count();
                (open > 0).then(|| {
                    Finding::new(
                        &PARENT_RESOLVED_EARLY,
                        Some(Subject::ChangeRequest(p.id())),
                        format!("resolved while {open} part(s) are not"),
                    )
                })
            })
            .collect()
    }
}
