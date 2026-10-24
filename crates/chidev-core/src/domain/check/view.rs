//! What a check reads: the model before and after the change, and what is kept beside the Project.

use std::collections::BTreeMap;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::id::{HarnessId, InterfaceId};

/// The model a check is run on. It is everything a check may look at, so that a check is a function of it alone.
#[derive(Debug, Clone, Copy)]
pub struct CheckView<'a> {
    /// The Project before the change; none when nothing changed (the on-demand hook) or none was kept.
    pub before: Option<&'a Project>,
    /// The Project as it is, or as the use case would keep it.
    pub after: &'a Project,
    /// Every Pin, as the use case would leave them.
    pub pins: &'a [Pin],
    /// Every change request, as the use case would leave them.
    pub change_requests: &'a [ChangeRequest],
    /// The latest measurement of each subject (an Interface for a mutation run, a sim Harness for a simulation)
    /// and kind that has one. The hooks of a change give none: a measurement is read by the on-demand hook, and a
    /// change does not move it.
    pub measurements: &'a [Measurement],
    /// What the settings digest of the measure is now, for each Interface the runner can say it for; an
    /// Interface it cannot say it for (no runner, or no layout for it) is absent, and its measurements are not
    /// compared on settings.
    pub measure_settings: &'a BTreeMap<InterfaceId, String>,
    /// The same for the sim Harnesses: what the settings digest of a simulation of each is now, when the runner
    /// can say it (no runner, no layout for the script, or no `[measure.sim]` for the Context: absent).
    pub sim_settings: &'a BTreeMap<HarnessId, String>,
}
