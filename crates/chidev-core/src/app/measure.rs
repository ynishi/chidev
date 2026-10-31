//! Measuring: the operations that run a measure and keep what it observed, and the reading of what is kept. A
//! mutation run takes the code a Pin accepted and tests it against changes of itself; a simulation builds the
//! program from the code of the Pins of the Interfaces a sim Harness builds and drives it with the script; a run of
//! examples builds the same program and runs each example of a Harness of examples against it. A measure takes
//! minutes and makes facts, so it is an operation of its own and never a moment of the checks; the checks
//! (`measure.failed`, `mutation.*`, `sim.*`, `examples.*`) only judge the record it leaves.

use std::collections::{BTreeMap, BTreeSet};

use super::commit::{RecordChanges, RecordTrigger};
use super::ports::external::ExternalExecutor;
use super::ports::measure::{
    ExamplesRequest, HarnessText, InterfaceText, MutateRequest, RunnerError, SettingsTarget,
    SimulateRequest,
};
use super::ports::{AssetKind, IdSource, Store, StoreRevision};
use super::{App, AppError, CorrectionStatus, context_named, correction_status};
use crate::domain::entity::harness::Harness;
use crate::domain::entity::measurement::{Counts, MeasureBody, Measurement, MeasurementDraft};
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::{Project, ProjectError, refused};
use crate::domain::repository::StoreError;
use crate::domain::service::examples;
use crate::domain::service::measure::{Standing, examples_standing, sim_standing, standing};
use crate::domain::service::pin::current_basis;
use crate::domain::service::sim::{basis_of, cited_in, regress_names, sweep};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::example::ExampleName;
use crate::domain::vo::examples_run::{
    ExampleRecord, ExampleResult, ExamplesOutcome, ExamplesStamp,
};
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{ClauseId, HarnessId, InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    MeasureKind, MeasureStamp, MeasureSubject, SettingsDigest, SettingsNow, StampPart,
};
use crate::domain::vo::name::{AssetName, ContextName};
use crate::domain::vo::pin_basis::PinBasis;
use crate::domain::vo::sim::{CitedClause, ClauseName, SimOutcome, SimStamp};
use crate::domain::vo::source::Source;
use crate::domain::vo::target::Target;

/// How many times a commit that lost a race with another writer is tried again. A measurement stands on a stamp, not
/// on the store's revision, so a write in between does not make it wrong: it is kept, and a stale stamp would show.
const COMMIT_TRIES: usize = 5;

/// The latest measurements, and what the settings are now for the subjects that are in the model.
#[derive(Debug, Default)]
pub(super) struct Measured {
    pub(super) measurements: Vec<Measurement>,
    pub(super) settings: BTreeMap<InterfaceId, SettingsNow>,
    pub(super) sim_settings: BTreeMap<HarnessId, SettingsNow>,
    pub(super) examples_settings: BTreeMap<HarnessId, SettingsNow>,
}

/// Whose settings a target of [`SettingsTarget`] that was asked of the runner are.
enum Asked {
    Interface(InterfaceId),
    SimHarness(HarnessId),
    ExamplesHarness(HarnessId),
}

/// What the settings are now, from what the runner said of them: no runner is a project that has no `[measure]`
/// table, and any other error is a runner that cannot read the settings for this subject, for the reason it gives.
fn settings_now(said: Result<SettingsDigest, RunnerError>) -> SettingsNow {
    match said {
        Ok(digest) => SettingsNow::Known(digest),
        Err(RunnerError::Unconfigured) => SettingsNow::Unconfigured,
        Err(why) => SettingsNow::Unreadable(why.to_string()),
    }
}

/// The Harnesses a mutation run of `interface` runs, by [`tests_run`] over the basis of its Pin. An Interface with
/// no basis (it is not pinned) has no tests to name, and its settings are unreadable for that reason.
fn tests_of(p: &Project, interface: InterfaceId) -> Result<Vec<(&Harness, &Source)>, SettingsNow> {
    let basis = current_basis(p, interface).ok_or_else(|| {
        SettingsNow::Unreadable("the interface is not pinned, so no tests are known".to_string())
    })?;
    Ok(tests_run(p, &basis))
}

/// The Harnesses a mutation run of an Interface runs, which stand in the `basis` of its Pin: the tests that hold a
/// source. A simulation does not test the code against the Interface (it is run by `simulate`), and examples are
/// data, not tests, and have no code to read. One rule for the run and for the settings it is stamped with, so that
/// the settings are those of the Harnesses the run runs.
fn tests_run<'p>(p: &'p Project, basis: &PinBasis) -> Vec<(&'p Harness, &'p Source)> {
    basis
        .harnesses()
        .keys()
        .filter_map(|id| {
            p.contexts()
                .flat_map(|c| c.harnesses())
                .find(|h| h.id() == *id)
        })
        .filter(|h| h.kind() == HarnessKind::Tests)
        .filter_map(|h| Some((h, h.source()?)))
        .collect()
}

/// A mutant and what became of it, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutantLine {
    /// Its key.
    pub key: String,
    /// Where it is: `line:column`.
    pub site: String,
    /// The class of change.
    pub operator: String,
    /// What it changes: `from -> to`.
    pub change: String,
    /// What became of it: `killed`, `survived`, `stillborn` or `timeout`.
    pub outcome: &'static str,
}

/// A promise a case broke, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingLine {
    /// The step of the trace.
    pub step: u32,
    /// The Clause, as a script names it: `<contract>:<label>`; its id when the model has not the Clause any more.
    pub clause: String,
    /// What did not hold.
    pub text: String,
}

/// A Clause a case did not reach, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreachedLine {
    /// The Clause, as a script names it: `<contract>:<label>`; its id when the model has not the Clause any more.
    pub clause: String,
    /// The situation that was needed.
    pub text: String,
}

/// One case of a simulation, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseLine {
    /// The case: `seed-3` or `regress-<name>`.
    pub case: String,
    /// How the script ended: `normal`, `timeout` or `script:<code>`.
    pub exit: String,
    /// The promises it broke.
    pub findings: Vec<FindingLine>,
    /// The Clauses whose situation it did not reach.
    pub unreached: Vec<UnreachedLine>,
    /// What the script could not decide.
    pub undecided: Vec<String>,
    /// The digest of the schedule it ran.
    pub schedule: Option<String>,
    /// The digest of the trace the program made.
    pub trace: Option<String>,
}

/// A case run a second time, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerunLine {
    /// The case.
    pub case: String,
    /// `same`, `schedule-differs` or `trace-differs`.
    pub result: &'static str,
}

/// One example of a run of examples, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleLine {
    /// The name of its Case, as the model has it now; the Case's id when the model has not the Case any more.
    pub example: String,
    /// Whether the program answered as the example expects.
    pub passed: bool,
    /// Why it did not, one reason each, in the order the judge told them; none when it passed.
    pub reasons: Vec<String>,
}

/// A command of a run, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceLine {
    /// Which step.
    pub step: String,
    /// The command.
    pub command: String,
    /// The exit code.
    pub exit_code: Option<i32>,
    /// How long it ran.
    pub duration_ms: u64,
}

/// What a measure did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureDone {
    /// The measurement's id.
    pub id: String,
    /// What was measured.
    pub kind: &'static str,
    /// The stamp's digest.
    pub stamp: String,
    /// Why the run failed (`kind: message`); none when it ran to its end.
    pub failure: Option<String>,
    /// How many places the provider found to change.
    pub sites: u32,
    /// How many mutants were evaluated (the length of `mutants`).
    pub evaluated: u32,
    /// Whether the run ended having evaluated fewer mutants than there are sites, `max_mutants` having limited it
    /// (see `mutation.capped`); a run that failed is not capped.
    pub capped: bool,
    /// How many mutants ended each way.
    pub counts: Counts,
    /// Each mutant that was run.
    pub mutants: Vec<MutantLine>,
    /// Each case of a simulation that was run.
    pub cases: Vec<CaseLine>,
    /// The cases of a simulation that were run a second time.
    pub reruns: Vec<RerunLine>,
    /// Each example of a run of examples that was run.
    pub examples: Vec<ExampleLine>,
    /// What the commands did.
    pub evidence: Vec<EvidenceLine>,
}

/// What a simulation found, in numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimSummary {
    /// How many cases were run.
    pub cases: u32,
    /// How many Clauses have a finding.
    pub findings: u32,
    /// How many Clauses no seed reached.
    pub unreached: u32,
    /// How many cases did not end normally.
    pub abnormal: u32,
    /// How many cases were run a second time.
    pub reruns: u32,
    /// How many of those did not give the same schedule and trace.
    pub differing: u32,
}

/// What a run of examples found, in numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExamplesSummary {
    /// How many examples passed.
    pub passed: u32,
    /// How many examples failed.
    pub failed: u32,
}

/// How much of the code a mutation run looked at: the places the provider found, and the mutants evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    /// How many places the provider found to change.
    pub sites: u32,
    /// How many mutants were evaluated.
    pub evaluated: u32,
    /// Whether `max_mutants` left sites out of an `ok` run.
    pub capped: bool,
}

/// A part of what a measurement stands on that has moved, with the name of the asset it is about. The name is
/// present exactly when the part names an asset ([`StampPart::asset`]): an asset the store has no name for is a
/// store out of step with its model, which is an error and never a part without a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedPart {
    part: StampPart,
    name: Option<AssetName>,
}

impl MovedPart {
    /// The part that moved.
    pub fn part(&self) -> &StampPart {
        &self.part
    }

    /// The name of the asset the part is about; none for a part that is not about one.
    pub fn name(&self) -> Option<&AssetName> {
        self.name.as_ref()
    }
}

/// Where a measurement stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasurementState {
    /// What it stands on and the settings are as measured.
    Current,
    /// Something it stands on has moved: these parts, in the order the stamp gives them. There is at least one.
    Stale(Vec<MovedPart>),
    /// The Interface or Harness it is of was retired since. It is not judged: the thing is not in the model.
    Retired,
    /// It is of an Interface that has no Pin, so there is no accepted code to compare it with.
    Unpinned,
}

impl MeasurementState {
    /// The word for it: `current`, `stale`, `retired` or `unpinned`.
    pub fn word(&self) -> &'static str {
        match self {
            MeasurementState::Current => "current",
            MeasurementState::Stale(_) => "stale",
            MeasurementState::Retired => "retired",
            MeasurementState::Unpinned => "unpinned",
        }
    }
}

/// Where the latest measurement of an Interface or a Harness stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementStanding {
    /// The Context's name.
    pub context: String,
    /// The Interface's name (a mutation run).
    pub interface: Option<String>,
    /// The Harness's name (a simulation or a run of examples).
    pub harness: Option<String>,
    /// What was measured.
    pub kind: &'static str,
    /// The measurement's id.
    pub id: String,
    /// The stamp's digest.
    pub stamp: String,
    /// `Current` while what it stands on and the settings are as measured; `Stale` once any moved, with the parts
    /// that did; `Retired` once the Interface or Harness it is of was retired (the name then ends in `(retired)`);
    /// `Unpinned` for an Interface that has no Pin.
    pub state: MeasurementState,
    /// `ok`, or `failed` when the run did not finish.
    pub outcome: &'static str,
    /// Why it failed, when it did.
    pub failure: Option<String>,
    /// What the run found, in numbers, by the kind of run.
    pub summary: MeasureSummary,
    /// When the run began, in milliseconds since the epoch.
    pub at: u64,
    /// The corrections made of its texts, in the order they were made.
    pub corrections: Vec<CorrectionStatus>,
}

/// What a run found, in numbers. A kind holds only the numbers that belong to it, so a simulation has no mutants
/// counted and a run of examples has no sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureSummary {
    /// A mutation run: how much of the code it looked at, and how many mutants ended each way.
    Mutation {
        /// The places the provider found and the mutants evaluated.
        coverage: Coverage,
        /// How many mutants ended each way.
        counts: Counts,
    },
    /// A simulation.
    Simulation(SimSummary),
    /// A run of examples.
    Examples(ExamplesSummary),
}

/// What the run `m` found, in numbers; the one place that makes a [`MeasureSummary`].
fn summary_of(m: &Measurement) -> MeasureSummary {
    let n = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
    match m.body() {
        MeasureBody::Mutation { .. } => MeasureSummary::Mutation {
            coverage: Coverage {
                sites: m.sites(),
                evaluated: m.evaluated(),
                capped: m.capped(),
            },
            counts: m.counts(),
        },
        MeasureBody::Simulation { outcome, .. } => MeasureSummary::Simulation(sim_summary(outcome)),
        MeasureBody::Examples { outcome, .. } => MeasureSummary::Examples(ExamplesSummary {
            passed: n(outcome.passed()),
            failed: n(outcome.records().len() - outcome.passed()),
        }),
    }
}

fn sim_summary(outcome: &SimOutcome) -> SimSummary {
    let n = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
    SimSummary {
        cases: n(outcome.cases.len()),
        findings: n(outcome.found().len()),
        unreached: n(outcome.unreached().len()),
        abnormal: n(outcome.abnormal().count()),
        reruns: n(outcome.reruns.len()),
        differing: n(outcome.differing().count()),
    }
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// The latest measurements, and the settings now of the subjects they are of that are in the model. What the
    /// runner says is kept as it is: no runner is `Unconfigured`, and any error of a runner that is there is
    /// `Unreadable`, never a subject left out as if its settings did not matter. A subject that is not in the model
    /// has no settings here, and the checks read it as settings that were not read.
    pub(super) async fn measured(&self, p: &Project) -> Result<Measured, AppError> {
        let measurements = self.store.latest_measurements().await?;
        self.with_settings(p, measurements).await
    }

    /// The code the Pin `pin` accepted, as the store kept it; none when the store kept no text for it (a Pin made
    /// before the code was kept) or kept text that is not the code the Pin's digest is of. The one place that holds a
    /// Pin to its code, for every use case that runs or reads the code a Pin accepted.
    pub(super) async fn kept_pin_code(&self, pin: &Pin) -> Result<Option<String>, AppError> {
        Ok(self
            .store
            .pin_code(pin.id())
            .await?
            .filter(|c| Digest::of(c.as_bytes()) == pin.code()))
    }

    /// The latest runs of examples alone, and the settings now of the Harnesses they are of that are in the model.
    /// What a confirmation (a pin, a resolution) reads of the measurements: a mutation run or a simulation is not
    /// asked about, for the checks that judge them run on demand only, and what the runner is asked costs.
    pub(super) async fn examples_measured(&self, p: &Project) -> Result<Measured, AppError> {
        let measurements = self
            .store
            .latest_measurements_of(MeasureKind::Examples)
            .await?;
        self.with_settings(p, measurements).await
    }

    /// `measurements`, with the settings now of the subjects they are of that are in the model. Every subject is
    /// collected first and the runner is asked once for all of them, so that what they share (a program several of
    /// them use) is read once; the answers come back beside their targets.
    async fn with_settings(
        &self,
        p: &Project,
        measurements: Vec<Measurement>,
    ) -> Result<Measured, AppError> {
        let mut settings = BTreeMap::new();
        let mut sim_settings = BTreeMap::new();
        let mut examples_settings = BTreeMap::new();
        let mut asked: Vec<(SettingsTarget, Asked)> = Vec::new();
        for m in &measurements {
            match m.subject() {
                MeasureSubject::Interface(i) => {
                    let Some(context) = p.contexts().find(|c| c.interface(i).is_some()) else {
                        continue;
                    };
                    let interface = self.name_of(i.as_uuid()).await?;
                    // An Interface with no basis is not asked about as if it ran no Harness: nothing says which
                    // tests a run of it would run.
                    let tests = match tests_of(p, i) {
                        Ok(tests) => tests,
                        Err(unreadable) => {
                            settings.insert(i, unreadable);
                            continue;
                        }
                    };
                    let mut harnesses = Vec::with_capacity(tests.len());
                    for (h, _) in tests {
                        harnesses.push(self.name_of(h.id().as_uuid()).await?);
                    }
                    asked.push((
                        SettingsTarget::Mutation {
                            context: context.name().clone(),
                            interface,
                            harnesses,
                        },
                        Asked::Interface(i),
                    ));
                }
                MeasureSubject::Harness(h) => {
                    let Some((context, harness_entity)) = p
                        .contexts()
                        .find_map(|c| c.harness(h).map(|entity| (c, entity)))
                    else {
                        continue;
                    };
                    let harness = self.name_of(h.as_uuid()).await?;
                    let mut builds = Vec::new();
                    for iface in harness_entity.builds() {
                        builds.push(self.name_of(iface.as_uuid()).await?);
                    }
                    // A kind of run asks the runner for the settings of that kind: a Harness whose kind has
                    // changed since is asked as its runs were made.
                    let context = context.name().clone();
                    asked.push(if m.kind() == MeasureKind::Examples {
                        (
                            SettingsTarget::Examples {
                                context,
                                harness,
                                builds,
                            },
                            Asked::ExamplesHarness(h),
                        )
                    } else {
                        (
                            SettingsTarget::Simulation {
                                context,
                                harness,
                                builds,
                            },
                            Asked::SimHarness(h),
                        )
                    });
                }
            }
        }
        let (targets, whose): (Vec<SettingsTarget>, Vec<Asked>) = asked.into_iter().unzip();
        let said = if targets.is_empty() {
            Vec::new()
        } else {
            self.runner.settings_of(&targets).await
        };
        for (k, (target, whose)) in targets.iter().zip(whose).enumerate() {
            // The answer is the one beside the target, wherever the runner put it (most often where it was asked);
            // a runner that left a target out gives no settings for it.
            let answer = said
                .get(k)
                .filter(|(t, _)| t == target)
                .or_else(|| said.iter().find(|(t, _)| t == target));
            let now = match answer {
                Some((_, digest)) => settings_now(digest.clone()),
                None => SettingsNow::Unreadable("the runner gave no settings for it".to_string()),
            };
            match whose {
                Asked::Interface(i) => settings.insert(i, now),
                Asked::SimHarness(h) => sim_settings.insert(h, now),
                Asked::ExamplesHarness(h) => examples_settings.insert(h, now),
            };
        }
        Ok(Measured {
            measurements,
            settings,
            sim_settings,
            examples_settings,
        })
    }

    /// Measures the code the Pin of the Interface `interface` of the Context `context` accepted, and keeps what the
    /// run observed. The code run is the Pin's, as it was pinned, and the Harnesses are the model's as they are now;
    /// the working tree's own copies are not read, and no file of the project is changed. A run that could not
    /// measure (the baseline fails, a command or the provider fails, the time runs out) is kept as a failed
    /// measurement; a configuration that cannot measure at all is an error and keeps nothing.
    pub async fn measure(
        &mut self,
        kind: MeasureKind,
        context: &str,
        interface: &str,
    ) -> Result<MeasureDone, AppError> {
        let instead = match kind {
            MeasureKind::Mutate => None,
            MeasureKind::Simulate => Some("simulate"),
            MeasureKind::Examples => Some("run_examples"),
        };
        if let Some(instead) = instead {
            return Err(AppError::Input(format!(
                "a `{}` measure is of a harness, not of an interface; use `{instead}`",
                kind.word()
            )));
        }
        let p = self.project().await?;
        let (context, interface) = (
            ContextName::try_from(context)?,
            AssetName::try_from(interface)?,
        );
        let ctx = context_named(&p, &context)?;
        let iface = InterfaceId::from_uuid(
            self.asset(ctx, &context, AssetKind::Interface, &interface)
                .await?,
        );
        let pin = self
            .store
            .pin_of(iface)
            .await?
            .ok_or_else(|| AppError::NotPinned {
                context: context.clone(),
                interface: interface.clone(),
            })?;
        let code = self
            .kept_pin_code(&pin)
            .await?
            .ok_or_else(|| AppError::PinCodeNotKept {
                interface: interface.clone(),
            })?;
        let basis = current_basis(&p, iface).ok_or_else(|| AppError::NotPinned {
            context: context.clone(),
            interface: interface.clone(),
        })?;
        let tests = tests_run(&p, &basis);
        let mut harnesses = Vec::with_capacity(tests.len());
        for (h, source) in tests {
            harnesses.push(HarnessText {
                name: self.name_of(h.id().as_uuid()).await?,
                source: source.as_str().to_string(),
            });
        }
        let id = MeasurementId::from_uuid(self.ids.next_uuid());
        let at = self.clock.now();
        let request = MutateRequest {
            run: id.to_string(),
            context,
            interface,
            code,
            harnesses,
        };
        let report = self.runner.mutate(&request).await?;
        // The stamp names the settings the run resolved and ran under, not a digest asked for before it.
        let stamp = MeasureStamp::new(pin.code(), basis, report.settings);
        let measurement = Measurement::new(MeasurementDraft {
            id,
            at,
            body: MeasureBody::Mutation {
                interface: iface,
                stamp,
                sites: report.sites,
                planned: report.planned,
                mutants: report.mutants,
            },
            failure: report.failure,
            evidence: report.evidence,
        })?;
        self.keep_measurement(RecordTrigger::Measure, &measurement)
            .await?;
        self.measured_words(&measurement).await
    }

    /// Simulates the sim Harness `harness` of the Context `context` and keeps what the run observed. The program
    /// is built from the code the Pins of the Interfaces the Harness builds accepted, as they were pinned, and the
    /// script is the model's as it is now; the working tree's own copies are not read, and no file of the project
    /// is changed. The Clauses the script reports are named `<contract>:<label>` and resolved to the Clauses the
    /// sweep cites; the record keeps their ids. A run that could not finish (the program does not build, the script
    /// broke the protocol, the time ran out) is kept as a failed measurement; a Harness that is not a well-formed
    /// sim Harness, an Interface it builds that is not pinned, or a configuration that cannot simulate it is an
    /// error and keeps nothing.
    pub async fn simulate(
        &mut self,
        context: &str,
        harness: &str,
    ) -> Result<MeasureDone, AppError> {
        let p = self.project().await?;
        let (context, harness) = (
            ContextName::try_from(context)?,
            AssetName::try_from(harness)?,
        );
        let ctx = context_named(&p, &context)?;
        let id = HarnessId::from_uuid(
            self.asset(ctx, &context, AssetKind::Harness, &harness)
                .await?,
        );
        let owner = p
            .context(ctx)
            .ok_or_else(|| AppError::Input(format!("the context `{context}` is gone")))?;
        let h = owner
            .harness(id)
            .ok_or_else(|| AppError::Input(format!("the harness `{harness}` is gone")))?;
        let not_sim = || {
            AppError::Input(format!(
                "the harness `{harness}` is of {}; only a sim harness is simulated (put it with kind `sim`)",
                h.kind().word()
            ))
        };
        if h.kind() != HarnessKind::Sim {
            return Err(not_sim());
        }
        let Some(script) = h.source() else {
            return Err(not_sim());
        };
        // A sim Harness kept in a shape the protocol does not run is refused for the rules a put of it breaks.
        let problems: Vec<ProjectError> = owner
            .sim_problems(h)
            .into_iter()
            .map(ProjectError::SimShape)
            .collect();
        if !problems.is_empty() {
            return Err(refused(problems).into());
        }
        let sweep_case = sweep(h).ok_or_else(|| {
            AppError::Input(format!("the sim harness `{harness}` has no `sweep` case"))
        })?;
        let mut clauses = Vec::new();
        for c in cited_in(&p, sweep_case) {
            let contract = self.name_of(c.contract.as_uuid()).await?;
            let name = ClauseName::new(contract, c.label);
            clauses.push(CitedClause { name, id: c.clause });
        }
        let interfaces = self.built_code(&context, h).await?;
        let pins = self.store.pins().await?;
        let basis = basis_of(&p, id, &pins).ok_or_else(|| {
            AppError::Input(format!("the sim harness `{harness}` is not in the project"))
        })?;
        let run_id = MeasurementId::from_uuid(self.ids.next_uuid());
        let at = self.clock.now();
        let request = SimulateRequest {
            run: run_id.to_string(),
            context,
            harness,
            script: script.as_str().to_string(),
            interfaces,
            clauses,
            regress: regress_names(h),
        };
        let report = self.runner.simulate(&request).await?;
        // The stamp names the settings the run resolved and ran under, not a digest asked for before it.
        let stamp = SimStamp::new(basis, report.settings);
        let measurement = Measurement::new(MeasurementDraft {
            id: run_id,
            at,
            body: MeasureBody::Simulation {
                harness: id,
                stamp,
                outcome: SimOutcome {
                    cases: report.cases,
                    reruns: report.reruns,
                },
            },
            failure: report.failure,
            evidence: report.evidence,
        })?;
        self.keep_measurement(RecordTrigger::Simulate, &measurement)
            .await?;
        self.measured_words(&measurement).await
    }

    /// Runs the examples of the Harness of examples `harness` of the Context `context` against the program built
    /// from the code the Pins of the Interfaces it builds accepted, and keeps what the run observed. The examples
    /// are the model's as they are now, one for each Case; the working tree's own copies are not read, and no file
    /// of the project is changed. The runner names each example by its Case's name, and each is resolved to the
    /// Case: the record keeps the Case's id, so a Case that is named otherwise later is still the one. A name the
    /// Harness has no Case of stops the recording, and keeps nothing. A run that could not finish (the program does
    /// not build, the time runs out) is kept as a failed measurement; a Harness that is not of examples, an
    /// Interface it builds that is not pinned, or a configuration that cannot run it is an error and keeps nothing.
    pub async fn run_examples(
        &mut self,
        context: &str,
        harness: &str,
    ) -> Result<MeasureDone, AppError> {
        let p = self.project().await?;
        let (context, harness) = (
            ContextName::try_from(context)?,
            AssetName::try_from(harness)?,
        );
        let ctx = context_named(&p, &context)?;
        let id = HarnessId::from_uuid(
            self.asset(ctx, &context, AssetKind::Harness, &harness)
                .await?,
        );
        let owner = p
            .context(ctx)
            .ok_or_else(|| AppError::Input(format!("the context `{context}` is gone")))?;
        let h = owner
            .harness(id)
            .ok_or_else(|| AppError::Input(format!("the harness `{harness}` is gone")))?;
        if h.kind() != HarnessKind::Examples {
            return Err(AppError::Input(format!(
                "the harness `{harness}` is of {}; only a harness of examples is run as examples (put it with kind `examples`)",
                h.kind().word()
            )));
        }
        let mut examples = Vec::with_capacity(h.cases().len());
        for case in h.cases() {
            let held = case.example().ok_or_else(|| {
                AppError::Input(format!(
                    "the case `{}` of the harness `{harness}` holds no example",
                    case.name()
                ))
            })?;
            let name = ExampleName::new(case.name().as_str()).map_err(|e| {
                AppError::Input(format!(
                    "the case `{}` of the harness `{harness}` is not an example name: {e}",
                    case.name()
                ))
            })?;
            examples.push((name, held.clone()));
        }
        let interfaces = self.built_code(&context, h).await?;
        let pins = self.store.pins().await?;
        let basis = examples::basis_of(&p, id, &pins).ok_or_else(|| {
            AppError::Input(format!(
                "the harness `{harness}` is not a harness of examples in the project"
            ))
        })?;
        let run_id = MeasurementId::from_uuid(self.ids.next_uuid());
        let at = self.clock.now();
        let request = ExamplesRequest {
            run: run_id.to_string(),
            context,
            harness: harness.clone(),
            interfaces,
            examples,
        };
        let report = self.runner.run_examples(&request).await?;
        // Each example the runner reported is a Case of the Harness, or the run is not recorded: a record that left
        // an example out would say less than the run did.
        let mut records = Vec::with_capacity(report.examples.len());
        for (name, verdict) in &report.examples {
            let case = h
                .cases()
                .iter()
                .find(|c| c.name().as_str() == name.as_str())
                .ok_or_else(|| {
                    AppError::Input(format!(
                        "the runner reported the example `{name}`, which is no case of the harness `{harness}`"
                    ))
                })?;
            records.push(ExampleRecord::from_verdict(case.id(), verdict));
        }
        // The stamp names the settings the run resolved and ran under, not a digest asked for before it.
        let stamp = ExamplesStamp::new(basis, report.settings);
        let measurement = Measurement::new(MeasurementDraft {
            id: run_id,
            at,
            body: MeasureBody::Examples {
                harness: id,
                stamp,
                outcome: ExamplesOutcome::new(records)?,
            },
            failure: report.failure,
            evidence: report.evidence,
        })?;
        self.keep_measurement(RecordTrigger::Examples, &measurement)
            .await?;
        self.measured_words(&measurement).await
    }

    /// The code the Pin of each Interface the Harness `h` builds accepted, as it was pinned, by the Interface's name.
    /// An Interface with no Pin, or whose code the store does not keep, is an error.
    async fn built_code(
        &self,
        context: &ContextName,
        h: &Harness,
    ) -> Result<Vec<InterfaceText>, AppError> {
        let mut interfaces = Vec::new();
        for iface in h.builds().iter().copied() {
            let name = self.name_of(iface.as_uuid()).await?;
            let pin = self
                .store
                .pin_of(iface)
                .await?
                .ok_or_else(|| AppError::NotPinned {
                    context: context.clone(),
                    interface: name.clone(),
                })?;
            let code = self
                .kept_pin_code(&pin)
                .await?
                .ok_or_else(|| AppError::PinCodeNotKept {
                    interface: name.clone(),
                })?;
            interfaces.push(InterfaceText { name, code });
        }
        Ok(interfaces)
    }

    /// Keeps `measurement` alone as the record of `trigger`, trying again when another writer got in between.
    async fn keep_measurement(
        &mut self,
        trigger: RecordTrigger,
        measurement: &Measurement,
    ) -> Result<(), AppError> {
        let record = || RecordChanges {
            measurements: vec![measurement.clone()],
            ..RecordChanges::default()
        };
        for _ in 1..COMMIT_TRIES {
            let at: StoreRevision = self.store.revision().await?;
            match self.commit_record(trigger, at, record()).await {
                Err(AppError::Store(StoreError::Conflict)) => continue,
                other => return other,
            }
        }
        let at = self.store.revision().await?;
        self.commit_record(trigger, at, record()).await
    }

    /// `<contract>:<label>` for each Clause the simulation `outcome` names, from the model as it is now. A Clause
    /// the model has not any more is shown by its id.
    async fn clause_names(
        &self,
        outcome: Option<&SimOutcome>,
    ) -> Result<BTreeMap<ClauseId, String>, AppError> {
        let mut wanted = BTreeSet::new();
        for run in outcome.map(|o| o.cases.as_slice()).unwrap_or_default() {
            wanted.extend(run.findings.iter().map(|f| f.clause));
            wanted.extend(run.unreached.iter().map(|u| u.clause));
        }
        let mut names = BTreeMap::new();
        if wanted.is_empty() {
            return Ok(names);
        }
        let p = self.project().await?;
        for id in wanted {
            let Some(contract) = p.contexts().find_map(|c| c.contract_of(id)) else {
                continue;
            };
            let Some(clause) = contract.clause(id) else {
                continue;
            };
            let contract = self.name_of(contract.id().as_uuid()).await?;
            names.insert(id, ClauseName::new(contract, clause.label()).to_string());
        }
        Ok(names)
    }

    /// Each example of the run of examples `m`, in words. A Case is named as the model has it now, so a Case that was
    /// named otherwise when the run was made is shown by its name today; the Case's id when the model has it no more.
    async fn example_lines(&self, m: &Measurement) -> Result<Vec<ExampleLine>, AppError> {
        let Some(outcome) = m.examples() else {
            return Ok(Vec::new());
        };
        let p = self.project().await?;
        let harness = m
            .harness()
            .and_then(|id| p.contexts().find_map(|c| c.harness(id)));
        Ok(outcome
            .records()
            .iter()
            .map(|r| ExampleLine {
                example: harness
                    .and_then(|h| h.case(r.case))
                    .map_or_else(|| r.case.to_string(), |c| c.name().to_string()),
                passed: r.passed(),
                reasons: match &r.result {
                    ExampleResult::Passed => Vec::new(),
                    ExampleResult::Failed(reasons) => {
                        reasons.iter().map(ToString::to_string).collect()
                    }
                },
            })
            .collect())
    }

    async fn measured_words(&self, m: &Measurement) -> Result<MeasureDone, AppError> {
        let digest = |d: Option<Digest>| d.map(|d| d.to_string());
        let sim = m.sim();
        let names = self.clause_names(sim).await?;
        let examples = self.example_lines(m).await?;
        let named = |id: ClauseId| {
            names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| id.as_uuid().to_string())
        };
        Ok(MeasureDone {
            id: m.id().to_string(),
            kind: m.kind().word(),
            stamp: m.stamp().fingerprint().to_string(),
            failure: m.failure().map(ToString::to_string),
            sites: m.sites(),
            evaluated: m.evaluated(),
            capped: m.capped(),
            counts: m.counts(),
            mutants: m
                .mutants()
                .iter()
                .map(|r| MutantLine {
                    key: r.mutant.key().to_string(),
                    site: format!("{}:{}", r.mutant.site().line(), r.mutant.site().column()),
                    operator: r.mutant.operator().to_string(),
                    change: format!("{} -> {}", r.mutant.from(), r.mutant.to()),
                    outcome: r.outcome.word(),
                })
                .collect(),
            cases: sim
                .map(|s| {
                    s.cases
                        .iter()
                        .map(|c| CaseLine {
                            case: c.case.to_string(),
                            exit: c.exit.word(),
                            findings: c
                                .findings
                                .iter()
                                .map(|f| FindingLine {
                                    step: f.step,
                                    clause: named(f.clause),
                                    text: f.text.clone(),
                                })
                                .collect(),
                            unreached: c
                                .unreached
                                .iter()
                                .map(|u| UnreachedLine {
                                    clause: named(u.clause),
                                    text: u.text.clone(),
                                })
                                .collect(),
                            undecided: c.undecided.clone(),
                            schedule: digest(c.schedule),
                            trace: digest(c.trace),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            reruns: sim
                .map(|s| {
                    s.reruns
                        .iter()
                        .map(|r| RerunLine {
                            case: r.case.to_string(),
                            result: r.result.word(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            examples,
            evidence: m
                .evidence()
                .iter()
                .map(|e| EvidenceLine {
                    step: e.step.clone(),
                    command: e.command.clone(),
                    exit_code: e.exit_code,
                    duration_ms: e.duration_ms,
                })
                .collect(),
        })
    }

    /// What a measurement's `standing` says of where it stands: for a stale one, the parts that moved, each with the
    /// name of the asset it is about when it is one. An asset with no name is a store out of step with its model,
    /// which fails the reading and is never left out.
    async fn state_of(&self, standing: Standing) -> Result<MeasurementState, AppError> {
        match standing {
            Standing::Current => Ok(MeasurementState::Current),
            Standing::Stale(moved) => {
                let mut parts = Vec::with_capacity(moved.parts().len());
                for part in moved.parts() {
                    parts.push(self.moved_part(part).await?);
                }
                Ok(MeasurementState::Stale(parts))
            }
        }
    }

    /// The part `part` with the name of the asset it is about, when it is about one. The only way to make a
    /// [`MovedPart`]: a part that names an asset gets the name the store has for it, or the error of not having one.
    async fn moved_part(&self, part: &StampPart) -> Result<MovedPart, AppError> {
        let name = match part.asset() {
            Some(asset) => Some(self.name_of(asset.as_uuid()).await?),
            None => None,
        };
        Ok(MovedPart {
            part: part.clone(),
            name,
        })
    }

    /// Where the latest measurement of each Interface and each sim Harness stands, in the order of the Contexts.
    pub(super) async fn measurement_standings(
        &self,
        p: &Project,
        measured: &Measured,
    ) -> Result<Vec<MeasurementStanding>, AppError> {
        let mut out = Vec::new();
        let pins = self.store.pins().await?;
        for c in p.contexts() {
            for i in c.interfaces() {
                for run in measured
                    .measurements
                    .iter()
                    .filter_map(Measurement::as_mutation)
                    .filter(|run| run.interface() == i.id())
                {
                    let m = run.measurement();
                    let state = match pins.iter().find(|pin| pin.interface() == i.id()) {
                        Some(pin) => {
                            let now = SettingsNow::of(&measured.settings, &i.id());
                            self.state_of(standing(run, p, pin, &now)).await?
                        }
                        None => MeasurementState::Unpinned,
                    };
                    out.push(MeasurementStanding {
                        context: c.name().to_string(),
                        interface: Some(self.name_of(i.id().as_uuid()).await?.to_string()),
                        harness: None,
                        kind: m.kind().word(),
                        id: m.id().to_string(),
                        stamp: m.stamp().fingerprint().to_string(),
                        state,
                        outcome: if m.failure().is_some() {
                            "failed"
                        } else {
                            "ok"
                        },
                        failure: m.failure().map(ToString::to_string),
                        summary: summary_of(m),
                        at: m.at().millis(),
                        corrections: m.corrections().iter().map(correction_status).collect(),
                    });
                }
            }
            for h in c.harnesses() {
                for run in measured
                    .measurements
                    .iter()
                    .filter_map(Measurement::as_simulation)
                    .filter(|run| run.harness() == h.id())
                {
                    let m = run.measurement();
                    let now = SettingsNow::of(&measured.sim_settings, &h.id());
                    let state = self.state_of(sim_standing(run, p, &pins, &now)).await?;
                    out.push(MeasurementStanding {
                        context: c.name().to_string(),
                        interface: None,
                        harness: Some(self.name_of(h.id().as_uuid()).await?.to_string()),
                        kind: m.kind().word(),
                        id: m.id().to_string(),
                        stamp: m.stamp().fingerprint().to_string(),
                        state,
                        outcome: if m.failure().is_some() {
                            "failed"
                        } else {
                            "ok"
                        },
                        failure: m.failure().map(ToString::to_string),
                        summary: summary_of(m),
                        at: m.at().millis(),
                        corrections: m.corrections().iter().map(correction_status).collect(),
                    });
                }
                for run in measured
                    .measurements
                    .iter()
                    .filter_map(Measurement::as_examples)
                    .filter(|run| run.harness() == h.id())
                {
                    let m = run.measurement();
                    let now = SettingsNow::of(&measured.examples_settings, &h.id());
                    let state = self
                        .state_of(examples_standing(run, p, &pins, &now))
                        .await?;
                    out.push(MeasurementStanding {
                        context: c.name().to_string(),
                        interface: None,
                        harness: Some(self.name_of(h.id().as_uuid()).await?.to_string()),
                        kind: m.kind().word(),
                        id: m.id().to_string(),
                        stamp: m.stamp().fingerprint().to_string(),
                        state,
                        outcome: if m.failure().is_some() {
                            "failed"
                        } else {
                            "ok"
                        },
                        failure: m.failure().map(ToString::to_string),
                        summary: summary_of(m),
                        at: m.at().millis(),
                        corrections: m.corrections().iter().map(correction_status).collect(),
                    });
                }
            }
        }
        // What was measured of a thing that is retired since is kept, and named from the record of it. It is not
        // judged: the thing is not in the model.
        for m in &measured.measurements {
            let target = match m.subject() {
                MeasureSubject::Interface(i) => Target::Interface(i),
                MeasureSubject::Harness(h) => Target::Harness(h),
            };
            let Some(r) = p.retired_of(target) else {
                continue;
            };
            let name = Some(format!("{} (retired)", self.retired_name(r, &[]).await?));
            let is_interface = matches!(target, Target::Interface(_));
            out.push(MeasurementStanding {
                context: r.context().to_string(),
                interface: name.clone().filter(|_| is_interface),
                harness: name.filter(|_| !is_interface),
                kind: m.kind().word(),
                id: m.id().to_string(),
                stamp: m.stamp().fingerprint().to_string(),
                state: MeasurementState::Retired,
                outcome: if m.failure().is_some() {
                    "failed"
                } else {
                    "ok"
                },
                failure: m.failure().map(ToString::to_string),
                summary: summary_of(m),
                at: m.at().millis(),
                corrections: m.corrections().iter().map(correction_status).collect(),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::{ProjectId, test_uuid};

    #[test]
    fn an_interface_with_no_basis_has_unreadable_settings_and_is_not_asked_as_if_it_ran_no_tests() {
        // an Interface the Project does not have (so it is not pinned) has no basis
        let p = Project::new(ProjectId::from_uuid(test_uuid(1)));
        let interface = InterfaceId::from_uuid(test_uuid(2));
        let Err(now) = tests_of(&p, interface) else {
            panic!("an Interface with no basis has no tests to name");
        };
        let SettingsNow::Unreadable(why) = &now else {
            panic!("not unreadable: {now:?}");
        };
        assert!(why.contains("not pinned"), "{why}");
        assert_eq!(
            now.why_not_compared().as_deref(),
            Some("the settings cannot be read: the interface is not pinned, so no tests are known")
        );
    }
}
