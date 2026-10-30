//! The Measurement: the record of one run of a measure. It is a fact, not a judgment: it says what was run and what
//! became of it, tied by a stamp to what it measured. What a run holds is its [`MeasureBody`], one shape for each
//! kind: a mutation run is of one Interface's code and says what became of each mutant; a simulation is of one sim
//! Harness and says what each case of the script found; a run of examples is of one Harness of examples and says
//! what became of each example. The rules read it and say what is worth a person's attention; a newer run of the
//! same measure replaces it as the current one, and the older ones stay as history.

use std::collections::BTreeSet;
use std::fmt;

use crate::domain::vo::correction::{
    Correctable, Correction, CorrectionError, SoughtText, check_places, masked,
};
use crate::domain::vo::examples_run::{ExampleResult, ExamplesOutcome, ExamplesStamp};
use crate::domain::vo::id::{HarnessId, InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    MeasureError, MeasureFailure, MeasureKind, MeasureStamp, MeasureSubject, Mutant, MutantOutcome,
    MutantResult, StampRef, StepEvidence,
};
use crate::domain::vo::sim::{CaseExit, CaseRun, SimOutcome, SimStamp};
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

/// What a run is, by its kind: what it measured, what it stood on, and what it observed. A kind holds only the
/// pieces that belong to it, so a run cannot be of an Interface and stand on a simulation's stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasureBody {
    /// Single-site mutants of the code of an Interface.
    Mutation {
        /// The Interface whose code was measured.
        interface: InterfaceId,
        /// The code of the Pin the run stands on, and the settings.
        stamp: MeasureStamp,
        /// How many places the provider found to change, before it chose among them.
        sites: u32,
        /// How many mutants the provider gave to run: at most `max_mutants` of the sites. Fewer than the sites means
        /// the run was capped; more than the mutants below means it stopped early, and then it failed.
        planned: u32,
        /// Each mutant that was run and what became of it.
        mutants: Vec<MutantResult>,
    },
    /// A sim Harness run as a driver of the program built from the code of the Pins it cites.
    Simulation {
        /// The sim Harness that was run.
        harness: HarnessId,
        /// The Harness and what it cites, as the run stood on them, and the settings.
        stamp: SimStamp,
        /// What each case of the script found, and how the second runs compared.
        outcome: SimOutcome,
    },
    /// A Harness of examples run against the program built from the code of the Pins of the Interfaces it builds.
    Examples {
        /// The Harness of examples that was run.
        harness: HarnessId,
        /// The Harness as the run stood on it, with the code pinned for what it builds, and the settings.
        stamp: ExamplesStamp,
        /// What became of each example.
        outcome: ExamplesOutcome,
    },
}

/// What a run gave, before it is checked.
#[derive(Debug, Clone)]
pub struct MeasurementDraft {
    /// The id the caller made.
    pub id: MeasurementId,
    /// When the run began.
    pub at: Timestamp,
    /// What was measured, what the run stands on, and what it found.
    pub body: MeasureBody,
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
    at: Timestamp,
    body: MeasureBody,
    failure: Option<MeasureFailure>,
    evidence: Vec<StepEvidence>,
    /// The corrections of its texts, in the order they were made. Empty from [`Measurement::new`].
    corrections: Vec<Correction<MeasurementField>>,
}

impl Measurement {
    /// The run `draft` describes. A mutation run has mutants with different keys, no more than the provider gave,
    /// and the provider gave no more than there were sites; a run that did not fail evaluated every mutant it was
    /// given. A simulation and a run of examples have no value to check here (an outcome of examples refuses a Case
    /// twice when it is made).
    pub fn new(draft: MeasurementDraft) -> Result<Measurement, MeasureError> {
        if let MeasureBody::Mutation {
            sites,
            planned,
            mutants,
            ..
        } = &draft.body
        {
            let mut seen = BTreeSet::new();
            for m in mutants {
                if !seen.insert(m.mutant.key().clone()) {
                    return Err(MeasureError::DuplicateKey(m.mutant.key().to_string()));
                }
            }
            if mutants.len() > *planned as usize {
                return Err(MeasureError::MoreMutantsThanPlanned {
                    mutants: mutants.len(),
                    planned: *planned,
                });
            }
            if planned > sites {
                return Err(MeasureError::MorePlannedThanSites {
                    planned: *planned,
                    sites: *sites,
                });
            }
            if draft.failure.is_none() && mutants.len() < *planned as usize {
                return Err(MeasureError::UnfinishedWithoutFailure {
                    mutants: mutants.len(),
                    planned: *planned,
                });
            }
        }
        Ok(Measurement {
            id: draft.id,
            at: draft.at,
            body: draft.body,
            failure: draft.failure,
            evidence: draft.evidence,
            corrections: Vec::new(),
        })
    }

    /// The run with the `corrections` a store kept for it.
    pub(crate) fn with_corrections(
        mut self,
        corrections: Vec<Correction<MeasurementField>>,
    ) -> Measurement {
        self.corrections = corrections;
        self
    }

    /// The corrections made of its texts, in the order they were made: each keeps the fields, why, who and when,
    /// and no text.
    pub fn corrections(&self) -> &[Correction<MeasurementField>] {
        &self.corrections
    }

    /// Its id.
    pub fn id(&self) -> MeasurementId {
        self.id
    }

    /// What it holds, by its kind.
    pub fn body(&self) -> &MeasureBody {
        &self.body
    }

    /// What it measured.
    pub fn kind(&self) -> MeasureKind {
        match self.body {
            MeasureBody::Mutation { .. } => MeasureKind::Mutate,
            MeasureBody::Simulation { .. } => MeasureKind::Simulate,
            MeasureBody::Examples { .. } => MeasureKind::Examples,
        }
    }

    /// What it is of.
    pub fn subject(&self) -> MeasureSubject {
        match &self.body {
            MeasureBody::Mutation { interface, .. } => MeasureSubject::Interface(*interface),
            MeasureBody::Simulation { harness, .. } | MeasureBody::Examples { harness, .. } => {
                MeasureSubject::Harness(*harness)
            }
        }
    }

    /// The Interface whose code it measured; none for a run of a Harness.
    pub fn interface(&self) -> Option<InterfaceId> {
        match self.subject() {
            MeasureSubject::Interface(i) => Some(i),
            MeasureSubject::Harness(_) => None,
        }
    }

    /// The Harness it ran, a sim Harness or a Harness of examples; none for a mutation run.
    pub fn harness(&self) -> Option<HarnessId> {
        match self.subject() {
            MeasureSubject::Harness(h) => Some(h),
            MeasureSubject::Interface(_) => None,
        }
    }

    /// What it stands on.
    pub fn stamp(&self) -> StampRef<'_> {
        match &self.body {
            MeasureBody::Mutation { stamp, .. } => StampRef::Code(stamp),
            MeasureBody::Simulation { stamp, .. } => StampRef::Sim(stamp),
            MeasureBody::Examples { stamp, .. } => StampRef::Examples(stamp),
        }
    }

    /// It as a mutation run; none for any other kind.
    pub fn as_mutation(&self) -> Option<MutationRun<'_>> {
        match &self.body {
            MeasureBody::Mutation {
                interface, stamp, ..
            } => Some(MutationRun {
                interface: *interface,
                stamp,
                measurement: self,
            }),
            MeasureBody::Simulation { .. } | MeasureBody::Examples { .. } => None,
        }
    }

    /// It as a simulation; none for any other kind.
    pub fn as_simulation(&self) -> Option<SimulationRun<'_>> {
        match &self.body {
            MeasureBody::Simulation { harness, stamp, .. } => Some(SimulationRun {
                harness: *harness,
                stamp,
                measurement: self,
            }),
            MeasureBody::Mutation { .. } | MeasureBody::Examples { .. } => None,
        }
    }

    /// It as a run of examples; none for any other kind.
    pub fn as_examples(&self) -> Option<ExamplesRun<'_>> {
        match &self.body {
            MeasureBody::Examples { harness, stamp, .. } => Some(ExamplesRun {
                harness: *harness,
                stamp,
                measurement: self,
            }),
            MeasureBody::Mutation { .. } | MeasureBody::Simulation { .. } => None,
        }
    }

    /// When it began.
    pub fn at(&self) -> Timestamp {
        self.at
    }

    /// How many places the provider found to change; 0 for any run but a mutation run.
    pub fn sites(&self) -> u32 {
        match &self.body {
            MeasureBody::Mutation { sites, .. } => *sites,
            MeasureBody::Simulation { .. } | MeasureBody::Examples { .. } => 0,
        }
    }

    /// How many mutants the provider gave to run (at most `max_mutants` of the sites); 0 for any run but a mutation
    /// run.
    pub fn planned(&self) -> u32 {
        match &self.body {
            MeasureBody::Mutation { planned, .. } => *planned,
            MeasureBody::Simulation { .. } | MeasureBody::Examples { .. } => 0,
        }
    }

    /// How many mutants were run and have an outcome.
    pub fn evaluated(&self) -> u32 {
        u32::try_from(self.mutants().len()).unwrap_or(u32::MAX)
    }

    /// Whether the run ended having evaluated fewer mutants than the code has sites, because the provider was
    /// asked for no more than `max_mutants`. A run that failed is not capped: it did not end, and says why. What
    /// the run left out is not measured, so an `ok` run that is capped does not show the whole code.
    pub fn capped(&self) -> bool {
        self.failure.is_none() && self.planned() < self.sites()
    }

    /// The mutants it ran, in the order the provider gave them; none for any run but a mutation run.
    pub fn mutants(&self) -> &[MutantResult] {
        match &self.body {
            MeasureBody::Mutation { mutants, .. } => mutants,
            MeasureBody::Simulation { .. } | MeasureBody::Examples { .. } => &[],
        }
    }

    /// What the cases of the script gave; none for any other kind.
    pub fn sim(&self) -> Option<&SimOutcome> {
        match &self.body {
            MeasureBody::Simulation { outcome, .. } => Some(outcome),
            MeasureBody::Mutation { .. } | MeasureBody::Examples { .. } => None,
        }
    }

    /// What became of each example; none for any other kind.
    pub fn examples(&self) -> Option<&ExamplesOutcome> {
        match &self.body {
            MeasureBody::Examples { outcome, .. } => Some(outcome),
            MeasureBody::Mutation { .. } | MeasureBody::Simulation { .. } => None,
        }
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
        for m in self.mutants() {
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
        self.mutants()
            .iter()
            .filter(|m| m.outcome == MutantOutcome::Survived)
    }
}

/// A measurement seen as the mutation run it is: the Interface whose code was measured and the stamp it stands on,
/// with the measurement itself for what else a caller reads of it (the failure, the mutants). The kind is chosen
/// once, by [`Measurement::as_mutation`], so what reads a mutation run has no other kind to turn away.
#[derive(Debug, Clone, Copy)]
pub struct MutationRun<'a> {
    interface: InterfaceId,
    stamp: &'a MeasureStamp,
    measurement: &'a Measurement,
}

impl<'a> MutationRun<'a> {
    /// The Interface whose code was measured.
    pub fn interface(&self) -> InterfaceId {
        self.interface
    }

    /// What the run stands on.
    pub fn stamp(&self) -> &'a MeasureStamp {
        self.stamp
    }

    /// The measurement.
    pub fn measurement(&self) -> &'a Measurement {
        self.measurement
    }
}

/// A measurement seen as the simulation it is: the sim Harness that was run and the stamp it stands on, with the
/// measurement itself for what else a caller reads of it (the failure, the outcome). The kind is chosen once, by
/// [`Measurement::as_simulation`].
#[derive(Debug, Clone, Copy)]
pub struct SimulationRun<'a> {
    harness: HarnessId,
    stamp: &'a SimStamp,
    measurement: &'a Measurement,
}

impl<'a> SimulationRun<'a> {
    /// The sim Harness that was run.
    pub fn harness(&self) -> HarnessId {
        self.harness
    }

    /// What the simulation stands on.
    pub fn stamp(&self) -> &'a SimStamp {
        self.stamp
    }

    /// The measurement.
    pub fn measurement(&self) -> &'a Measurement {
        self.measurement
    }
}

/// A measurement seen as the run of examples it is: the Harness of examples that was run and the stamp it stands on,
/// with the measurement itself for what else a caller reads of it (the failure, the outcome). The kind is chosen
/// once, by [`Measurement::as_examples`].
#[derive(Debug, Clone, Copy)]
pub struct ExamplesRun<'a> {
    harness: HarnessId,
    stamp: &'a ExamplesStamp,
    measurement: &'a Measurement,
}

impl<'a> ExamplesRun<'a> {
    /// The Harness of examples that was run.
    pub fn harness(&self) -> HarnessId {
        self.harness
    }

    /// What the run stands on.
    pub fn stamp(&self) -> &'a ExamplesStamp {
        self.stamp
    }

    /// The measurement.
    pub fn measurement(&self) -> &'a Measurement {
        self.measurement
    }
}

/// A place of text in a measurement, for a correction. An index counts from 0 in the list it is of. The settings and
/// the names that key a row (a mutant's key, the name of a case, the Case an example is of) are not fields: they are
/// part of what the run stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MeasurementField {
    /// The command of a step.
    StepCommand {
        /// The step, in the order of the evidence.
        step: usize,
    },
    /// What a step wrote on standard error.
    StepStderr {
        /// The step, in the order of the evidence.
        step: usize,
    },
    /// Why the run failed.
    Failure,
    /// The text a mutant replaces.
    MutantFrom {
        /// The mutant, in the order the provider gave them.
        mutant: usize,
    },
    /// The text a mutant puts in.
    MutantTo {
        /// The mutant, in the order the provider gave them.
        mutant: usize,
    },
    /// What a case of a simulation found, in the script's words.
    CaseFinding {
        /// The case, in the order they were run.
        case: usize,
        /// The finding of the case.
        finding: usize,
    },
    /// The situation a case did not reach, in the script's words.
    CaseUnreached {
        /// The case, in the order they were run.
        case: usize,
        /// The Clause it did not reach, in the order the case reported them.
        unreached: usize,
    },
    /// Why the script of a case could not decide something.
    CaseUndecided {
        /// The case, in the order they were run.
        case: usize,
        /// The line, in the order the case reported them.
        undecided: usize,
    },
    /// Why the script of a case said it could not start the program (`CANNOT_START <text>`), in the script's words.
    /// Only a case that ended so has it; the other ways a case ends hold no text.
    CaseExit {
        /// The case, in the order they were run.
        case: usize,
    },
    /// The text of a reason an example of a run failed: what the program gave, or why it could not be run. Only a
    /// reason that holds a text has it; the others (an exit code, a final line feed) hold none.
    ExampleReason {
        /// The example, in the order they were run.
        example: usize,
        /// The reason, in the order the judge told them.
        reason: usize,
    },
}

impl MeasurementField {
    /// The words for the field, which a message and an answer name it by: `step[1].stderr`, `failure`,
    /// `case[0].finding[1]`.
    pub fn word(&self) -> String {
        match self {
            MeasurementField::StepCommand { step } => format!("step[{step}].command"),
            MeasurementField::StepStderr { step } => format!("step[{step}].stderr"),
            MeasurementField::Failure => "failure".to_string(),
            MeasurementField::MutantFrom { mutant } => format!("mutant[{mutant}].from"),
            MeasurementField::MutantTo { mutant } => format!("mutant[{mutant}].to"),
            MeasurementField::CaseFinding { case, finding } => {
                format!("case[{case}].finding[{finding}]")
            }
            MeasurementField::CaseUnreached { case, unreached } => {
                format!("case[{case}].unreached[{unreached}]")
            }
            MeasurementField::CaseUndecided { case, undecided } => {
                format!("case[{case}].undecided[{undecided}]")
            }
            MeasurementField::CaseExit { case } => format!("case[{case}].exit"),
            MeasurementField::ExampleReason { example, reason } => {
                format!("example[{example}].reason[{reason}]")
            }
        }
    }
}

impl fmt::Display for MeasurementField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.word())
    }
}

impl Correctable for Measurement {
    type Field = MeasurementField;

    fn texts(&self) -> Vec<(MeasurementField, &str)> {
        let mut out = Vec::new();
        for (step, e) in self.evidence.iter().enumerate() {
            if !e.command.is_empty() {
                out.push((MeasurementField::StepCommand { step }, e.command.as_str()));
            }
            if !e.stderr.is_empty() {
                out.push((MeasurementField::StepStderr { step }, e.stderr.as_str()));
            }
        }
        if let Some(failure) = &self.failure {
            out.push((MeasurementField::Failure, failure.message()));
        }
        for (mutant, m) in self.mutants().iter().enumerate() {
            out.push((MeasurementField::MutantFrom { mutant }, m.mutant.from()));
            out.push((MeasurementField::MutantTo { mutant }, m.mutant.to()));
        }
        if let Some(sim) = self.sim() {
            for (case, run) in sim.cases.iter().enumerate() {
                for (finding, f) in run.findings.iter().enumerate() {
                    out.push((
                        MeasurementField::CaseFinding { case, finding },
                        f.text.as_str(),
                    ));
                }
                for (unreached, u) in run.unreached.iter().enumerate() {
                    out.push((
                        MeasurementField::CaseUnreached { case, unreached },
                        u.text.as_str(),
                    ));
                }
                for (undecided, text) in run.undecided.iter().enumerate() {
                    out.push((
                        MeasurementField::CaseUndecided { case, undecided },
                        text.as_str(),
                    ));
                }
                if let CaseExit::CannotStart(why) = &run.exit {
                    out.push((MeasurementField::CaseExit { case }, why.as_str()));
                }
            }
        }
        if let Some(examples) = self.examples() {
            for (example, record) in examples.records().iter().enumerate() {
                let ExampleResult::Failed(reasons) = &record.result else {
                    continue;
                };
                for (reason, r) in reasons.iter().enumerate() {
                    if let Some(text) = r.text() {
                        out.push((MeasurementField::ExampleReason { example, reason }, text));
                    }
                }
            }
        }
        out
    }

    fn correct(
        &mut self,
        sought: &SoughtText,
        correction: Correction<MeasurementField>,
        _taken: &dyn Fn(&MeasurementField, &str) -> bool,
    ) -> Result<(), CorrectionError> {
        check_places(self, sought, correction.fields())?;
        let mut next = self.clone();
        for field in correction.fields() {
            next.replace_text(*field, sought)?;
        }
        next.corrections.push(correction);
        *self = next;
        Ok(())
    }
}

impl Measurement {
    /// Replaces the text sought in one field, and builds the value again with its own constructor. Called on a
    /// copy: an error leaves it half done, and the copy is dropped.
    fn replace_text(
        &mut self,
        field: MeasurementField,
        sought: &SoughtText,
    ) -> Result<(), CorrectionError> {
        let word = field.word();
        let text = self
            .texts()
            .into_iter()
            .find(|(f, _)| *f == field)
            .map(|(_, t)| t.to_string())
            .ok_or_else(|| CorrectionError::NoSuchField(word.clone()))?;
        let new = masked(sought, &word, &text)?;
        let invalid = || CorrectionError::Invalid(word.clone());
        let no_such = || CorrectionError::NoSuchField(word.clone());
        match field {
            MeasurementField::StepCommand { step } => {
                self.evidence.get_mut(step).ok_or_else(no_such)?.command = new;
            }
            MeasurementField::StepStderr { step } => {
                self.evidence.get_mut(step).ok_or_else(no_such)?.stderr = new;
            }
            MeasurementField::Failure => {
                let kind = self.failure.as_ref().ok_or_else(no_such)?.kind();
                self.failure = Some(MeasureFailure::new(kind, new).map_err(|_| invalid())?);
            }
            MeasurementField::MutantFrom { mutant } | MeasurementField::MutantTo { mutant } => {
                let MeasureBody::Mutation { mutants, .. } = &mut self.body else {
                    return Err(no_such());
                };
                let m = mutants.get_mut(mutant).ok_or_else(no_such)?;
                let (from, to) = if matches!(field, MeasurementField::MutantFrom { .. }) {
                    (new, m.mutant.to().to_string())
                } else {
                    (m.mutant.from().to_string(), new)
                };
                m.mutant = Mutant::new(
                    m.mutant.key().clone(),
                    m.mutant.site(),
                    m.mutant.operator().clone(),
                    from,
                    to,
                )
                .map_err(|_| invalid())?;
            }
            MeasurementField::CaseFinding { case, finding } => {
                let run = self.case_mut(case).ok_or_else(no_such)?;
                run.findings.get_mut(finding).ok_or_else(no_such)?.text = new;
            }
            MeasurementField::CaseUnreached { case, unreached } => {
                let run = self.case_mut(case).ok_or_else(no_such)?;
                run.unreached.get_mut(unreached).ok_or_else(no_such)?.text = new;
            }
            MeasurementField::CaseUndecided { case, undecided } => {
                let run = self.case_mut(case).ok_or_else(no_such)?;
                *run.undecided.get_mut(undecided).ok_or_else(no_such)? = new;
            }
            MeasurementField::CaseExit { case } => {
                // The exit keeps its rule: one line that says something.
                if new.trim().is_empty() || new.contains('\n') {
                    return Err(invalid());
                }
                let run = self.case_mut(case).ok_or_else(no_such)?;
                if !run.exit.cannot_start() {
                    return Err(no_such());
                }
                run.exit = CaseExit::CannotStart(new);
            }
            MeasurementField::ExampleReason { example, reason } => {
                let MeasureBody::Examples { outcome, .. } = &mut self.body else {
                    return Err(no_such());
                };
                let record = outcome.records_mut().get_mut(example).ok_or_else(no_such)?;
                let ExampleResult::Failed(reasons) = &mut record.result else {
                    return Err(no_such());
                };
                let slot = reasons.get_mut(reason).ok_or_else(no_such)?;
                // a reason that holds no text is no place
                *slot = slot.with_text(new).ok_or_else(no_such)?;
            }
        }
        Ok(())
    }

    /// The run of a case of the simulation, to change.
    fn case_mut(&mut self, case: usize) -> Option<&mut CaseRun> {
        match &mut self.body {
            MeasureBody::Simulation { outcome, .. } => outcome.cases.get_mut(case),
            MeasureBody::Mutation { .. } | MeasureBody::Examples { .. } => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A measurement to test the rules and the store with.

    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::examples_run::ExamplesBasis;
    use crate::domain::vo::id::test_uuid;
    use crate::domain::vo::measure::{
        MeasureStamp, Mutant, MutantKey, Operator, SettingsDigest, SettingsNow, Site,
    };
    use crate::domain::vo::pin_basis::PinBasis;
    use crate::domain::vo::sim::{SimBasis, SimStamp};

    /// The settings the runs of this fixture are made with: the digest of the word `settings`.
    pub(crate) fn settings() -> SettingsDigest {
        settings_of("settings")
    }

    /// The settings that are the digest of `word`.
    pub(crate) fn settings_of(word: &str) -> SettingsDigest {
        SettingsDigest::new(Digest::of(word.as_bytes()))
    }

    /// The settings as they are now when they are those the runs of this fixture were made with.
    pub(crate) fn same_settings() -> SettingsNow {
        SettingsNow::Known(settings())
    }

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
            at: Timestamp::from_millis(n as u64),
            body: MeasureBody::Mutation {
                interface: subject,
                stamp: MeasureStamp::new(code, basis, settings()),
                sites: mutants.len() as u32 + 2,
                planned: mutants.len() as u32,
                mutants,
            },
            failure: None,
            evidence: evidence(),
        }
    }

    /// Gives a mutation run `sites` found and `planned` given, whatever it had.
    pub(crate) fn set_counts(d: &mut MeasurementDraft, sites: u32, planned: u32) {
        match &mut d.body {
            MeasureBody::Mutation {
                sites: s,
                planned: p,
                ..
            } => {
                *s = sites;
                *p = planned;
            }
            MeasureBody::Simulation { .. } | MeasureBody::Examples { .. } => {
                panic!("only a mutation run has sites")
            }
        }
    }

    /// A run of the examples of `harness` that stood on `basis`, with this outcome.
    pub(crate) fn examples_draft(
        n: u128,
        harness: HarnessId,
        basis: ExamplesBasis,
        outcome: ExamplesOutcome,
    ) -> MeasurementDraft {
        MeasurementDraft {
            id: MeasurementId::from_uuid(test_uuid(n)),
            at: Timestamp::from_millis(n as u64),
            body: MeasureBody::Examples {
                harness,
                stamp: ExamplesStamp::new(basis, settings()),
                outcome,
            },
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
            at: Timestamp::from_millis(n as u64),
            body: MeasureBody::Simulation {
                harness,
                stamp: SimStamp::new(basis, settings()),
                outcome,
            },
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
            set_counts(&mut d, sites, planned);
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
        set_counts(&mut many, 3, 0);
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
        set_counts(&mut beyond, 0, 1);
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
    fn a_simulation_reads_its_body_and_has_no_mutants() {
        let harness = HarnessId::from_uuid(test_uuid(400));
        let sim =
            Measurement::new(sim_draft(1, harness, sim_basis(), SimOutcome::default())).unwrap();
        assert_eq!(sim.kind(), MeasureKind::Simulate);
        assert_eq!(sim.subject(), MeasureSubject::Harness(harness));
        assert_eq!(sim.harness(), Some(harness));
        assert_eq!(sim.interface(), None);
        assert!(sim.mutants().is_empty());
        assert_eq!((sim.sites(), sim.planned(), sim.evaluated()), (0, 0, 0));
        assert!(sim.sim().is_some());
        assert!(sim.stamp().sim().is_some());
        assert!(sim.stamp().code().is_none());
        assert_eq!(sim.counts(), Counts::default());
    }

    #[test]
    fn a_run_is_seen_as_its_own_kind_and_as_no_other() {
        let mutation = Measurement::new(draft(
            1,
            iface(),
            Digest::of(b"c"),
            empty_basis(),
            vec![result("a", 1, MutantOutcome::Killed)],
        ))
        .unwrap();
        let run = mutation.as_mutation().expect("a mutation run");
        assert_eq!(run.interface(), iface());
        assert_eq!(StampRef::Code(run.stamp()), mutation.stamp());
        assert_eq!(run.measurement(), &mutation);
        assert!(mutation.as_simulation().is_none());

        let harness = HarnessId::from_uuid(test_uuid(400));
        let sim =
            Measurement::new(sim_draft(2, harness, sim_basis(), SimOutcome::default())).unwrap();
        let run = sim.as_simulation().expect("a simulation");
        assert_eq!(run.harness(), harness);
        assert_eq!(StampRef::Sim(run.stamp()), sim.stamp());
        assert_eq!(run.measurement(), &sim);
        assert!(sim.as_mutation().is_none());
    }

    // --- correction

    use MeasurementField as Field;

    const TOKEN: &str = "tok-SECRET-123";

    fn sought() -> SoughtText {
        SoughtText::try_from(TOKEN).unwrap()
    }

    fn correction(fields: Vec<Field>) -> Correction<Field> {
        Correction::new(
            fields,
            crate::domain::vo::prose::Prose::try_from("a token was pasted").unwrap(),
            crate::domain::vo::answer::DecisionMaker::person("carol").unwrap(),
            Timestamp::from_millis(88),
        )
        .unwrap()
    }

    fn free(_: &Field, _: &str) -> bool {
        false
    }

    fn step(command: &str, stderr: &str) -> StepEvidence {
        StepEvidence {
            step: "baseline-test".into(),
            command: command.into(),
            command_digest: None,
            exit_code: Some(1),
            duration_ms: 3,
            stderr: stderr.into(),
        }
    }

    fn mutant_with(from: &str, to: &str) -> MutantResult {
        use crate::domain::vo::measure::{MutantKey, Operator, Site};
        MutantResult {
            mutant: Mutant::new(
                MutantKey::try_from("k1").unwrap(),
                Site::new(1, 1).unwrap(),
                Operator::try_from("rel").unwrap(),
                from.into(),
                to.into(),
            )
            .unwrap(),
            outcome: MutantOutcome::Survived,
        }
    }

    /// A mutation run that failed, with the token in a command, a stderr, the failure and a mutant.
    fn leaky_run() -> Measurement {
        let mut d = draft(
            1,
            iface(),
            Digest::of(b"c"),
            empty_basis(),
            vec![mutant_with(&format!("a {TOKEN}"), &format!("b {TOKEN}"))],
        );
        d.evidence = vec![
            step(&format!("run --token {TOKEN}"), &format!("bad {TOKEN}")),
            step("cargo test", ""),
        ];
        set_counts(&mut d, 2, 2);
        d.failure = Some(MeasureFailure::of(
            FailureKind::Baseline,
            format!("failed with {TOKEN}"),
        ));
        Measurement::new(d).unwrap()
    }

    /// A simulation with the token in a finding, a situation not reached and a line not decided.
    fn leaky_sim() -> Measurement {
        use crate::domain::vo::id::ClauseId;
        use crate::domain::vo::sim::{CaseExit, CaseFinding, CaseUnreached, SimCase};
        let clause = ClauseId::from_uuid(test_uuid(7));
        let run = CaseRun {
            case: SimCase::Seed(0),
            exit: CaseExit::Normal,
            findings: vec![
                CaseFinding {
                    step: 1,
                    clause,
                    text: "plain".into(),
                },
                CaseFinding {
                    step: 2,
                    clause,
                    text: format!("lost {TOKEN}"),
                },
            ],
            unreached: vec![CaseUnreached {
                clause,
                text: format!("needs {TOKEN}"),
            }],
            undecided: vec![format!("cannot tell {TOKEN}")],
            schedule: None,
            trace: None,
        };
        let outcome = SimOutcome {
            cases: vec![run],
            reruns: Vec::new(),
        };
        let harness = HarnessId::from_uuid(test_uuid(400));
        Measurement::new(sim_draft(2, harness, sim_basis(), outcome)).unwrap()
    }

    fn every_field(m: &Measurement) -> Vec<Field> {
        m.texts().into_iter().map(|(f, _)| f).collect()
    }

    #[test]
    fn a_measurement_lists_every_text_it_holds_in_a_fixed_order() {
        let words =
            |m: &Measurement| -> Vec<String> { every_field(m).iter().map(Field::word).collect() };
        // an empty stderr is not a text that is set
        assert_eq!(
            words(&leaky_run()),
            [
                "step[0].command",
                "step[0].stderr",
                "step[1].command",
                "failure",
                "mutant[0].from",
                "mutant[0].to",
            ]
        );
        assert_eq!(
            words(&leaky_sim()),
            [
                "step[0].command",
                "case[0].finding[0]",
                "case[0].finding[1]",
                "case[0].unreached[0]",
                "case[0].undecided[0]",
            ]
        );
    }

    #[test]
    fn a_correction_replaces_the_text_and_keeps_what_the_run_stands_on() {
        for before in [leaky_run(), leaky_sim()] {
            let mut m = before.clone();
            let fields: Vec<Field> = m
                .texts()
                .into_iter()
                .filter(|(_, t)| t.contains(TOKEN))
                .map(|(f, _)| f)
                .collect();
            m.correct(&sought(), correction(fields), &free).unwrap();
            assert!(m.texts().iter().all(|(_, t)| !t.contains(TOKEN)));
            assert_eq!(m.id(), before.id());
            assert_eq!(m.stamp(), before.stamp());
            assert_eq!(m.at(), before.at());
            assert_eq!(m.counts(), before.counts());
            assert_eq!(m.sites(), before.sites());
            assert_eq!(m.planned(), before.planned());
            assert_eq!(m.corrections().len(), 1);
            assert_eq!(m.corrections()[0].at().millis(), 88);
            assert!(!format!("{:?}", m.corrections()).contains("SECRET"));
            assert!(before.corrections().is_empty());
        }
        let mut m = leaky_run();
        m.correct(
            &sought(),
            correction(vec![Field::StepStderr { step: 0 }, Field::Failure]),
            &free,
        )
        .unwrap();
        assert_eq!(m.evidence()[0].stderr, "bad [redacted]");
        assert_eq!(m.failure().unwrap().message(), "failed with [redacted]");
        assert_eq!(m.failure().unwrap().kind(), FailureKind::Baseline);
        // what was not named still holds it
        assert!(m.evidence()[0].command.contains(TOKEN));
        assert!(m.mutants()[0].mutant.from().contains(TOKEN));
        let mut s = leaky_sim();
        s.correct(
            &sought(),
            correction(vec![Field::CaseFinding {
                case: 0,
                finding: 1,
            }]),
            &free,
        )
        .unwrap();
        let run = &s.sim().unwrap().cases[0];
        assert_eq!(run.findings[1].text, "lost [redacted]");
        assert_eq!(run.findings[1].step, 2);
        assert!(run.unreached[0].text.contains(TOKEN));
    }

    #[test]
    fn a_correction_is_all_or_nothing_and_names_only_texts_that_are_there() {
        let mut m = leaky_run();
        let before = m.clone();
        for (field, refusal) in [
            // an empty stderr is not a field
            (
                Field::StepStderr { step: 1 },
                CorrectionError::NoSuchField("step[1].stderr".into()),
            ),
            (
                Field::MutantTo { mutant: 4 },
                CorrectionError::NoSuchField("mutant[4].to".into()),
            ),
            // a mutation run has no cases
            (
                Field::CaseFinding {
                    case: 0,
                    finding: 0,
                },
                CorrectionError::NoSuchField("case[0].finding[0]".into()),
            ),
        ] {
            assert_eq!(
                m.correct(&sought(), correction(vec![Field::Failure, field]), &free),
                Err(refusal)
            );
            assert_eq!(m, before);
        }
        // a field without the text
        assert_eq!(
            m.correct(
                &sought(),
                correction(vec![Field::StepCommand { step: 1 }]),
                &free
            ),
            Err(CorrectionError::TextAbsent("step[1].command".into()))
        );
        assert_eq!(m, before);
    }

    #[test]
    fn a_mutant_the_correction_would_make_a_no_change_is_refused_by_its_own_constructor() {
        // `a tok` -> `a [redacted]` is the mutant that was already `a [redacted]` (it changes nothing)
        let d = draft(
            1,
            iface(),
            Digest::of(b"c"),
            empty_basis(),
            vec![mutant_with(&format!("a {TOKEN}"), "a [redacted]")],
        );
        let mut m = Measurement::new(d).unwrap();
        let before = m.clone();
        assert_eq!(
            m.correct(
                &sought(),
                correction(vec![Field::MutantFrom { mutant: 0 }]),
                &free
            ),
            Err(CorrectionError::Invalid("mutant[0].from".into()))
        );
        assert_eq!(m, before);
    }

    /// A simulation whose one case could not start the program, and said why with the token.
    fn cannot_start_sim(why: &str) -> Measurement {
        use crate::domain::vo::sim::SimCase;
        let run = CaseRun {
            case: SimCase::Seed(0),
            exit: CaseExit::CannotStart(why.to_string()),
            findings: Vec::new(),
            unreached: Vec::new(),
            undecided: Vec::new(),
            schedule: None,
            trace: None,
        };
        let outcome = SimOutcome {
            cases: vec![run],
            reruns: Vec::new(),
        };
        let harness = HarnessId::from_uuid(test_uuid(400));
        Measurement::new(sim_draft(3, harness, sim_basis(), outcome)).unwrap()
    }

    #[test]
    fn the_text_of_a_case_that_could_not_start_is_a_place_and_is_corrected_like_the_others() {
        let mut m = cannot_start_sim(&format!("python3 {TOKEN}: No such file"));
        assert_eq!(
            m.texts()
                .iter()
                .map(|(f, _)| f.word())
                .collect::<Vec<_>>()
                .last()
                .map(String::as_str),
            Some("case[0].exit")
        );
        m.correct(
            &sought(),
            correction(vec![Field::CaseExit { case: 0 }]),
            &free,
        )
        .unwrap();
        assert_eq!(
            m.sim().unwrap().cases[0].exit,
            CaseExit::CannotStart("python3 [redacted]: No such file".to_string())
        );
        assert!(m.texts().iter().all(|(_, t)| !t.contains(TOKEN)));
        assert_eq!(m.corrections().len(), 1);

        // the other ends of a case hold no text, so there is no such place; and a case that did not end so is none
        let mut normal = leaky_sim();
        let before = normal.clone();
        assert_eq!(
            normal.correct(
                &sought(),
                correction(vec![Field::CaseExit { case: 0 }]),
                &free
            ),
            Err(CorrectionError::NoSuchField("case[0].exit".into()))
        );
        assert_eq!(normal, before);

        // a text that is the whole of the reason is replaced whole, and the exit keeps saying something
        let mut whole = cannot_start_sim(TOKEN);
        whole
            .correct(
                &sought(),
                correction(vec![Field::CaseExit { case: 0 }]),
                &free,
            )
            .unwrap();
        assert_eq!(
            whole.sim().unwrap().cases[0].exit,
            CaseExit::CannotStart("[redacted]".to_string())
        );
    }

    #[test]
    fn a_stored_measurement_keeps_its_corrections() {
        let mut m = leaky_run();
        m.correct(&sought(), correction(vec![Field::Failure]), &free)
            .unwrap();
        let kept = leaky_run().with_corrections(m.corrections().to_vec());
        assert_eq!(kept.corrections(), m.corrections());
    }

    // --- a run of examples

    use crate::domain::vo::examples_run::{
        ExampleRecord, ExampleResult, ExamplesBasis, ExamplesOutcome, Got, Reason,
    };

    fn examples_basis() -> ExamplesBasis {
        ExamplesBasis {
            harness: crate::domain::vo::revision::Revision::first(),
            builds: Default::default(),
        }
    }

    /// A run of examples: the first failed with the token in several reasons, the second passed, the third failed
    /// with reasons that hold no text.
    fn leaky_examples() -> Measurement {
        use crate::domain::vo::id::CaseId;
        let line = |got: &str| Got::Line {
            line: 2,
            got: Some(got.to_string()),
        };
        let records = vec![
            ExampleRecord {
                case: CaseId::from_uuid(test_uuid(1)),
                result: ExampleResult::Failed(vec![
                    Reason::Output(line(&format!("secret {TOKEN}"))),
                    Reason::Exit { got: 2 },
                    Reason::Errors(line("plain")),
                    Reason::Abnormal(format!("cannot start {TOKEN}")),
                ]),
            },
            ExampleRecord {
                case: CaseId::from_uuid(test_uuid(2)),
                result: ExampleResult::Passed,
            },
            ExampleRecord {
                case: CaseId::from_uuid(test_uuid(3)),
                result: ExampleResult::Failed(vec![
                    Reason::Output(Got::Line { line: 1, got: None }),
                    Reason::Unreadable(format!("missing {TOKEN}.args")),
                ]),
            },
        ];
        Measurement::new(examples_draft(
            4,
            HarnessId::from_uuid(test_uuid(400)),
            examples_basis(),
            ExamplesOutcome::new(records).unwrap(),
        ))
        .unwrap()
    }

    #[test]
    fn a_run_of_examples_reads_its_body_and_has_no_mutants_or_cases() {
        let harness = HarnessId::from_uuid(test_uuid(400));
        let m = leaky_examples();
        assert_eq!(m.kind(), MeasureKind::Examples);
        assert_eq!(m.subject(), MeasureSubject::Harness(harness));
        assert_eq!(m.harness(), Some(harness));
        assert_eq!(m.interface(), None);
        assert!(m.mutants().is_empty() && m.sim().is_none());
        assert_eq!((m.sites(), m.planned(), m.evaluated()), (0, 0, 0));
        assert_eq!(m.counts(), Counts::default());
        assert_eq!(m.examples().unwrap().records().len(), 3);
        assert!(m.stamp().examples().is_some());
        assert!(m.stamp().sim().is_none() && m.stamp().code().is_none());
        let run = m.as_examples().expect("a run of examples");
        assert_eq!(run.harness(), harness);
        assert_eq!(StampRef::Examples(run.stamp()), m.stamp());
        assert_eq!(run.measurement(), &m);
        assert!(m.as_mutation().is_none() && m.as_simulation().is_none());
        // and no other kind is a run of examples
        assert!(leaky_run().as_examples().is_none() && leaky_run().examples().is_none());
        assert!(leaky_sim().as_examples().is_none() && leaky_sim().examples().is_none());
    }

    #[test]
    fn an_example_in_a_run_twice_is_refused_when_the_outcome_is_made() {
        use crate::domain::vo::id::CaseId;
        let record = ExampleRecord {
            case: CaseId::from_uuid(test_uuid(1)),
            result: ExampleResult::Passed,
        };
        assert!(ExamplesOutcome::new(vec![record.clone(), record]).is_err());
    }

    #[test]
    fn only_the_reasons_that_hold_a_text_are_places_of_an_example() {
        let words: Vec<String> = every_field(&leaky_examples())
            .iter()
            .map(Field::word)
            .collect();
        assert_eq!(
            words,
            [
                "step[0].command",
                "example[0].reason[0]",
                "example[0].reason[2]",
                "example[0].reason[3]",
                "example[2].reason[1]",
            ]
        );
    }

    #[test]
    fn a_correction_replaces_the_text_of_a_reason_and_keeps_what_the_run_stands_on() {
        let before = leaky_examples();
        let mut m = before.clone();
        let fields: Vec<Field> = m
            .texts()
            .into_iter()
            .filter(|(_, t)| t.contains(TOKEN))
            .map(|(f, _)| f)
            .collect();
        assert_eq!(fields.len(), 3);
        m.correct(&sought(), correction(fields), &free).unwrap();
        assert!(m.texts().iter().all(|(_, t)| !t.contains(TOKEN)));
        let records = m.examples().unwrap().records();
        assert_eq!(
            records[0].result,
            ExampleResult::Failed(vec![
                Reason::Output(Got::Line {
                    line: 2,
                    got: Some("secret [redacted]".to_string())
                }),
                Reason::Exit { got: 2 },
                Reason::Errors(Got::Line {
                    line: 2,
                    got: Some("plain".to_string())
                }),
                Reason::Abnormal("cannot start [redacted]".to_string()),
            ])
        );
        assert_eq!(
            records[2].result,
            ExampleResult::Failed(vec![
                Reason::Output(Got::Line { line: 1, got: None }),
                Reason::Unreadable("missing [redacted].args".to_string()),
            ])
        );
        assert_eq!(m.stamp(), before.stamp());
        assert_eq!(
            m.examples().unwrap().records()[1],
            before.examples().unwrap().records()[1]
        );
        assert_eq!(m.corrections().len(), 1);
        assert!(!format!("{:?}", m.corrections()).contains("SECRET"));
    }

    #[test]
    fn a_correction_of_a_place_that_an_example_does_not_have_is_refused_and_changes_nothing() {
        let mut m = leaky_examples();
        let before = m.clone();
        for field in [
            // a reason that holds no text
            Field::ExampleReason {
                example: 0,
                reason: 1,
            },
            // a reason that is not there
            Field::ExampleReason {
                example: 0,
                reason: 9,
            },
            // an example that passed has no reason
            Field::ExampleReason {
                example: 1,
                reason: 0,
            },
            // an example that is not there
            Field::ExampleReason {
                example: 7,
                reason: 0,
            },
        ] {
            assert_eq!(
                m.correct(&sought(), correction(vec![field]), &free),
                Err(CorrectionError::NoSuchField(field.word())),
                "{field}"
            );
            assert_eq!(m, before);
        }
        // a run of another kind has no examples
        let mut run = leaky_run();
        let field = Field::ExampleReason {
            example: 0,
            reason: 0,
        };
        assert_eq!(
            run.correct(&sought(), correction(vec![field]), &free),
            Err(CorrectionError::NoSuchField("example[0].reason[0]".into()))
        );
    }
}
