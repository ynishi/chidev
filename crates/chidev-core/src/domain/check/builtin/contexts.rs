//! Checks on what a Context has.

use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::vo::finding::{RuleId, Subject};

/// A Context with no Interface.
#[derive(Debug, Clone, Copy)]
pub struct NoInterface;

static NO_INTERFACE: Descriptor = Descriptor {
    id: RuleId::from_static("context.no-interface"),
    summary: "a context with no interface",
    help: "A context starts as a name; its code is described by its interfaces. Put one once there is code, or \
           leave the note while the context is being drawn.",
    hooks: &[Hook::Put, Hook::Check],
    default_level: Level::Note,
    mandatory: false,
    params: &[],
    executor: Executor::Builtin,
};

impl Check for NoInterface {
    fn descriptor(&self) -> &'static Descriptor {
        &NO_INTERFACE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        view.after
            .contexts()
            .filter(|c| c.interfaces().next().is_none())
            .map(|c| {
                Finding::new(
                    &NO_INTERFACE,
                    Some(Subject::Context(c.id())),
                    format!("context `{}` has no interface", c.name()),
                )
            })
            .collect()
    }
}

/// A Context with no Harness.
#[derive(Debug, Clone, Copy)]
pub struct NoHarness;

static NO_HARNESS: Descriptor = Descriptor {
    id: RuleId::from_static("context.no-harness"),
    summary: "a context with no harness",
    help: "Without a harness nothing checks the context's code, and nothing can be pinned. Put one once there are \
           cases to cite, or leave the note while the context is being drawn.",
    hooks: &[Hook::Put, Hook::Check],
    default_level: Level::Note,
    mandatory: false,
    params: &[],
    executor: Executor::Builtin,
};

impl Check for NoHarness {
    fn descriptor(&self) -> &'static Descriptor {
        &NO_HARNESS
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        view.after
            .contexts()
            .filter(|c| c.harnesses().next().is_none())
            .map(|c| {
                Finding::new(
                    &NO_HARNESS,
                    Some(Subject::Context(c.id())),
                    format!("context `{}` has no harness", c.name()),
                )
            })
            .collect()
    }
}
