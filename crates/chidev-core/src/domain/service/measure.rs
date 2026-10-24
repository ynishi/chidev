//! Telling whether a measurement still describes the Project: a mutation run stands on the code of a Pin and on
//! what that Pin stands on (the Interface, the Harnesses that check it, the Contracts they cite), a simulation on
//! the sim Harness, the Contracts its Cases cite a Clause of and the Interfaces it builds with the code pinned for
//! them; both on the settings
//! they were run with. The Project moves on; this reads how far.

use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::service::pin::current_basis;
use crate::domain::service::sim::basis_of;
use crate::domain::vo::id::{HarnessId, InterfaceId};
use crate::domain::vo::measure::MeasureKind;

/// Whether a measurement still describes the Project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// What it stood on and the settings are as they were measured.
    Current,
    /// Something it stood on has moved: the Pin was made again with other code, the Interface, a Harness or a
    /// Contract changed, the subject is gone, or the settings differ.
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

/// Whether the mutation run `measurement` still describes `project` and the Pin of its Interface. `settings` is
/// what the settings digest is now, when the runner can say; without it the settings are not compared.
pub fn standing(
    measurement: &Measurement,
    project: &Project,
    pin: &Pin,
    settings: Option<&str>,
) -> Standing {
    let (Some(stamp), Some(interface)) = (measurement.stamp().code(), measurement.interface())
    else {
        return Standing::Stale;
    };
    match current_basis(project, interface) {
        Some(basis) if stamp.holds(pin.code(), &basis, settings) => Standing::Current,
        _ => Standing::Stale,
    }
}

/// Whether the simulation `measurement` still describes `project`, given the Pins as they are now. A sim Harness
/// that is gone, or no longer of the kind, has no basis to hold.
pub fn sim_standing(
    measurement: &Measurement,
    project: &Project,
    pins: &[Pin],
    settings: Option<&str>,
) -> Standing {
    let (Some(stamp), Some(harness)) = (measurement.stamp().sim(), measurement.harness()) else {
        return Standing::Stale;
    };
    match basis_of(project, harness, pins) {
        Some(basis) if stamp.holds(&basis, settings) => Standing::Current,
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
        .find(|m| m.kind() == kind && m.interface() == Some(interface))
}

/// The simulation of `harness` among `all`, the last one if more than one is given.
pub fn latest_sim(all: &[Measurement], harness: HarnessId) -> Option<&Measurement> {
    all.iter()
        .rev()
        .find(|m| m.kind() == MeasureKind::Simulate && m.harness() == Some(harness))
}
