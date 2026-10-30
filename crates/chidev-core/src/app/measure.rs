//! Measuring: the operations that run a measure and keep what it observed, and the reading of what is kept. A
//! mutation run takes the code a Pin accepted and tests it against changes of itself; a simulation builds the
//! program from the code of the Pins of the Interfaces a sim Harness builds and drives it with the script. A measure takes minutes and
//! makes facts, so it is an operation of its own and never a moment of the checks; the checks (`measure.failed`, `mutation.*`,
//! `sim.*`) only judge the record it leaves.

use std::collections::{BTreeMap, BTreeSet};

use super::commit::{RecordChanges, RecordTrigger};
use super::ports::external::ExternalExecutor;
use super::ports::measure::{
    HarnessText, InterfaceText, MutateRequest, RunnerError, SimulateRequest,
};
use super::ports::{AssetKind, IdSource, Store, StoreRevision};
use super::{App, AppError, CorrectionStatus, context_named, correction_status};
use crate::domain::entity::harness::Harness;
use crate::domain::entity::measurement::{Counts, MeasureBody, Measurement, MeasurementDraft};
use crate::domain::entity::project::{Project, ProjectError, refused};
use crate::domain::repository::StoreError;
use crate::domain::service::measure::{Standing, sim_standing, standing};
use crate::domain::service::pin::current_basis;
use crate::domain::service::sim::{basis_of, cited_in, regress_names, sweep};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{ClauseId, HarnessId, InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    MeasureKind, MeasureStamp, MeasureSubject, SettingsDigest, SettingsNow,
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
}

/// What the settings are now, from what the runner said of them: no runner is a project that has no `[measure]`
/// table, and any other error is a runner that cannot read the settings for this subject.
fn settings_now(said: Result<SettingsDigest, RunnerError>) -> SettingsNow {
    match said {
        Ok(digest) => SettingsNow::Known(digest),
        Err(RunnerError::Unconfigured) => SettingsNow::Unconfigured,
        Err(_) => SettingsNow::Unreadable,
    }
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

/// Where the latest measurement of an Interface or a sim Harness stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementStanding {
    /// The Context's name.
    pub context: String,
    /// The Interface's name (a mutation run).
    pub interface: Option<String>,
    /// The Harness's name (a simulation).
    pub harness: Option<String>,
    /// What was measured.
    pub kind: &'static str,
    /// The measurement's id.
    pub id: String,
    /// The stamp's digest.
    pub stamp: String,
    /// `current` while what it stands on and the settings are as measured; `stale` once any moved; `retired` once
    /// the Interface or Harness it is of was retired (the name then ends in `(retired)`).
    pub state: &'static str,
    /// `ok`, or `failed` when the run did not finish.
    pub outcome: &'static str,
    /// Why it failed, when it did.
    pub failure: Option<String>,
    /// How many mutants ended each way.
    pub counts: Counts,
    /// How much of the code's sites a mutation run evaluated; none for a simulation.
    pub coverage: Option<Coverage>,
    /// What a simulation found; none for a mutation run.
    pub sim: Option<SimSummary>,
    /// When the run began, in milliseconds since the epoch.
    pub at: u64,
    /// The corrections made of its texts, in the order they were made.
    pub corrections: Vec<CorrectionStatus>,
}

/// How much of the code a mutation run looked at; none for a simulation.
fn coverage_of(m: &Measurement) -> Option<Coverage> {
    (m.kind() == MeasureKind::Mutate).then(|| Coverage {
        sites: m.sites(),
        evaluated: m.evaluated(),
        capped: m.capped(),
    })
}

fn summary_of(outcome: &SimOutcome) -> SimSummary {
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
    /// has no settings here, and the checks read it as settings that cannot be read.
    pub(super) async fn measured(&self, p: &Project) -> Result<Measured, AppError> {
        let measurements = self.store.latest_measurements().await?;
        let mut settings = BTreeMap::new();
        let mut sim_settings = BTreeMap::new();
        for m in &measurements {
            match m.subject() {
                MeasureSubject::Interface(i) => {
                    let Some(context) = p.contexts().find(|c| c.interface(i).is_some()) else {
                        continue;
                    };
                    let interface = self.name_of(i.as_uuid()).await?;
                    let mut harnesses = Vec::new();
                    for (h, _) in current_basis(p, i)
                        .map(|basis| tests_run(p, &basis))
                        .unwrap_or_default()
                    {
                        harnesses.push(self.name_of(h.id().as_uuid()).await?);
                    }
                    let said = self
                        .runner
                        .settings(context.name(), &interface, &harnesses)
                        .await;
                    settings.insert(i, settings_now(said));
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
                    let said = self
                        .runner
                        .sim_settings(context.name(), &harness, &builds)
                        .await;
                    sim_settings.insert(h, settings_now(said));
                }
            }
        }
        Ok(Measured {
            measurements,
            settings,
            sim_settings,
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
        if kind != MeasureKind::Mutate {
            return Err(AppError::Input(format!(
                "a `{}` measure is of a harness, not of an interface; use `simulate`",
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
            .store
            .pin_code(pin.id())
            .await?
            .filter(|c| Digest::of(c.as_bytes()) == pin.code())
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
                .store
                .pin_code(pin.id())
                .await?
                .filter(|c| Digest::of(c.as_bytes()) == pin.code())
                .ok_or_else(|| AppError::PinCodeNotKept {
                    interface: name.clone(),
                })?;
            interfaces.push(InterfaceText { name, code });
        }
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

    async fn measured_words(&self, m: &Measurement) -> Result<MeasureDone, AppError> {
        let digest = |d: Option<Digest>| d.map(|d| d.to_string());
        let sim = m.sim();
        let names = self.clause_names(sim).await?;
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
                for m in measured
                    .measurements
                    .iter()
                    .filter(|m| m.interface() == Some(i.id()))
                {
                    let state = match pins.iter().find(|pin| pin.interface() == i.id()) {
                        Some(pin) => {
                            standing(m, p, pin, SettingsNow::of(&measured.settings, &i.id()))
                        }
                        None => Standing::Stale,
                    };
                    out.push(MeasurementStanding {
                        context: c.name().to_string(),
                        interface: Some(self.name_of(i.id().as_uuid()).await?.to_string()),
                        harness: None,
                        kind: m.kind().word(),
                        id: m.id().to_string(),
                        stamp: m.stamp().fingerprint().to_string(),
                        state: state.word(),
                        outcome: if m.failure().is_some() {
                            "failed"
                        } else {
                            "ok"
                        },
                        failure: m.failure().map(ToString::to_string),
                        counts: m.counts(),
                        coverage: coverage_of(m),
                        sim: None,
                        at: m.at().millis(),
                        corrections: m.corrections().iter().map(correction_status).collect(),
                    });
                }
            }
            for h in c.harnesses() {
                for m in measured
                    .measurements
                    .iter()
                    .filter(|m| m.harness() == Some(h.id()))
                {
                    let state = sim_standing(
                        m,
                        p,
                        &pins,
                        SettingsNow::of(&measured.sim_settings, &h.id()),
                    );
                    out.push(MeasurementStanding {
                        context: c.name().to_string(),
                        interface: None,
                        harness: Some(self.name_of(h.id().as_uuid()).await?.to_string()),
                        kind: m.kind().word(),
                        id: m.id().to_string(),
                        stamp: m.stamp().fingerprint().to_string(),
                        state: state.word(),
                        outcome: if m.failure().is_some() {
                            "failed"
                        } else {
                            "ok"
                        },
                        failure: m.failure().map(ToString::to_string),
                        counts: m.counts(),
                        coverage: None,
                        sim: m.sim().map(summary_of),
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
                state: "retired",
                outcome: if m.failure().is_some() {
                    "failed"
                } else {
                    "ok"
                },
                failure: m.failure().map(ToString::to_string),
                counts: m.counts(),
                coverage: coverage_of(m),
                sim: m.sim().map(summary_of),
                at: m.at().millis(),
                corrections: m.corrections().iter().map(correction_status).collect(),
            });
        }
        Ok(out)
    }
}
