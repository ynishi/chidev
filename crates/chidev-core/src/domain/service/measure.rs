//! Telling whether a measurement still describes the Project: a mutation run stands on the code of a Pin and on
//! what that Pin stands on (the Interface, the Harnesses that check it, the Contracts they cite), a simulation on
//! the sim Harness, the Contracts its Cases cite a Clause of and the Interfaces it builds with the code pinned for
//! them; both on the settings
//! they were run with. The Project moves on; this reads how far, and which part moved.

use crate::domain::entity::measurement::{Measurement, MutationRun, SimulationRun};
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::service::pin::current_basis;
use crate::domain::service::sim::basis_of;
use crate::domain::vo::id::{HarnessId, InterfaceId};
use crate::domain::vo::measure::{SettingsNow, StampPart};

/// The parts of a stamp that moved, at least one: the only way to have it is [`Standing::of`], so a stale standing
/// always says what moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved(Vec<StampPart>);

impl Moved {
    /// The parts that moved, in the order the stamp gives them.
    pub fn parts(&self) -> &[StampPart] {
        &self.0
    }

    /// What moved, said in a line for a finding: the word of each part ([`StampPart::word`]) once, in the order they
    /// first come, joined by commas. Settings that could not be compared say why beside their word:
    /// `settings-unconfigured (measure is not configured)`.
    pub fn said(&self) -> String {
        let mut said: Vec<String> = Vec::new();
        for part in &self.0 {
            let one = match part.why() {
                Some(why) => format!("{} ({why})", part.word()),
                None => part.word().to_string(),
            };
            if !said.contains(&one) {
                said.push(one);
            }
        }
        said.join(", ")
    }
}

/// Whether a measurement still describes the Project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// What it stood on and the settings are as they were measured.
    Current,
    /// Something it stood on has moved: the Pin was made again with other code, the Interface, a Harness or a
    /// Contract changed, the subject is gone, the settings differ, or the settings cannot be read now (there is
    /// no `[measure]` table, or the runner cannot say them for this subject).
    Stale(Moved),
}

impl Standing {
    /// The standing of a measurement of which these `parts` have moved: current when none did.
    pub fn of(parts: Vec<StampPart>) -> Standing {
        if parts.is_empty() {
            Standing::Current
        } else {
            Standing::Stale(Moved(parts))
        }
    }

    /// Whether it is stale.
    pub fn is_stale(&self) -> bool {
        matches!(self, Standing::Stale(_))
    }
}

/// Whether the mutation run `run` still describes `project` and the Pin of its Interface. `settings` is
/// what the settings are now; a run whose settings are not known (`Unconfigured`, `Unreadable`) is stale, for there
/// is nothing to show it was made under them. An Interface that is not in the Project any more is `Gone`.
pub fn standing(
    run: MutationRun<'_>,
    project: &Project,
    pin: &Pin,
    settings: &SettingsNow,
) -> Standing {
    Standing::of(match current_basis(project, run.interface()) {
        Some(basis) => run.stamp().moved(pin.code(), &basis, settings),
        None => vec![StampPart::Gone],
    })
}

/// Whether the simulation `run` still describes `project`, given the Pins as they are now. A sim Harness
/// that is gone, or no longer of the kind, is `Gone`: it has no basis to compare with. The settings are judged as
/// [`standing`] judges them.
pub fn sim_standing(
    run: SimulationRun<'_>,
    project: &Project,
    pins: &[Pin],
    settings: &SettingsNow,
) -> Standing {
    Standing::of(match basis_of(project, run.harness(), pins) {
        Some(basis) => run.stamp().moved(&basis, settings),
        None => vec![StampPart::Gone],
    })
}

/// The mutation run for `interface` among `all`, the last one if more than one is given (the store gives the
/// newest of each subject; a later one replaces an earlier one).
pub fn latest(all: &[Measurement], interface: InterfaceId) -> Option<MutationRun<'_>> {
    all.iter()
        .rev()
        .filter_map(Measurement::as_mutation)
        .find(|run| run.interface() == interface)
}

/// The simulation of `harness` among `all`, the last one if more than one is given.
pub fn latest_sim(all: &[Measurement], harness: HarnessId) -> Option<SimulationRun<'_>> {
    all.iter()
        .rev()
        .filter_map(Measurement::as_simulation)
        .find(|run| run.harness() == harness)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::{ContractId, test_uuid};
    use crate::domain::vo::measure::SettingsGap;
    use crate::domain::vo::pin_basis::StaleReason;

    #[test]
    fn nothing_moved_is_current_and_a_stale_standing_always_says_what_moved() {
        assert_eq!(Standing::of(Vec::new()), Standing::Current);
        let standing = Standing::of(vec![StampPart::Code]);
        assert!(standing.is_stale());
        assert!(!Standing::Current.is_stale());
        let Standing::Stale(moved) = standing else {
            panic!("a part moved");
        };
        assert_eq!(moved.parts(), [StampPart::Code]);
    }

    #[test]
    fn a_subject_that_is_not_in_the_project_any_more_is_gone() {
        use crate::domain::entity::measurement::fixture::{
            draft, empty_basis, same_settings, sim_draft,
        };
        use crate::domain::vo::digest::Digest;
        use crate::domain::vo::id::{PinId, ProjectId};
        use crate::domain::vo::revision::Revision;
        use crate::domain::vo::sim::SimBasis;

        let project = Project::new(ProjectId::from_uuid(test_uuid(1)));
        let gone = Standing::of(vec![StampPart::Gone]);
        let interface = InterfaceId::from_uuid(test_uuid(100));
        let mutation = Measurement::new(draft(
            1,
            interface,
            Digest::of(b"c"),
            empty_basis(),
            Vec::new(),
        ))
        .unwrap();
        let pin = Pin::new(
            PinId::from_uuid(test_uuid(9)),
            interface,
            Digest::of(b"c"),
            empty_basis(),
        );
        let run = mutation.as_mutation().expect("a mutation run");
        assert_eq!(standing(run, &project, &pin, &same_settings()), gone);
        let basis = SimBasis {
            harness: Revision::first(),
            contracts: Default::default(),
            interfaces: Default::default(),
        };
        let sim = Measurement::new(sim_draft(
            2,
            HarnessId::from_uuid(test_uuid(400)),
            basis,
            Default::default(),
        ))
        .unwrap();
        let run = sim.as_simulation().expect("a simulation");
        assert_eq!(sim_standing(run, &project, &[], &same_settings()), gone);
    }

    #[test]
    fn the_words_of_what_moved_come_once_each_in_order() {
        let contract = ContractId::from_uuid(test_uuid(3));
        let Standing::Stale(moved) = Standing::of(vec![
            StampPart::Code,
            StampPart::Pin(StaleReason::Contract(contract)),
            StampPart::Pin(StaleReason::Interface),
            StampPart::Contract(contract),
            StampPart::Settings(SettingsGap::Differ),
        ]) else {
            panic!("parts moved");
        };
        assert_eq!(moved.said(), "code, contract, interface, settings");
    }

    #[test]
    fn settings_that_cannot_be_compared_say_why_beside_the_other_parts() {
        let said = |gap: SettingsGap| {
            let Standing::Stale(moved) =
                Standing::of(vec![StampPart::Code, StampPart::Settings(gap)])
            else {
                panic!("parts moved");
            };
            moved.said()
        };
        assert_eq!(said(SettingsGap::Differ), "code, settings");
        assert_eq!(
            said(SettingsGap::Unconfigured),
            "code, settings-unconfigured (measure is not configured)"
        );
        assert_eq!(
            said(SettingsGap::Unreadable("no layout".to_string())),
            "code, settings-unreadable (the settings cannot be read: no layout)"
        );
    }
}
