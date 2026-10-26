//! Checks on the Pins.

use std::borrow::Cow;

use super::CONFIRM_MOMENTS;
use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Level, Moment};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding, Ground};
use crate::domain::service::cite::revision_of;
use crate::domain::service::pin::{StaleReason, Staleness, staleness};
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::id::InterfaceId;
use crate::domain::vo::target::Target;

/// A Pin whose basis moved.
#[derive(Debug, Clone, Copy)]
pub struct StalePin;

static STALE_PIN: Descriptor = Descriptor {
    id: RuleId::from_static("pin.stale"),
    summary: Cow::Borrowed("a pin whose code no longer stands on what it was pinned against"),
    help: Cow::Borrowed(
        "The interface, a harness that checks it or a contract it cites changed after the code was pinned. Run \
           the harness against the code and pin it again.",
    ),
    moments: Cow::Borrowed(&[Moment::Edit, Moment::Pin, Moment::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for StalePin {
    fn descriptor(&self) -> &'static Descriptor {
        &STALE_PIN
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        view.pins
            .iter()
            .filter_map(|pin| {
                let (message, grounds) = match staleness(pin, view.project) {
                    Staleness::Fresh => return None,
                    // A retired Interface is gone on purpose and nothing can fix the Pin: no finding.
                    Staleness::InterfaceGone
                        if view
                            .project
                            .retired_of(Target::Interface(pin.interface()))
                            .is_some() =>
                    {
                        return None;
                    }
                    Staleness::InterfaceGone => {
                        ("the pinned interface is gone".to_string(), Vec::new())
                    }
                    // What moved is a ground, with the revision it has now (none when it is gone); the use case
                    // names it after the message.
                    Staleness::Stale(why) => (
                        "the code was pinned against what has moved".to_string(),
                        why.into_iter()
                            .map(|reason| Ground::Pin {
                                now: revision_of(view.project, target_of(pin.interface(), &reason)),
                                reason,
                            })
                            .collect(),
                    ),
                };
                Some(
                    Finding::new(
                        &STALE_PIN,
                        Some(Subject::Interface(pin.interface())),
                        message,
                    )
                    .grounded(grounds),
                )
            })
            .collect()
    }
}

/// The target a Pin stands on that `reason` says moved: the pinned Interface for `pinned`.
fn target_of(pinned: InterfaceId, reason: &StaleReason) -> Target {
    match reason {
        StaleReason::Interface => Target::Interface(pinned),
        StaleReason::Harness(id) => Target::Harness(*id),
        StaleReason::Contract(id) => Target::Contract(*id),
        StaleReason::UsedInterface(id) => Target::Interface(*id),
    }
}

/// An Interface whose code was never pinned.
#[derive(Debug, Clone, Copy)]
pub struct Unpinned;

static UNPINNED: Descriptor = Descriptor {
    id: RuleId::from_static("pin.unpinned"),
    summary: Cow::Borrowed("an interface whose code was never pinned"),
    help: Cow::Borrowed(
        "Nothing records which code the harness checked. Pin the code once a case checks the interface.",
    ),
    moments: Cow::Borrowed(CONFIRM_MOMENTS),
    default_level: Level::Note,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for Unpinned {
    fn descriptor(&self) -> &'static Descriptor {
        &UNPINNED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        view.project
            .contexts()
            .flat_map(|c| c.interfaces())
            .filter(|i| !view.pins.iter().any(|p| p.interface() == i.id()))
            .map(|i| {
                Finding::new(
                    &UNPINNED,
                    Some(Subject::Interface(i.id())),
                    "the code of this interface was never pinned",
                )
            })
            .collect()
    }
}
