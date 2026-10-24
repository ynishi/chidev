//! The Measurement: the record of one run of a measure over one Interface's code. It is a fact, not a judgment: it
//! says what was run and what became of each mutant, tied by a stamp to the code and the Harness it measured. The
//! rules read it and say what is worth a person's attention; a newer run of the same measure replaces it as the
//! current one, and the older ones stay as history.

use std::collections::BTreeSet;

use crate::domain::vo::id::{InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    MeasureError, MeasureFailure, MeasureKind, MeasureStamp, MutantOutcome, MutantResult,
    StepEvidence,
};
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
    /// The Interface whose code was measured.
    pub subject: InterfaceId,
    /// What the run stands on.
    pub stamp: MeasureStamp,
    /// When the run began.
    pub at: Timestamp,
    /// How many places the provider found to change, before it chose among them.
    pub sites: u32,
    /// Each mutant that was run and what became of it.
    pub mutants: Vec<MutantResult>,
    /// Why the run failed, when it did. A failed run keeps the mutants it got through, as history; the rules do
    /// not judge them.
    pub failure: Option<MeasureFailure>,
    /// What the commands of the run did.
    pub evidence: Vec<StepEvidence>,
}

/// One run of a measure over one Interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    id: MeasurementId,
    kind: MeasureKind,
    subject: InterfaceId,
    stamp: MeasureStamp,
    at: Timestamp,
    sites: u32,
    mutants: Vec<MutantResult>,
    failure: Option<MeasureFailure>,
    evidence: Vec<StepEvidence>,
}

impl Measurement {
    /// The run `draft` describes. The keys of its mutants are all different, and it has no more mutants than sites.
    pub fn new(draft: MeasurementDraft) -> Result<Measurement, MeasureError> {
        let mut seen = BTreeSet::new();
        for m in &draft.mutants {
            if !seen.insert(m.mutant.key().clone()) {
                return Err(MeasureError::DuplicateKey(m.mutant.key().to_string()));
            }
        }
        if draft.mutants.len() > draft.sites as usize {
            return Err(MeasureError::MoreMutantsThanSites {
                mutants: draft.mutants.len(),
                sites: draft.sites,
            });
        }
        Ok(Measurement {
            id: draft.id,
            kind: draft.kind,
            subject: draft.subject,
            stamp: draft.stamp,
            at: draft.at,
            sites: draft.sites,
            mutants: draft.mutants,
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

    /// The Interface whose code it measured.
    pub fn subject(&self) -> InterfaceId {
        self.subject
    }

    /// What it stands on.
    pub fn stamp(&self) -> &MeasureStamp {
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

    /// The mutants it ran, in the order the provider gave them.
    pub fn mutants(&self) -> &[MutantResult] {
        &self.mutants
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
    use crate::domain::vo::measure::{Mutant, MutantKey, Operator, Site};
    use crate::domain::vo::pin_basis::PinBasis;

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
            subject,
            stamp: MeasureStamp::new(code, basis, "settings").unwrap(),
            at: Timestamp::from_millis(n as u64),
            sites: mutants.len() as u32 + 2,
            mutants,
            failure: None,
            evidence: vec![StepEvidence {
                step: "baseline-test".into(),
                command: "cargo test".into(),
                command_digest: Some("ab".repeat(32)),
                exit_code: Some(0),
                duration_ms: 12,
                stderr: String::new(),
            }],
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
    }

    #[test]
    fn a_key_given_twice_or_more_mutants_than_sites_is_refused() {
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
        many.sites = 0;
        assert_eq!(
            Measurement::new(many).unwrap_err(),
            MeasureError::MoreMutantsThanSites {
                mutants: 1,
                sites: 0
            }
        );
    }
}
