//! Telling whether a measurement still describes the Project: a measurement stands on the code of a Pin and on what
//! that Pin stands on (the Interface, the Harnesses that check it, the Contracts they cite), and on the settings it
//! was run with. The Project moves on; this reads how far.

use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::service::pin::current_basis;
use crate::domain::vo::id::InterfaceId;
use crate::domain::vo::measure::MeasureKind;

/// Whether a measurement still describes the Project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// The Pin's code, what it stands on, and the settings are as they were measured.
    Current,
    /// Something it stood on has moved: the Pin was made again with other code, the Interface, a Harness or a
    /// Contract changed, the Interface is gone, or the settings differ.
    Stale,
}

impl Standing {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Standing::Current => "current",
            Standing::Stale => "stale",
        }
    }
}

/// Whether `measurement` still describes `project` and the Pin of its Interface. `settings` is what the settings
/// digest is now, when the runner can say; without it the settings are not compared.
pub fn standing(
    measurement: &Measurement,
    project: &Project,
    pin: &Pin,
    settings: Option<&str>,
) -> Standing {
    match current_basis(project, measurement.subject()) {
        Some(basis) if measurement.stamp().holds(pin.code(), &basis, settings) => Standing::Current,
        _ => Standing::Stale,
    }
}

/// The measurement of `kind` for `interface` among `all`, the last one if more than one is given (the store gives
/// the newest of each subject; a later one replaces an earlier one).
pub fn latest(
    all: &[Measurement],
    kind: MeasureKind,
    interface: InterfaceId,
) -> Option<&Measurement> {
    all.iter()
        .rev()
        .find(|m| m.kind() == kind && m.subject() == interface)
}
