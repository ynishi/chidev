//! The Measurement: the record of one run of a measure. It is a fact, not a judgment: it says what was run and what
//! became of it, tied by a stamp to what it measured. A mutation run is of one Interface's code and says what became
//! of each mutant; a simulation is of one sim Harness and says what each case of the script found. The rules read
//! it and say what is worth a person's attention; a newer run of the same measure replaces it as the current one,
//! and the older ones stay as history.

use std::collections::BTreeSet;

use crate::domain::vo::id::{HarnessId, InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    MeasureError, MeasureFailure, MeasureKind, MeasureSubject, MutantOutcome, MutantResult, Stamp,
    StepEvidence,
};
use crate::domain::vo::sim::SimOutcome;
use crate::domain::vo::timestamp::Timestamp;

/// How many mutants ended each way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Did not build.
    pub stillborn: u32,
    /// Noticed by a test.
    pub killed: u32,
    /// Not noticed by any test.
    pub survived: u32,
    /// Did not end in time.
    pub timeout: u32,
}

/// What a run gave, before it is checked.
#[derive(Debug, Clone)]
pub struct MeasurementDraft {
    /// The id the caller made.
    pub id: MeasurementId,
    /// What was measured.
    pub kind: MeasureKind,
    /// What was measured: an Interface for a mutation run, a Harness for a simulation.
    pub subject: MeasureSubject,
    /// What the run stands on.
    pub stamp: Stamp,
    /// When the run began.
    pub at: Timestamp,
    /// How many places the provider found to change, before it chose among them (a mutation run).
    pub sites: u32,
    /// How many mutants the provider gave to run: at most `max_mutants` of the sites. Fewer than the sites means the
    /// run was capped; more than the mutants below means it stopped early, and then it failed (a mutation run).
    pub planned: u32,
    /// Each mutant that was run and what became of it (a mutation run).
    pub mutants: Vec<MutantResult>,
    /// What each case of the script found, and how the second runs compared (a simulation).
    pub sim: Option<SimOutcome>,
    /// Why the run failed, when it did. A failed run keeps what it got through, as history; the rules do not
    /// judge it.
    pub failure: Option<MeasureFailure>,
    /// What the commands of the run did.
    pub evidence: Vec<StepEvidence>,
}

/// One run of a measure over one subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    id: MeasurementId,
    kind: MeasureKind,
    subject: MeasureSubject,
    stamp: Stamp,
    at: Timestamp,
    sites: u32,
    planned: u32,
    mutants: Vec<MutantResult>,
    sim: Option<SimOutcome>,
    failure: Option<MeasureFailure>,
    evidence: Vec<StepEvidence>,
}

impl Measurement {
    /// The run `draft` describes. A mutation run is of an Interface, stands on a code stamp, has mutants with
    /// different keys, no more than the provider gave, and the provider gave no more than there were sites; a run
    /// that did not fail evaluated every mutant it was given. A simulation is of a Harness, stands on a sim stamp
    /// and has no mutants.
    pub fn new(draft: MeasurementDraft) -> Result<Measurement, MeasureError> {
        let not = |expected: &'static str, what: &'static str| MeasureError::NotOfKind {
            kind: draft.kind.word(),
            expected,
            what,
        };
        match draft.kind {
            MeasureKind::Mutate => {
                if !matches!(draft.subject, MeasureSubject::Interface(_)) {
                    return Err(not("an interface", "a harness subject"));
                }
                if draft.stamp.code().is_none() {
                    return Err(not("an interface", "a simulation's stamp"));
                }
                if draft.sim.is_some() {
                    return Err(not("an interface", "a simulation's result"));
                }
            }
            MeasureKind::Simulate => {
                if !matches!(draft.subject, MeasureSubject::Harness(_)) {
                    return Err(not("a harness", "an interface subject"));
                }
                if draft.stamp.sim().is_none() {
                    return Err(not("a harness", "a mutation run's stamp"));
                }
                if !draft.mutants.is_empty() || draft.sites != 0 || draft.planned != 0 {
                    return Err(not("a harness", "mutants"));
                }
            }
        }
        let mut seen = BTreeSet::new();
        for m in &draft.mutants {
            if !seen.insert(m.mutant.key().clone()) {
                return Err(MeasureError::DuplicateKey(m.mutant.key().to_string()));
            }
        }
        if draft.mutants.len() > draft.planned as usize {
            return Err(MeasureError::MoreMutantsThanPlanned {
                mutants: draft.mutants.len(),
                planned: draft.planned,
            });
        }
        if draft.planned > draft.sites {
            return Err(MeasureError::MorePlannedThanSites {
                planned: draft.planned,
                sites: draft.sites,
            });
        }
        if draft.failure.is_none() && draft.mutants.len() < draft.planned as usize {
            return Err(MeasureError::UnfinishedWithoutFailure {
                mutants: draft.mutants.len(),
                planned: draft.planned,
            });
        }
        Ok(Measurement {
            id: draft.id,
            kind: draft.kind,
            subject: draft.subject,
            stamp: draft.stamp,
            at: draft.at,
            sites: draft.sites,
            planned: draft.planned,
            mutants: draft.mutants,
            sim: draft.sim,
            failure: draft.failure,
            evidence: draft.evidence,
        })
    }

    /// Its id.
    pub fn id(&self) -> MeasurementId {
        self.id
    }

    /// What it measured.
    pub fn kind(&self) -> MeasureKind {
        self.kind
    }

    /// What it is of.
    pub fn subject(&self) -> MeasureSubject {
        self.subject
    }

    /// The Interface whose code it measured; none for a simulation.
    pub fn interface(&self) -> Option<InterfaceId> {
        match self.subject {
            MeasureSubject::Interface(i) => Some(i),
            MeasureSubject::Harness(_) => None,
        }
    }

    /// The sim Harness it ran; none for a mutation run.
    pub fn harness(&self) -> Option<HarnessId> {
        match self.subject {
            MeasureSubject::Harness(h) => Some(h),
            MeasureSubject::Interface(_) => None,
        }
    }

    /// What it stands on.
    pub fn stamp(&self) -> &Stamp {
        &self.stamp
    }

    /// When it began.
    pub fn at(&self) -> Timestamp {
        self.at
    }

    /// How many places the provider found to change.
    pub fn sites(&self) -> u32 {
        self.sites
    }

    /// How many mutants the provider gave to run (at most `max_mutants` of the sites).
    pub fn planned(&self) -> u32 {
        self.planned
    }

    /// How many mutants were run and have an outcome.
    pub fn evaluated(&self) -> u32 {
        u32::try_from(self.mutants.len()).unwrap_or(u32::MAX)
    }

    /// Whether the run ended having evaluated fewer mutants than the code has sites, because the provider was
    /// asked for no more than `max_mutants`. A run that failed is not capped: it did not end, and says why. What
    /// the run left out is not measured, so an `ok` run that is capped does not show the whole code.
    pub fn capped(&self) -> bool {
        self.failure.is_none() && self.kind == MeasureKind::Mutate && self.planned < self.sites
    }

    /// The mutants it ran, in the order the provider gave them.
    pub fn mutants(&self) -> &[MutantResult] {
        &self.mutants
    }

    /// What the cases of the script gave; none for a mutation run.
    pub fn sim(&self) -> Option<&SimOutcome> {
        self.sim.as_ref()
    }

    /// Why it failed; none when it ran to its end.
    pub fn failure(&self) -> Option<&MeasureFailure> {
        self.failure.as_ref()
    }

    /// What its commands did.
    pub fn evidence(&self) -> &[StepEvidence] {
        &self.evidence
    }

    /// How many mutants ended each way.
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for m in &self.mutants {
            let slot = match m.outcome {
                MutantOutcome::Stillborn => &mut counts.stillborn,
                MutantOutcome::Killed => &mut counts.killed,
                MutantOutcome::Survived => &mut counts.survived,
                MutantOutcome::Timeout => &mut counts.timeout,
            };
            *slot += 1;
        }
        counts
    }

    /// The mutants no test noticed, in order.
    pub fn survivors(&self) -> impl Iterator<Item = &MutantResult> {
        self.mutants
            .iter()
            .filter(|m| m.outcome == MutantOutcome::Survived)
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A measurement to test the rules and the store with.

    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::test_uuid;
    use crate::domain::vo::measure::{MeasureStamp, Mutant, MutantKey, Operator, Site};
    use crate::domain::vo::pin_basis::PinBasis;
    use crate::domain::vo::sim::{SimBasis, SimStamp};

    pub(crate) fn result(key: &str, line: u32, outcome: MutantOutcome) -> MutantResult {
        MutantResult {
            mutant: Mutant::new(
                MutantKey::try_from(key).unwrap(),
                Site::new(line, 5).unwrap(),
                Operator::try_from("rel").unwrap(),
                "<".into(),
                "<=".into(),
            )
            .unwrap(),
            outcome,
        }
    }

    pub(crate) fn evidence() -> Vec<StepEvidence> {
        vec![StepEvidence {
            step: "baseline-test".into(),
            command: "cargo test".into(),
            command_digest: Some("ab".repeat(32)),
            exit_code: Some(0),
            duration_ms: 12,
            stderr: String::new(),
        }]
    }

    pub(crate) fn draft(
        n: u128,
        subject: InterfaceId,
        code: Digest,
        basis: PinBasis,
        mutants: Vec<MutantResult>,
    ) -> MeasurementDraft {
        MeasurementDraft {
            id: MeasurementId::from_uuid(test_uuid(n)),
            kind: MeasureKind::Mutate,
            subject: MeasureSubject::Interface(subject),
            stamp: MeasureStamp::new(code, basis, "settings").unwrap().into(),
            at: Timestamp::from_millis(n as u64),
            sites: mutants.len() as u32 + 2,
            planned: mutants.len() as u32,
            mutants,
            sim: None,
            failure: None,
            evidence: evidence(),
        }
    }

    /// A simulation of `harness` that stood on `basis`, with this outcome.
    pub(crate) fn sim_draft(
        n: u128,
        harness: HarnessId,
        basis: SimBasis,
        outcome: SimOutcome,
    ) -> MeasurementDraft {
        MeasurementDraft {
            id: MeasurementId::from_uuid(test_uuid(n)),
            kind: MeasureKind::Simulate,
            subject: MeasureSubject::Harness(harness),
            stamp: SimStamp::new(basis, "settings").unwrap().into(),
            at: Timestamp::from_millis(n as u64),
            sites: 0,
            planned: 0,
            mutants: Vec::new(),
            sim: Some(outcome),
            failure: None,
            evidence: evidence(),
        }
    }

    pub(crate) fn empty_basis() -> PinBasis {
        PinBasis::new(
            crate::domain::vo::revision::Revision::first(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::test_uuid;
    use crate::domain::vo::measure::FailureKind;
    use crate::domain::vo::sim::SimBasis;

    fn iface() -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(100))
    }

    #[test]
    fn a_run_counts_its_mutants_by_outcome_and_names_the_survivors() {
        let m = Measurement::new(draft(
            1,
            iface(),
            Digest::of(b"c"),
            empty_basis(),
            vec![
                result("a", 1, MutantOutcome::Killed),
                result("b", 2, MutantOutcome::Survived),
                result("c", 3, MutantOutcome::Stillborn),
                result("d", 4, MutantOutcome::Killed),
                result("e", 5, MutantOutcome::Timeout),
            ],
        ))
        .unwrap();
        assert_eq!(
            m.counts(),
            Counts {
                stillborn: 1,
                killed: 2,
                survived: 1,
                timeout: 1
            }
        );
        let survivors: Vec<_> = m.survivors().map(|s| s.mutant.key().as_str()).collect();
        assert_eq!(survivors, ["b"]);
        assert_eq!(m.sites(), 7);
        assert_eq!(m.interface(), Some(iface()));
        assert_eq!(m.harness(), None);
    }

    #[test]
    fn a_run_is_capped_when_it_gave_fewer_mutants_than_it_found_sites_and_did_not_fail() {
        let run = |sites: u32, planned: u32, mutants: usize, failure: bool| {
            let results = (0..mutants)
                .map(|n| result(&format!("k{n}"), n as u32 + 1, MutantOutcome::Killed))
                .collect();
            let mut d = draft(1, iface(), Digest::of(b"c"), empty_basis(), results);
            d.sites = sites;
            d.planned = planned;
            if failure {
                d.failure = Some(MeasureFailure::of(FailureKind::Budget, "used up"));
            }
            Measurement::new(d)
        };
        // 41 sites, 40 given and evaluated: capped, with the numbers to say so
        let capped = run(41, 40, 40, false).unwrap();
        assert!(capped.capped());
        assert_eq!(
            (capped.sites(), capped.planned(), capped.evaluated()),
            (41, 40, 40)
        );
        // every site evaluated: not capped
        assert!(!run(40, 40, 40, false).unwrap().capped());
        assert!(!run(0, 0, 0, false).unwrap().capped());
        // a run that stopped early (the budget) is a failure and not a cap, whatever the sites
        let stopped = run(41, 40, 12, true).unwrap();
        assert!(!stopped.capped());
        assert_eq!(stopped.evaluated(), 12);
        // fewer evaluated than given, with no failure to say why, is not a measurement
        assert_eq!(
            run(41, 40, 12, false).unwrap_err(),
            MeasureError::UnfinishedWithoutFailure {
                mutants: 12,
                planned: 40
            }
        );
        // a simulation is never capped
        let harness = HarnessId::from_uuid(test_uuid(400));
        let sim =
            Measurement::new(sim_draft(2, harness, sim_basis(), SimOutcome::default())).unwrap();
        assert!(!sim.capped());
    }

    #[test]
    fn a_key_given_twice_or_more_mutants_than_the_provider_gave_is_refused() {
        let twice = draft(
            1,
            iface(),
            Digest::of(b"c"),
            empty_basis(),
            vec![
                result("a", 1, MutantOutcome::Killed),
                result("a", 2, MutantOutcome::Killed),
            ],
        );
        assert_eq!(
            Measurement::new(twice).unwrap_err(),
            MeasureError::DuplicateKey("a".into())
        );
        let mut many = draft(
            1,
            iface(),
            Digest::of(b"c"),
            empty_basis(),
            vec![result("a", 1, MutantOutcome::Killed)],
        );
        many.planned = 0;
        assert_eq!(
            Measurement::new(many).unwrap_err(),
            MeasureError::MoreMutantsThanPlanned {
                mutants: 1,
                planned: 0
            }
        );
        let mut beyond = draft(
            1,
            iface(),
            Digest::of(b"c"),
            empty_basis(),
            vec![result("a", 1, MutantOutcome::Killed)],
        );
        beyond.sites = 0;
        assert_eq!(
            Measurement::new(beyond).unwrap_err(),
            MeasureError::MorePlannedThanSites {
                planned: 1,
                sites: 0
            }
        );
    }

    fn sim_basis() -> SimBasis {
        SimBasis {
            harness: crate::domain::vo::revision::Revision::first(),
            contracts: Default::default(),
            interfaces: Default::default(),
        }
    }

    #[test]
    fn a_simulation_is_of_a_harness_and_a_mutation_run_of_an_interface() {
        let harness = HarnessId::from_uuid(test_uuid(400));
        let sim =
            Measurement::new(sim_draft(1, harness, sim_basis(), SimOutcome::default())).unwrap();
        assert_eq!(sim.harness(), Some(harness));
        assert_eq!(sim.interface(), None);
        assert!(sim.sim().is_some());
        assert_eq!(sim.counts(), Counts::default());

        // the pieces of one kind do not go with the other
        let mut wrong_subject = sim_draft(2, harness, sim_basis(), SimOutcome::default());
        wrong_subject.subject = MeasureSubject::Interface(iface());
        assert!(matches!(
            Measurement::new(wrong_subject),
            Err(MeasureError::NotOfKind { .. })
        ));
        let mut with_mutants = sim_draft(3, harness, sim_basis(), SimOutcome::default());
        with_mutants.mutants = vec![result("a", 1, MutantOutcome::Killed)];
        with_mutants.sites = 1;
        with_mutants.planned = 1;
        assert!(matches!(
            Measurement::new(with_mutants),
            Err(MeasureError::NotOfKind { .. })
        ));
        let mut mutate_with_sim = draft(4, iface(), Digest::of(b"c"), empty_basis(), vec![]);
        mutate_with_sim.sim = Some(SimOutcome::default());
        assert!(matches!(
            Measurement::new(mutate_with_sim),
            Err(MeasureError::NotOfKind { .. })
        ));
        let mut mutate_with_sim_stamp = sim_draft(5, harness, sim_basis(), SimOutcome::default());
        mutate_with_sim_stamp.kind = MeasureKind::Mutate;
        mutate_with_sim_stamp.subject = MeasureSubject::Interface(iface());
        assert!(matches!(
            Measurement::new(mutate_with_sim_stamp),
            Err(MeasureError::NotOfKind { .. })
        ));
    }
}
