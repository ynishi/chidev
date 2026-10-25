//! What a check reads: the model as it stands in one state, and what is kept beside the Project.

use std::collections::BTreeMap;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::id::{HarnessId, InterfaceId};

/// The model a check is run on: one state of it. It is everything a check may look at, so that a check is a
/// function of it alone. A check never sees two states; what a change introduced is found by running the checks
/// on each state of a [`Change`] and comparing what they found (see [`introduced`](super::introduced::introduced)).
#[derive(Debug, Clone, Copy)]
pub struct CheckView<'a> {
    /// The Project as it is, or as the use case would keep it.
    pub project: &'a Project,
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

/// A change to the model, as the states on either side of it: what the use case read, and what it would keep.
/// A use case that edits gives the checks of the `put` hook both, so that only what the edit introduced is
/// reported. A use case that confirms (a pin, a resolution) or a person who asks gives one [`CheckView`].
#[derive(Debug, Clone, Copy)]
pub struct Change<'a> {
    /// The state before the change: the Project, the Pins and the change requests as the store has them.
    pub before: CheckView<'a>,
    /// The state the use case would keep. The Pins and the change requests may differ from `before` when the
    /// change records something with them (the work of a change request).
    pub after: CheckView<'a>,
}
