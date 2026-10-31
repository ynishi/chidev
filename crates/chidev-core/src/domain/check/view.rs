//! What a check reads: the model as it stands in one state, and what is kept beside the Project.

use std::collections::BTreeMap;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::crux::Crux;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::id::{ChangeRequestId, HarnessId, InterfaceId};
use crate::domain::vo::measure::SettingsNow;
use crate::domain::vo::tree::TreeNow;

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
    /// Every Crux, as the use case would leave them. A change request does not hold its Cruxes: those raised in it
    /// are the ones here whose `raised_in` is its id.
    pub cruxes: &'a [Crux],
    /// The latest measurement of each subject (an Interface for a mutation run, a Harness for a simulation or a run
    /// of examples) and kind that has one. The moments of a change give none: a measurement is read by the
    /// on-demand moment, and a change does not move it. The confirmations (a pin, a resolution) give the runs of
    /// examples alone, for a confirmation must not claim a state the latest run of them contradicts.
    pub measurements: &'a [Measurement],
    /// What the settings of a mutation run are now, for each Interface that is in the model and whose
    /// measurements are read. An Interface that is absent (it is no longer in the model, or the moment reads no
    /// measurements: an edit) is read by [`CheckView::measure_settings_now`] as settings that cannot be read, so
    /// its measurements are stale.
    pub measure_settings: &'a BTreeMap<InterfaceId, SettingsNow>,
    /// The same for the sim Harnesses: what the settings of a simulation of each are now, absent as above.
    pub sim_settings: &'a BTreeMap<HarnessId, SettingsNow>,
    /// The same for the Harnesses of examples: what the settings of a run of the examples of each are now, absent
    /// as above.
    pub examples_settings: &'a BTreeMap<HarnessId, SettingsNow>,
    /// The copies of the placed assets in the working tree, as they are now. The moments of a change and of a
    /// confirmation give none (every asset is then unplaced, which the checks of the tree say nothing about): a
    /// copy is read by the on-demand moment alone.
    pub tree: &'a TreeNow,
    /// The change request this state is the resolution of: set only when the use case that resolves one runs the
    /// checks, on the state with it already resolved. The checks that judge only unresolved requests judge this one
    /// too, so that the resolution is told what was left (`cr.keep-moved`, `cr.out-of-frame`). None in every other
    /// state.
    pub resolving: Option<ChangeRequestId>,
}

impl CheckView<'_> {
    /// What the settings of a mutation run of `interface` are now; settings that were not read when the view has
    /// none for it.
    pub fn measure_settings_now(&self, interface: InterfaceId) -> SettingsNow {
        SettingsNow::of(self.measure_settings, &interface)
    }

    /// What the settings of a simulation of `harness` are now; settings that were not read when the view has none
    /// for it.
    pub fn sim_settings_now(&self, harness: HarnessId) -> SettingsNow {
        SettingsNow::of(self.sim_settings, &harness)
    }

    /// What the settings of a run of the examples of `harness` are now; settings that were not read when the view
    /// has none for it.
    pub fn examples_settings_now(&self, harness: HarnessId) -> SettingsNow {
        SettingsNow::of(self.examples_settings, &harness)
    }
}

/// A change to the model, as the states on either side of it: what the use case read, and what it would keep.
/// A use case that edits gives the checks of an edit both, so that only what the edit introduced is
/// reported. A use case that confirms (a pin, a resolution) or a person who asks gives one [`CheckView`].
#[derive(Debug, Clone, Copy)]
pub struct Change<'a> {
    /// The state before the change: the Project, the Pins, the change requests and the Cruxes as the store has them.
    pub before: CheckView<'a>,
    /// The state the use case would keep. The Pins, the change requests and the Cruxes may differ from `before` when
    /// the change records something with them (the work of a change request, the decision of a Crux).
    pub after: CheckView<'a>,
}
