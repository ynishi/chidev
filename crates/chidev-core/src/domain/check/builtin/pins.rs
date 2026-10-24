//! Checks on the Pins.

use std::borrow::Cow;

use super::ALL_HOOKS;
use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::finding::Mention;
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::service::pin::{StaleReason, Staleness, staleness};
use crate::domain::vo::finding::{RuleId, Subject};

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
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Pin, Hook::Check]),
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
                let (message, mentions) = match staleness(pin, view.after) {
                    Staleness::Fresh => return None,
                    Staleness::InterfaceGone => {
                        ("the pinned interface is gone".to_string(), Vec::new())
                    }
                    // What moved is named by whoever has the names (the use case), after the message.
                    Staleness::Stale(why) => (
                        "the code was pinned against what has moved".to_string(),
                        why.iter()
                            .map(|r| match r {
                                StaleReason::Interface => Mention::Interface(pin.interface()),
                                StaleReason::Harness(id) => Mention::Harness(*id),
                                StaleReason::Contract(id) => Mention::Contract(*id),
                                StaleReason::UsedInterface(id) => Mention::Interface(*id),
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
                    .mentioning(mentions),
                )
            })
            .collect()
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
    hooks: Cow::Borrowed(ALL_HOOKS),
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
        view.after
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
