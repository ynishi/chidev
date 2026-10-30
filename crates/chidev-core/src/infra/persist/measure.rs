//! Measurements as the store keeps them: a run is one row (its stamp as a JSON document of the revisions it stood
//! on, its failure and its evidence as JSON) and what it ran are item rows: the mutants of a mutation run, the
//! cases of a simulation, the examples of a run of examples. The domain refuses what is not a measurement, so a row
//! that was changed by hand to something impossible is `Corrupt`, never quietly repaired.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::v1::CorrectionDoc;
use crate::app::ports::InvocationField;
use crate::domain::entity::measurement::{
    MeasureBody, Measurement, MeasurementDraft, MeasurementField,
};
use crate::domain::repository::StoreError;
use crate::domain::vo::correction::Correction;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::examples_run::{
    ExampleRecord, ExampleResult, ExamplesBasis, ExamplesOutcome, ExamplesStamp, Got, Reason,
};
use crate::domain::vo::id::{CaseId, ClauseId, ContractId, HarnessId, InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    FailureKind, MeasureFailure, MeasureKind, MeasureStamp, MeasureSubject, Mutant, MutantKey,
    MutantOutcome, MutantResult, Operator, SettingsDigest, Site, StepEvidence,
};
use crate::domain::vo::pin_basis::PinBasis;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::sim::{
    CaseExit, CaseFinding, CaseRun, CaseUnreached, Reproduction, Rerun, SimBasis, SimCase,
    SimOutcome, SimStamp,
};
use crate::domain::vo::timestamp::Timestamp;

fn corrupt(what: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(format!("a measurement: {what}"))
}

fn write<D: Serialize>(doc: &D) -> Result<String, StoreError> {
    serde_json::to_string(doc)
        .map_err(|e| StoreError::Failed(format!("cannot write a measurement: {e}")))
}

fn read<'a, D: Deserialize<'a>>(text: &'a str, what: &str) -> Result<D, StoreError> {
    serde_json::from_str(text).map_err(|e| corrupt(format!("{what}: {e}")))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevDoc {
    id: String,
    revision: u64,
}

/// The stamp of a mutation run, as the run row keeps it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StampDoc {
    code: String,
    interface: u64,
    harnesses: Vec<RevDoc>,
    contracts: Vec<RevDoc>,
    used_interfaces: Vec<RevDoc>,
    settings: String,
}

/// An Interface a simulation stood on: its revision and the digest of the code pinned for it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedDoc {
    id: String,
    revision: u64,
    pin: Option<String>,
}

/// The stamp of a simulation, as the run row keeps it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SimStampDoc {
    harness: u64,
    contracts: Vec<RevDoc>,
    interfaces: Vec<PinnedDoc>,
    settings: String,
}

/// An Interface a run of examples stood on: the digest of the code pinned for it (none when it has no Pin).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuiltDoc {
    id: String,
    pin: Option<String>,
}

/// The stamp of a run of examples, as the run row keeps it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExamplesStampDoc {
    harness: u64,
    builds: Vec<BuiltDoc>,
    settings: String,
}

/// What a run of examples says apart from its examples, for a reader of the row that does not want the items.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExamplesSummaryDoc {
    examples: u32,
    passed: u32,
    failed: u32,
}

/// Where a program's answer differs, as an item row keeps it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GotDoc {
    Line { line: usize, got: Option<String> },
    FinalNewline { line: usize, got_newline: bool },
}

/// One reason an example failed, as an item row keeps it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReasonDoc {
    Unreadable(String),
    Abnormal(String),
    Output(GotDoc),
    Errors(GotDoc),
    Exit(i32),
}

impl From<&Got> for GotDoc {
    fn from(got: &Got) -> GotDoc {
        match got {
            Got::Line { line, got } => GotDoc::Line {
                line: *line,
                got: got.clone(),
            },
            Got::FinalNewline { line, got_newline } => GotDoc::FinalNewline {
                line: *line,
                got_newline: *got_newline,
            },
        }
    }
}

impl From<GotDoc> for Got {
    fn from(doc: GotDoc) -> Got {
        match doc {
            GotDoc::Line { line, got } => Got::Line { line, got },
            GotDoc::FinalNewline { line, got_newline } => Got::FinalNewline { line, got_newline },
        }
    }
}

impl From<&Reason> for ReasonDoc {
    fn from(reason: &Reason) -> ReasonDoc {
        match reason {
            Reason::Unreadable(text) => ReasonDoc::Unreadable(text.clone()),
            Reason::Abnormal(text) => ReasonDoc::Abnormal(text.clone()),
            Reason::Output(got) => ReasonDoc::Output(got.into()),
            Reason::Errors(got) => ReasonDoc::Errors(got.into()),
            Reason::Exit { got } => ReasonDoc::Exit(*got),
        }
    }
}

impl From<ReasonDoc> for Reason {
    fn from(doc: ReasonDoc) -> Reason {
        match doc {
            ReasonDoc::Unreadable(text) => Reason::Unreadable(text),
            ReasonDoc::Abnormal(text) => Reason::Abnormal(text),
            ReasonDoc::Output(got) => Reason::Output(got.into()),
            ReasonDoc::Errors(got) => Reason::Errors(got.into()),
            ReasonDoc::Exit(got) => Reason::Exit { got },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureDoc {
    kind: String,
    message: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceDoc {
    step: String,
    command: String,
    command_digest: Option<String>,
    exit_code: Option<i32>,
    duration_ms: u64,
    stderr: String,
}

/// The counts of a mutation run, for a reader of the row that does not want the items.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryDoc {
    /// How many places the provider found to change.
    sites: u32,
    /// How many mutants it gave to run; fewer than `sites` is a run capped by `max_mutants`.
    planned: u32,
    killed: u32,
    survived: u32,
    stillborn: u32,
    timeout: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RerunDoc {
    case: String,
    result: String,
}

/// What a simulation row says apart from its cases: the cases run a second time.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SimSummaryDoc {
    cases: u32,
    findings: u32,
    reruns: Vec<RerunDoc>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetailDoc {
    from: String,
    to: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FindingDoc {
    step: u32,
    /// The id of the Clause, resolved when the run was recorded.
    clause: String,
    text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnreachedDoc {
    /// The id of the Clause, resolved when the run was recorded.
    clause: String,
    text: String,
}

/// What a case of a simulation found, as its item row keeps it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseDetailDoc {
    findings: Vec<FindingDoc>,
    unreached: Vec<UnreachedDoc>,
    undecided: Vec<String>,
    schedule: Option<String>,
    trace: Option<String>,
}

/// A run, as the columns of `measure_runs` hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunRow {
    pub id: String,
    pub kind: String,
    /// `interface` or `harness`.
    pub subject_kind: String,
    pub subject: String,
    /// The digest of the stamp, for a person to read; the stamp is in `inputs`.
    pub stamp: String,
    pub inputs: String,
    pub started_at: i64,
    pub outcome: String,
    pub failure: Option<String>,
    pub evidence: String,
    pub summary: String,
    /// The corrections made of its texts, as a JSON array; none when there is none.
    pub corrections: Option<String>,
}

/// A mutant, a case or an example, as the columns of `measure_items` hold it. A case has no site and no operator
/// (they are empty), its key is the case, its outcome the way the script ended and its detail what it found. An
/// example has neither either: its key is the id of its Case, its outcome `passed` or `failed` and its detail the
/// reasons it failed (a list, empty for an example that passed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemRow {
    pub key: String,
    pub site: String,
    pub operator: String,
    pub outcome: String,
    pub detail: String,
}

fn revs<K: Copy>(map: &BTreeMap<K, Revision>, id: impl Fn(K) -> Uuid) -> Vec<RevDoc> {
    map.iter()
        .map(|(k, r)| RevDoc {
            id: id(*k).to_string(),
            revision: r.number(),
        })
        .collect()
}

fn unrevs<K: Ord>(
    docs: &[RevDoc],
    make: impl Fn(Uuid) -> K,
) -> Result<BTreeMap<K, Revision>, StoreError> {
    docs.iter()
        .map(|d| {
            let uuid =
                Uuid::parse_str(&d.id).map_err(|_| corrupt(format!("`{}` is not an id", d.id)))?;
            let revision =
                Revision::restore(d.revision).ok_or_else(|| corrupt("a revision of 0"))?;
            Ok((make(uuid), revision))
        })
        .collect()
}

fn digest_of(text: &str) -> Result<Digest, StoreError> {
    Digest::restore(text).ok_or_else(|| corrupt(format!("`{text}` is not a digest")))
}

/// The settings digest a stamp row keeps, as the text of a digest; a value that is not one is corrupt.
fn settings_of(text: &str) -> Result<SettingsDigest, StoreError> {
    Digest::restore(text)
        .map(SettingsDigest::new)
        .ok_or_else(|| corrupt(format!("`{text}` is not a settings digest")))
}

fn clause_of(text: &str) -> Result<ClauseId, StoreError> {
    Uuid::parse_str(text)
        .map(ClauseId::from_uuid)
        .map_err(|_| corrupt(format!("`{text}` is not a clause id")))
}

/// The stamp document and the summary document of a run.
fn stamp_and_summary(m: &Measurement) -> Result<(String, String), StoreError> {
    match m.body() {
        MeasureBody::Mutation {
            stamp,
            sites,
            planned,
            ..
        } => {
            let b = stamp.basis();
            let counts = m.counts();
            Ok((
                write(&StampDoc {
                    code: stamp.code().to_string(),
                    interface: b.interface().number(),
                    harnesses: revs(b.harnesses(), |i: HarnessId| i.as_uuid()),
                    contracts: revs(b.contracts(), |i: ContractId| i.as_uuid()),
                    used_interfaces: revs(b.used_interfaces(), |i: InterfaceId| i.as_uuid()),
                    settings: stamp.settings().to_string(),
                })?,
                write(&SummaryDoc {
                    sites: *sites,
                    planned: *planned,
                    killed: counts.killed,
                    survived: counts.survived,
                    stillborn: counts.stillborn,
                    timeout: counts.timeout,
                })?,
            ))
        }
        MeasureBody::Simulation { stamp, outcome, .. } => {
            let b = stamp.basis();
            Ok((
                write(&SimStampDoc {
                    harness: b.harness.number(),
                    contracts: revs(&b.contracts, |i: ContractId| i.as_uuid()),
                    interfaces: b
                        .interfaces
                        .iter()
                        .map(|(id, (revision, pin))| PinnedDoc {
                            id: id.as_uuid().to_string(),
                            revision: revision.number(),
                            pin: pin.map(|d| d.to_string()),
                        })
                        .collect(),
                    settings: stamp.settings().to_string(),
                })?,
                write(&SimSummaryDoc {
                    cases: u32::try_from(outcome.cases.len()).unwrap_or(u32::MAX),
                    findings: u32::try_from(outcome.found().len()).unwrap_or(u32::MAX),
                    reruns: outcome
                        .reruns
                        .iter()
                        .map(|r| RerunDoc {
                            case: r.case.to_string(),
                            result: r.result.word().to_string(),
                        })
                        .collect(),
                })?,
            ))
        }
        MeasureBody::Examples { stamp, outcome, .. } => {
            let n = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
            let b = stamp.basis();
            Ok((
                write(&ExamplesStampDoc {
                    harness: b.harness.number(),
                    builds: b
                        .builds
                        .iter()
                        .map(|(id, pin)| BuiltDoc {
                            id: id.as_uuid().to_string(),
                            pin: pin.map(|d| d.to_string()),
                        })
                        .collect(),
                    settings: stamp.settings().to_string(),
                })?,
                write(&ExamplesSummaryDoc {
                    examples: n(outcome.records().len()),
                    passed: n(outcome.passed()),
                    failed: n(outcome.records().len() - outcome.passed()),
                })?,
            ))
        }
    }
}

/// The rows a measurement is kept as.
pub(crate) fn encode(m: &Measurement) -> Result<(RunRow, Vec<ItemRow>), StoreError> {
    let (inputs, summary) = stamp_and_summary(m)?;
    let run = RunRow {
        id: m.id().as_uuid().to_string(),
        kind: m.kind().word().to_string(),
        subject_kind: m.subject().word().to_string(),
        subject: m.subject().as_uuid().to_string(),
        stamp: m.stamp().fingerprint().to_string(),
        inputs,
        started_at: i64::try_from(m.at().millis())
            .map_err(|_| StoreError::Failed("a time too large to keep".to_string()))?,
        outcome: if m.failure().is_some() {
            "failed"
        } else {
            "ok"
        }
        .to_string(),
        failure: m
            .failure()
            .map(|f| {
                write(&FailureDoc {
                    kind: f.kind().word().to_string(),
                    message: f.message().to_string(),
                })
            })
            .transpose()?,
        evidence: write(
            &m.evidence()
                .iter()
                .map(|e| EvidenceDoc {
                    step: e.step.clone(),
                    command: e.command.clone(),
                    command_digest: e.command_digest.clone(),
                    exit_code: e.exit_code,
                    duration_ms: e.duration_ms,
                    stderr: e.stderr.clone(),
                })
                .collect::<Vec<_>>(),
        )?,
        summary,
        corrections: corrections_text(m.corrections(), MeasurementFieldDoc::of)?,
    };
    let mut items = m
        .mutants()
        .iter()
        .map(|r| {
            Ok(ItemRow {
                key: r.mutant.key().to_string(),
                site: format!("{}:{}", r.mutant.site().line(), r.mutant.site().column()),
                operator: r.mutant.operator().to_string(),
                outcome: r.outcome.word().to_string(),
                detail: write(&DetailDoc {
                    from: r.mutant.from().to_string(),
                    to: r.mutant.to().to_string(),
                })?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    for c in m.sim().map(|s| s.cases.as_slice()).unwrap_or_default() {
        items.push(ItemRow {
            key: c.case.to_string(),
            site: String::new(),
            operator: String::new(),
            outcome: c.exit.word(),
            detail: write(&CaseDetailDoc {
                findings: c
                    .findings
                    .iter()
                    .map(|f| FindingDoc {
                        step: f.step,
                        clause: f.clause.as_uuid().to_string(),
                        text: f.text.clone(),
                    })
                    .collect(),
                unreached: c
                    .unreached
                    .iter()
                    .map(|u| UnreachedDoc {
                        clause: u.clause.as_uuid().to_string(),
                        text: u.text.clone(),
                    })
                    .collect(),
                undecided: c.undecided.clone(),
                schedule: c.schedule.map(|d| d.to_string()),
                trace: c.trace.map(|d| d.to_string()),
            })?,
        });
    }
    for r in m.examples().map(|o| o.records()).unwrap_or_default() {
        let (outcome, reasons) = match &r.result {
            ExampleResult::Passed => ("passed", Vec::new()),
            ExampleResult::Failed(reasons) => {
                ("failed", reasons.iter().map(ReasonDoc::from).collect())
            }
        };
        items.push(ItemRow {
            key: r.case.as_uuid().to_string(),
            site: String::new(),
            operator: String::new(),
            outcome: outcome.to_string(),
            detail: write::<Vec<ReasonDoc>>(&reasons)?,
        });
    }
    Ok((run, items))
}

/// The examples the item rows of a run of examples are.
fn examples_of(items: &[ItemRow]) -> Result<ExamplesOutcome, StoreError> {
    let records = items
        .iter()
        .map(|i| {
            let case = Uuid::parse_str(&i.key)
                .map(CaseId::from_uuid)
                .map_err(|_| corrupt(format!("`{}` is not a case id", i.key)))?;
            let reasons: Vec<ReasonDoc> = read(&i.detail, "an example's reasons")?;
            let result = match (i.outcome.as_str(), reasons.is_empty()) {
                ("passed", true) => ExampleResult::Passed,
                ("failed", _) => {
                    ExampleResult::Failed(reasons.into_iter().map(Reason::from).collect())
                }
                (other, _) => {
                    return Err(corrupt(format!("an example with outcome `{other}`")));
                }
            };
            Ok(ExampleRecord { case, result })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    ExamplesOutcome::new(records).map_err(corrupt)
}

/// The mutants the item rows of a mutation run are.
fn mutants_of(items: &[ItemRow]) -> Result<Vec<MutantResult>, StoreError> {
    items
        .iter()
        .map(|i| {
            let (line, column) = i
                .site
                .split_once(':')
                .and_then(|(l, c)| Some((l.parse().ok()?, c.parse().ok()?)))
                .ok_or_else(|| corrupt(format!("site `{}`", i.site)))?;
            let detail: DetailDoc = read(&i.detail, "a mutant's change")?;
            Ok(MutantResult {
                mutant: Mutant::new(
                    MutantKey::try_from(i.key.as_str()).map_err(corrupt)?,
                    Site::new(line, column).map_err(corrupt)?,
                    Operator::try_from(i.operator.as_str()).map_err(corrupt)?,
                    detail.from,
                    detail.to,
                )
                .map_err(corrupt)?,
                outcome: MutantOutcome::from_word(&i.outcome)
                    .ok_or_else(|| corrupt(format!("outcome `{}`", i.outcome)))?,
            })
        })
        .collect()
}

/// The cases the item rows of a simulation are, and the reruns its summary lists.
fn sim_of(summary: &SimSummaryDoc, items: &[ItemRow]) -> Result<SimOutcome, StoreError> {
    let case_of = |text: &str| SimCase::try_from(text).map_err(|e| corrupt(format!("a case: {e}")));
    let cases = items
        .iter()
        .map(|i| {
            let detail: CaseDetailDoc = read(&i.detail, "a case's findings")?;
            Ok(CaseRun {
                case: case_of(&i.key)?,
                exit: CaseExit::from_word(&i.outcome)
                    .ok_or_else(|| corrupt(format!("exit `{}`", i.outcome)))?,
                findings: detail
                    .findings
                    .into_iter()
                    .map(|f| {
                        Ok(CaseFinding {
                            step: f.step,
                            clause: clause_of(&f.clause)?,
                            text: f.text,
                        })
                    })
                    .collect::<Result<_, StoreError>>()?,
                unreached: detail
                    .unreached
                    .into_iter()
                    .map(|u| {
                        Ok(CaseUnreached {
                            clause: clause_of(&u.clause)?,
                            text: u.text,
                        })
                    })
                    .collect::<Result<_, StoreError>>()?,
                undecided: detail.undecided,
                schedule: detail.schedule.as_deref().map(digest_of).transpose()?,
                trace: detail.trace.as_deref().map(digest_of).transpose()?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let reruns = summary
        .reruns
        .iter()
        .map(|r| {
            Ok(Rerun {
                case: case_of(&r.case)?,
                result: Reproduction::from_word(&r.result)
                    .ok_or_else(|| corrupt(format!("rerun result `{}`", r.result)))?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(SimOutcome { cases, reruns })
}

/// The measurement the rows are, if the domain agrees they are one.
pub(crate) fn decode(run: &RunRow, items: &[ItemRow]) -> Result<Measurement, StoreError> {
    let kind =
        MeasureKind::from_word(&run.kind).ok_or_else(|| corrupt(format!("kind `{}`", run.kind)))?;
    let uuid =
        |text: &str| Uuid::parse_str(text).map_err(|_| corrupt(format!("`{text}` is not an id")));
    let subject = match (kind, run.subject_kind.as_str()) {
        (MeasureKind::Mutate, "interface") => {
            MeasureSubject::Interface(InterfaceId::from_uuid(uuid(&run.subject)?))
        }
        (MeasureKind::Simulate | MeasureKind::Examples, "harness") => {
            MeasureSubject::Harness(HarnessId::from_uuid(uuid(&run.subject)?))
        }
        (_, other) => {
            return Err(corrupt(format!("a `{}` run of a `{other}`", kind.word())));
        }
    };
    let body = match (kind, subject) {
        (MeasureKind::Mutate, MeasureSubject::Interface(interface)) => {
            let stamp: StampDoc = read(&run.inputs, "its stamp")?;
            let basis = PinBasis::new(
                Revision::restore(stamp.interface).ok_or_else(|| corrupt("a revision of 0"))?,
                unrevs(&stamp.harnesses, HarnessId::from_uuid)?,
                unrevs(&stamp.contracts, ContractId::from_uuid)?,
                unrevs(&stamp.used_interfaces, InterfaceId::from_uuid)?,
            );
            let code = Digest::restore(&stamp.code).ok_or_else(|| corrupt("a code digest"))?;
            let stamp = MeasureStamp::new(code, basis, settings_of(&stamp.settings)?);
            let summary: SummaryDoc = read(&run.summary, "its summary")?;
            MeasureBody::Mutation {
                interface,
                stamp,
                sites: summary.sites,
                planned: summary.planned,
                mutants: mutants_of(items)?,
            }
        }
        (MeasureKind::Simulate, MeasureSubject::Harness(harness)) => {
            let stamp: SimStampDoc = read(&run.inputs, "its stamp")?;
            let interfaces = stamp
                .interfaces
                .iter()
                .map(|p| {
                    Ok((
                        InterfaceId::from_uuid(uuid(&p.id)?),
                        (
                            Revision::restore(p.revision)
                                .ok_or_else(|| corrupt("a revision of 0"))?,
                            p.pin.as_deref().map(digest_of).transpose()?,
                        ),
                    ))
                })
                .collect::<Result<_, StoreError>>()?;
            let basis = SimBasis {
                harness: Revision::restore(stamp.harness)
                    .ok_or_else(|| corrupt("a revision of 0"))?,
                contracts: unrevs(&stamp.contracts, ContractId::from_uuid)?,
                interfaces,
            };
            let stamp = SimStamp::new(basis, settings_of(&stamp.settings)?);
            let summary: SimSummaryDoc = read(&run.summary, "its summary")?;
            MeasureBody::Simulation {
                harness,
                stamp,
                outcome: sim_of(&summary, items)?,
            }
        }
        (MeasureKind::Examples, MeasureSubject::Harness(harness)) => {
            let stamp: ExamplesStampDoc = read(&run.inputs, "its stamp")?;
            let builds = stamp
                .builds
                .iter()
                .map(|b| {
                    Ok((
                        InterfaceId::from_uuid(uuid(&b.id)?),
                        b.pin.as_deref().map(digest_of).transpose()?,
                    ))
                })
                .collect::<Result<_, StoreError>>()?;
            let basis = ExamplesBasis {
                harness: Revision::restore(stamp.harness)
                    .ok_or_else(|| corrupt("a revision of 0"))?,
                builds,
            };
            let stamp = ExamplesStamp::new(basis, settings_of(&stamp.settings)?);
            // the summary is for a reader of the row; the items are what the run is
            let _: ExamplesSummaryDoc = read(&run.summary, "its summary")?;
            MeasureBody::Examples {
                harness,
                stamp,
                outcome: examples_of(items)?,
            }
        }
        (kind, subject) => {
            return Err(corrupt(format!(
                "a `{}` run of a `{}`",
                kind.word(),
                subject.word()
            )));
        }
    };
    let failure = run
        .failure
        .as_deref()
        .map(|text| {
            let doc: FailureDoc = read(text, "its failure")?;
            let kind = FailureKind::from_word(&doc.kind)
                .ok_or_else(|| corrupt(format!("failure kind `{}`", doc.kind)))?;
            MeasureFailure::new(kind, doc.message).map_err(corrupt)
        })
        .transpose()?;
    if failure.is_some() != (run.outcome == "failed") {
        return Err(corrupt(format!(
            "outcome `{}` disagrees with its failure",
            run.outcome
        )));
    }
    let evidence = read::<Vec<EvidenceDoc>>(&run.evidence, "its evidence")?
        .into_iter()
        .map(|e| StepEvidence {
            step: e.step,
            command: e.command,
            command_digest: e.command_digest,
            exit_code: e.exit_code,
            duration_ms: e.duration_ms,
            stderr: e.stderr,
        })
        .collect();
    Measurement::new(MeasurementDraft {
        id: MeasurementId::from_uuid(uuid(&run.id)?),
        at: Timestamp::from_millis(
            u64::try_from(run.started_at).map_err(|_| corrupt("a negative time"))?,
        ),
        body,
        failure,
        evidence,
    })
    .map_err(corrupt)
    .and_then(|m| {
        let corrections = corrections_read(run.corrections.as_deref(), MeasurementFieldDoc::field)?;
        Ok(m.with_corrections(corrections))
    })
}

/// A field of a measurement, as a correction keeps it: `{"step_stderr":1}`, `"failure"`,
/// `{"case_finding":[0,1]}`, `{"case_exit":0}`, `{"example_reason":[0,1]}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MeasurementFieldDoc {
    StepCommand(usize),
    StepStderr(usize),
    Failure,
    MutantFrom(usize),
    MutantTo(usize),
    CaseFinding(usize, usize),
    CaseUnreached(usize, usize),
    CaseUndecided(usize, usize),
    CaseExit(usize),
    ExampleReason(usize, usize),
}

impl MeasurementFieldDoc {
    /// The document of a field. A function of a reference, not a `From`, so that it can be handed on as one
    /// function for any lifetime.
    fn of(f: &MeasurementField) -> MeasurementFieldDoc {
        match *f {
            MeasurementField::StepCommand { step } => MeasurementFieldDoc::StepCommand(step),
            MeasurementField::StepStderr { step } => MeasurementFieldDoc::StepStderr(step),
            MeasurementField::Failure => MeasurementFieldDoc::Failure,
            MeasurementField::MutantFrom { mutant } => MeasurementFieldDoc::MutantFrom(mutant),
            MeasurementField::MutantTo { mutant } => MeasurementFieldDoc::MutantTo(mutant),
            MeasurementField::CaseFinding { case, finding } => {
                MeasurementFieldDoc::CaseFinding(case, finding)
            }
            MeasurementField::CaseUnreached { case, unreached } => {
                MeasurementFieldDoc::CaseUnreached(case, unreached)
            }
            MeasurementField::CaseUndecided { case, undecided } => {
                MeasurementFieldDoc::CaseUndecided(case, undecided)
            }
            MeasurementField::CaseExit { case } => MeasurementFieldDoc::CaseExit(case),
            MeasurementField::ExampleReason { example, reason } => {
                MeasurementFieldDoc::ExampleReason(example, reason)
            }
        }
    }

    /// The field the document names.
    fn field(&self) -> Result<MeasurementField, String> {
        Ok(match *self {
            MeasurementFieldDoc::StepCommand(step) => MeasurementField::StepCommand { step },
            MeasurementFieldDoc::StepStderr(step) => MeasurementField::StepStderr { step },
            MeasurementFieldDoc::Failure => MeasurementField::Failure,
            MeasurementFieldDoc::MutantFrom(mutant) => MeasurementField::MutantFrom { mutant },
            MeasurementFieldDoc::MutantTo(mutant) => MeasurementField::MutantTo { mutant },
            MeasurementFieldDoc::CaseFinding(case, finding) => {
                MeasurementField::CaseFinding { case, finding }
            }
            MeasurementFieldDoc::CaseUnreached(case, unreached) => {
                MeasurementField::CaseUnreached { case, unreached }
            }
            MeasurementFieldDoc::CaseUndecided(case, undecided) => {
                MeasurementField::CaseUndecided { case, undecided }
            }
            MeasurementFieldDoc::CaseExit(case) => MeasurementField::CaseExit { case },
            MeasurementFieldDoc::ExampleReason(example, reason) => {
                MeasurementField::ExampleReason { example, reason }
            }
        })
    }
}

/// A field of the record of a check's run, as a correction keeps it: `"stderr"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InvocationFieldDoc {
    Command,
    Stderr,
    Judge,
}

impl InvocationFieldDoc {
    /// The document of a field. A function of a reference, not a `From`, so that it can be handed on as one
    /// function for any lifetime.
    fn of(f: &InvocationField) -> InvocationFieldDoc {
        match f {
            InvocationField::Command => InvocationFieldDoc::Command,
            InvocationField::Stderr => InvocationFieldDoc::Stderr,
            InvocationField::Judge => InvocationFieldDoc::Judge,
        }
    }

    /// The field the document names.
    fn field(&self) -> Result<InvocationField, String> {
        Ok(match self {
            InvocationFieldDoc::Command => InvocationField::Command,
            InvocationFieldDoc::Stderr => InvocationField::Stderr,
            InvocationFieldDoc::Judge => InvocationField::Judge,
        })
    }
}

/// The corrections as a column keeps them: a JSON array of documents, and none (NULL) when there is no correction.
fn corrections_text<D, F: Serialize>(
    corrections: &[Correction<D>],
    field: impl Fn(&D) -> F + Copy,
) -> Result<Option<String>, StoreError> {
    if corrections.is_empty() {
        return Ok(None);
    }
    let docs: Vec<CorrectionDoc<F>> = corrections
        .iter()
        .map(|c| CorrectionDoc::of(c, field))
        .collect();
    write(&docs).map(Some)
}

/// The corrections a column keeps; none for NULL or an empty array.
fn corrections_read<D: PartialEq + std::fmt::Display, F: serde::de::DeserializeOwned>(
    text: Option<&str>,
    field: impl Fn(&F) -> Result<D, String> + Copy,
) -> Result<Vec<Correction<D>>, StoreError> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let docs: Vec<CorrectionDoc<F>> = read(text, "its corrections")?;
    docs.iter()
        .map(|d| d.read(field).map_err(corrupt))
        .collect()
}

/// The corrections of a row of the record of a check's run, as its column keeps them.
pub(crate) fn encode_invocation_corrections(
    corrections: &[Correction<InvocationField>],
) -> Result<Option<String>, StoreError> {
    corrections_text(corrections, InvocationFieldDoc::of)
}

/// The corrections a row of the record of a check's run keeps in its column.
pub(crate) fn decode_invocation_corrections(
    text: Option<&str>,
) -> Result<Vec<Correction<InvocationField>>, StoreError> {
    corrections_read(text, InvocationFieldDoc::field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entity::measurement::fixture::sim_draft;
    use crate::domain::vo::id::test_uuid;

    fn sim_measurement() -> Measurement {
        let case = |n: u32, clause: u128| CaseRun {
            case: SimCase::Seed(n),
            exit: CaseExit::Normal,
            findings: vec![CaseFinding {
                step: 3,
                clause: ClauseId::from_uuid(test_uuid(clause)),
                text: "capacity 0 accepted: \"x\"".into(),
            }],
            unreached: vec![CaseUnreached {
                clause: ClauseId::from_uuid(test_uuid(9)),
                text: "needs a full queue".into(),
            }],
            undecided: vec!["more than 1000 outcomes".into()],
            schedule: Some(Digest::of(b"s")),
            trace: Some(Digest::of(b"t")),
        };
        let basis = SimBasis {
            harness: Revision::first().next(),
            contracts: BTreeMap::from([(ContractId::from_uuid(test_uuid(3)), Revision::first())]),
            interfaces: BTreeMap::from([
                (
                    InterfaceId::from_uuid(test_uuid(4)),
                    (Revision::first(), Some(Digest::of(b"code"))),
                ),
                (
                    InterfaceId::from_uuid(test_uuid(5)),
                    (Revision::first(), None),
                ),
            ]),
        };
        Measurement::new(sim_draft(
            7,
            HarnessId::from_uuid(test_uuid(6)),
            basis,
            SimOutcome {
                cases: vec![
                    case(0, 4),
                    CaseRun {
                        exit: CaseExit::Script(2),
                        schedule: None,
                        trace: None,
                        findings: Vec::new(),
                        ..case(1, 4)
                    },
                ],
                reruns: vec![Rerun {
                    case: SimCase::Seed(0),
                    result: Reproduction::TraceDiffers,
                }],
            },
        ))
        .unwrap()
    }

    #[test]
    fn a_simulation_survives_the_rows_it_is_kept_as() {
        let m = sim_measurement();
        let (run, items) = encode(&m).unwrap();
        assert_eq!(run.kind, "simulate");
        assert_eq!(run.subject_kind, "harness");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].outcome, "script:2");
        assert_eq!(decode(&run, &items).unwrap(), m);
    }

    #[test]
    fn a_case_that_could_not_start_survives_the_rows_with_its_reason() {
        let mut m = sim_measurement();
        let mut draft = sim_draft(
            8,
            HarnessId::from_uuid(test_uuid(6)),
            m.stamp().sim().expect("a sim stamp").basis().clone(),
            SimOutcome {
                cases: vec![CaseRun {
                    case: SimCase::Seed(0),
                    exit: CaseExit::CannotStart("python3: No such file or directory".into()),
                    findings: Vec::new(),
                    unreached: Vec::new(),
                    undecided: Vec::new(),
                    schedule: None,
                    trace: None,
                }],
                reruns: Vec::new(),
            },
        );
        draft.failure = Some(MeasureFailure::of(FailureKind::NoStart, "none started"));
        m = Measurement::new(draft).unwrap();
        let (run, items) = encode(&m).unwrap();
        assert_eq!(run.outcome, "failed");
        assert_eq!(
            items[0].outcome,
            "cannot_start:python3: No such file or directory"
        );
        assert_eq!(decode(&run, &items).unwrap(), m);
    }

    #[test]
    fn a_mutation_run_keeps_the_sites_it_found_and_the_mutants_it_was_given() {
        use crate::domain::entity::measurement::fixture::{draft, empty_basis, result, set_counts};
        let mut d = draft(
            9,
            InterfaceId::from_uuid(test_uuid(100)),
            Digest::of(b"code"),
            empty_basis(),
            vec![
                result("a", 1, MutantOutcome::Killed),
                result("b", 2, MutantOutcome::Survived),
            ],
        );
        set_counts(&mut d, 41, 2);
        let m = Measurement::new(d).unwrap();
        assert!(m.capped());
        let (run, items) = encode(&m).unwrap();
        assert!(run.summary.contains("\"sites\":41") && run.summary.contains("\"planned\":2"));
        let back = decode(&run, &items).unwrap();
        assert_eq!(back, m);
        assert_eq!((back.sites(), back.planned(), back.evaluated()), (41, 2, 2));
        // a row whose summary says more mutants were given than items exist, with no failure, is not a run
        let mut short = run.clone();
        short.summary = short.summary.replace("\"planned\":2", "\"planned\":3");
        assert!(decode(&short, &items).is_err());
        // and a row from before the field existed is not read quietly
        let mut old = run.clone();
        old.summary = old.summary.replace("\"planned\":2,", "");
        assert!(decode(&old, &items).is_err());
    }

    fn correction<F: PartialEq + std::fmt::Display>(fields: Vec<F>) -> Correction<F> {
        Correction::new(
            fields,
            crate::domain::vo::prose::Prose::try_from("a token was pasted").unwrap(),
            crate::domain::vo::answer::DecisionMaker::person("carol").unwrap(),
            Timestamp::from_millis(50),
        )
        .unwrap()
    }

    #[test]
    fn a_corrected_measurement_survives_its_rows_and_they_hold_no_text() {
        use crate::domain::vo::correction::{Correctable, SoughtText};

        let m = sim_measurement();
        // a run never corrected has no corrections in its row
        let (run, _) = encode(&m).unwrap();
        assert_eq!(run.corrections, None);

        let mut corrected = m.clone();
        corrected
            .correct(
                &SoughtText::try_from("capacity 0").unwrap(),
                correction(vec![
                    MeasurementField::CaseFinding {
                        case: 0,
                        finding: 0,
                    },
                    MeasurementField::CaseFinding {
                        case: 1,
                        finding: 0,
                    },
                ]),
                &|_, _| false,
            )
            .unwrap_err();
        // the second case has no finding, so that correction is refused whole; naming the first case alone is not
        corrected
            .correct(
                &SoughtText::try_from("capacity 0").unwrap(),
                correction(vec![MeasurementField::CaseFinding {
                    case: 0,
                    finding: 0,
                }]),
                &|_, _| false,
            )
            .unwrap();
        let (run, items) = encode(&corrected).unwrap();
        assert_eq!(
            run.corrections.as_deref(),
            Some(
                "[{\"fields\":[{\"case_finding\":[0,0]}],\"reason\":\"a token was pasted\",\"by\":{\"person\":\"carol\"},\"at\":50}]"
            )
        );
        assert_eq!(decode(&run, &items).unwrap(), corrected);
        assert!(!items[0].detail.contains("capacity 0"));
        assert!(items[0].detail.contains("[redacted] accepted"));
        // the run keeps what it stood on
        let (before, _) = encode(&m).unwrap();
        assert_eq!(
            (&run.id, &run.stamp, &run.inputs, run.started_at),
            (&before.id, &before.stamp, &before.inputs, before.started_at)
        );
        // a column that is not a list of corrections is corrupt
        let mut odd = run.clone();
        odd.corrections = Some("{}".to_string());
        assert!(decode(&odd, &items).is_err());
        odd.corrections = Some(
            "[{\"fields\":[],\"reason\":\"x\",\"by\":{\"person\":\"a\"},\"at\":1}]".to_string(),
        );
        assert!(decode(&odd, &items).is_err());
    }

    #[test]
    fn a_corrected_exit_of_a_case_that_could_not_start_is_the_outcome_of_its_item() {
        use crate::domain::vo::correction::{Correctable, SoughtText};

        let mut m = sim_measurement();
        let mut draft = sim_draft(
            8,
            HarnessId::from_uuid(test_uuid(6)),
            m.stamp().sim().expect("a sim stamp").basis().clone(),
            SimOutcome {
                cases: vec![CaseRun {
                    case: SimCase::Seed(0),
                    exit: CaseExit::CannotStart("python3 tok-SECRET-123: No such file".into()),
                    findings: Vec::new(),
                    unreached: Vec::new(),
                    undecided: Vec::new(),
                    schedule: None,
                    trace: None,
                }],
                reruns: Vec::new(),
            },
        );
        draft.failure = Some(MeasureFailure::of(FailureKind::NoStart, "none started"));
        m = Measurement::new(draft).unwrap();
        m.correct(
            &SoughtText::try_from("tok-SECRET-123").unwrap(),
            correction(vec![MeasurementField::CaseExit { case: 0 }]),
            &|_, _| false,
        )
        .unwrap();
        let (run, items) = encode(&m).unwrap();
        assert_eq!(
            items[0].outcome,
            "cannot_start:python3 [redacted]: No such file"
        );
        assert!(
            run.corrections
                .as_deref()
                .is_some_and(|c| c.contains("{\"case_exit\":0}") && !c.contains("SECRET"))
        );
        assert_eq!(decode(&run, &items).unwrap(), m);
    }

    fn examples_measurement() -> Measurement {
        use crate::domain::entity::measurement::fixture::examples_draft;
        let case = |n: u128| CaseId::from_uuid(test_uuid(n));
        let line = |got: Option<&str>| Got::Line {
            line: 2,
            got: got.map(str::to_string),
        };
        let records = vec![
            ExampleRecord {
                case: case(1),
                result: ExampleResult::Passed,
            },
            ExampleRecord {
                case: case(2),
                result: ExampleResult::Failed(vec![
                    Reason::Output(line(Some("tok-SECRET-123 \"x\""))),
                    Reason::Output(line(None)),
                    Reason::Output(Got::FinalNewline {
                        line: 3,
                        got_newline: false,
                    }),
                    Reason::Exit { got: -1 },
                    Reason::Errors(line(Some("oops"))),
                ]),
            },
            ExampleRecord {
                case: case(3),
                result: ExampleResult::Failed(vec![
                    Reason::Unreadable("missing a3.args".to_string()),
                    Reason::Abnormal("did not end within 10 s".to_string()),
                ]),
            },
        ];
        let basis = ExamplesBasis {
            harness: Revision::first().next(),
            builds: BTreeMap::from([
                (
                    InterfaceId::from_uuid(test_uuid(4)),
                    Some(Digest::of(b"code")),
                ),
                (InterfaceId::from_uuid(test_uuid(5)), None),
            ]),
        };
        Measurement::new(examples_draft(
            7,
            HarnessId::from_uuid(test_uuid(6)),
            basis,
            ExamplesOutcome::new(records).unwrap(),
        ))
        .unwrap()
    }

    #[test]
    fn a_run_of_examples_survives_the_rows_it_is_kept_as() {
        let m = examples_measurement();
        let (run, items) = encode(&m).unwrap();
        assert_eq!(run.kind, "examples");
        assert_eq!(run.subject_kind, "harness");
        assert_eq!(
            run.summary,
            "{\"examples\":3,\"passed\":1,\"failed\":2}".to_string()
        );
        // one item for each example: the Case's id, no site and no operator, passed or failed, and the reasons
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0].key,
            CaseId::from_uuid(test_uuid(1)).as_uuid().to_string()
        );
        assert_eq!(
            (items[0].outcome.as_str(), items[0].detail.as_str()),
            ("passed", "[]")
        );
        assert_eq!(items[1].outcome, "failed");
        assert!(
            items
                .iter()
                .all(|i| i.site.is_empty() && i.operator.is_empty())
        );
        assert_eq!(decode(&run, &items).unwrap(), m);
        // and a failed run keeps the examples it got through
        let mut draft = crate::domain::entity::measurement::fixture::examples_draft(
            8,
            HarnessId::from_uuid(test_uuid(6)),
            m.stamp()
                .examples()
                .expect("an examples stamp")
                .basis()
                .clone(),
            ExamplesOutcome::default(),
        );
        draft.failure = Some(MeasureFailure::of(FailureKind::NoBuild, "does not build"));
        let failed = Measurement::new(draft).unwrap();
        let (run, items) = encode(&failed).unwrap();
        assert_eq!(run.outcome, "failed");
        assert!(items.is_empty());
        assert_eq!(decode(&run, &items).unwrap(), failed);
    }

    #[test]
    fn a_corrected_reason_of_an_example_is_the_detail_of_its_item_and_the_row_holds_no_text() {
        use crate::domain::vo::correction::{Correctable, SoughtText};

        let m = examples_measurement();
        let (before, _) = encode(&m).unwrap();
        assert_eq!(before.corrections, None);
        let mut corrected = m.clone();
        corrected
            .correct(
                &SoughtText::try_from("tok-SECRET-123").unwrap(),
                correction(vec![MeasurementField::ExampleReason {
                    example: 1,
                    reason: 0,
                }]),
                &|_, _| false,
            )
            .unwrap();
        let (run, items) = encode(&corrected).unwrap();
        assert!(
            run.corrections
                .as_deref()
                .is_some_and(|c| c.contains("{\"example_reason\":[1,0]}") && !c.contains("SECRET"))
        );
        assert!(!items[1].detail.contains("SECRET"));
        assert!(items[1].detail.contains("[redacted]"));
        assert_eq!(decode(&run, &items).unwrap(), corrected);
        // the run keeps what it stood on and the others of its items
        assert_eq!(
            (&run.id, &run.stamp, &run.inputs, run.started_at),
            (&before.id, &before.stamp, &before.inputs, before.started_at)
        );
        let (_, original) = encode(&m).unwrap();
        assert_eq!(items[0], original[0]);
        assert_eq!(items[2], original[2]);
    }

    #[test]
    fn rows_that_cannot_be_a_run_of_examples_are_corrupt() {
        let m = examples_measurement();
        let (run, items) = encode(&m).unwrap();
        let bad_item = |f: fn(&mut ItemRow)| {
            let mut items = items.clone();
            f(&mut items[1]);
            decode(&run, &items)
        };
        assert!(bad_item(|i| i.key = "a1".into()).is_err());
        assert!(bad_item(|i| i.outcome = "lost".into()).is_err());
        assert!(bad_item(|i| i.detail = "{".into()).is_err());
        assert!(bad_item(|i| i.detail = "[{\"unknown\":1}]".into()).is_err());
        // a passed example has no reason
        assert!(bad_item(|i| i.outcome = "passed".into()).is_err());
        // a Case twice
        let mut twice = items.clone();
        twice[2].key = twice[0].key.clone();
        assert!(decode(&run, &twice).is_err());
        // a run of examples of an Interface, a revision of 0, a digest that is none
        let kind = RunRow {
            subject_kind: "interface".to_string(),
            ..run.clone()
        };
        assert!(decode(&kind, &items).is_err());
        let mut stamp = run.clone();
        stamp.inputs = stamp.inputs.replacen("\"harness\":2", "\"harness\":0", 1);
        assert_ne!(stamp.inputs, run.inputs);
        assert!(decode(&stamp, &items).is_err());
        let mut pin = run.clone();
        pin.inputs = pin
            .inputs
            .replacen(&Digest::of(b"code").to_string(), "nothex", 1);
        assert!(decode(&pin, &items).is_err());
    }

    #[test]
    fn the_corrections_of_a_check_run_come_back_from_their_column() {
        assert_eq!(encode_invocation_corrections(&[]).unwrap(), None);
        assert_eq!(decode_invocation_corrections(None).unwrap(), Vec::new());
        let made = vec![
            correction(vec![InvocationField::Stderr, InvocationField::Command]),
            correction(vec![InvocationField::Judge]),
        ];
        let text = encode_invocation_corrections(&made).unwrap().unwrap();
        assert!(
            text.contains("\"stderr\"") && text.contains("\"judge\""),
            "{text}"
        );
        assert_eq!(decode_invocation_corrections(Some(&text)).unwrap(), made);
        assert!(decode_invocation_corrections(Some("nonsense")).is_err());
    }

    #[test]
    fn rows_that_cannot_be_a_simulation_are_corrupt() {
        let m = sim_measurement();
        let (run, items) = encode(&m).unwrap();
        let kind = |text: &str| RunRow {
            subject_kind: text.to_string(),
            ..run.clone()
        };
        assert!(decode(&kind("interface"), &items).is_err());
        let bad_case = |f: fn(&mut ItemRow)| {
            let mut items = items.clone();
            f(&mut items[0]);
            decode(&run, &items)
        };
        assert!(bad_case(|i| i.key = "seed-03".into()).is_err());
        assert!(bad_case(|i| i.outcome = "lost".into()).is_err());
        assert!(bad_case(|i| i.detail = "{".into()).is_err());
        // a clause is kept by its id: a label is not one
        let id = ClauseId::from_uuid(test_uuid(4)).as_uuid().to_string();
        let mut labelled = items.clone();
        labelled[0].detail = labelled[0].detail.replace(&id, "C4");
        assert_ne!(labelled[0].detail, items[0].detail);
        assert!(decode(&run, &labelled).is_err());
        // a revision of 0 is no revision, though the settings are a digest
        let mut stamp = run.clone();
        stamp.inputs = format!(
            "{{\"harness\":0,\"contracts\":[],\"interfaces\":[],\"settings\":\"{}\"}}",
            Digest::of(b"settings")
        );
        assert!(decode(&stamp, &items).is_err());
        let mut summary = run.clone();
        summary.summary = summary.summary.replace("trace-differs", "other");
        assert!(decode(&summary, &items).is_err());
    }

    #[test]
    fn the_settings_digest_of_a_stamp_survives_the_rows_and_a_value_that_is_not_one_is_corrupt() {
        use crate::domain::entity::measurement::fixture::{draft, empty_basis, settings};
        let shown = settings().to_string();
        let mutation = Measurement::new(draft(
            9,
            InterfaceId::from_uuid(test_uuid(100)),
            Digest::of(b"code"),
            empty_basis(),
            Vec::new(),
        ))
        .unwrap();
        let sim = sim_measurement();
        for m in [mutation, sim] {
            let (run, items) = encode(&m).unwrap();
            // it is kept as the text of a digest
            assert!(run.inputs.contains(&format!("\"settings\":\"{shown}\"")));
            assert_eq!(decode(&run, &items).unwrap(), m);
            // and nothing else is read as one: a word, a digest in upper case, one that is cut short, a blank
            for bad in ["s", &shown.to_uppercase(), &shown[1..], ""] {
                let mut odd = run.clone();
                odd.inputs = odd.inputs.replace(&shown, bad);
                assert_ne!(odd.inputs, run.inputs);
                let error = decode(&odd, &items).unwrap_err();
                assert!(
                    matches!(&error, StoreError::Corrupt(m) if m.contains("not a settings digest")),
                    "{bad:?}: {error:?}"
                );
            }
        }
    }

    #[test]
    fn a_run_that_failed_because_its_settings_moved_survives_the_rows_by_its_word() {
        assert_eq!(FailureKind::SettingsMoved.word(), "settings_moved");
        use crate::domain::entity::measurement::fixture::{draft, empty_basis};
        let mut d = draft(
            9,
            InterfaceId::from_uuid(test_uuid(100)),
            Digest::of(b"code"),
            empty_basis(),
            Vec::new(),
        );
        d.failure = Some(MeasureFailure::of(
            FailureKind::SettingsMoved,
            "the build command is not the one the settings were digested with",
        ));
        let m = Measurement::new(d).unwrap();
        let (run, items) = encode(&m).unwrap();
        assert_eq!(run.outcome, "failed");
        assert!(
            run.failure
                .as_deref()
                .is_some_and(|f| f.contains("\"kind\":\"settings_moved\"")),
            "{:?}",
            run.failure
        );
        let back = decode(&run, &items).unwrap();
        assert_eq!(back, m);
        assert_eq!(
            back.failure().map(MeasureFailure::kind),
            Some(FailureKind::SettingsMoved)
        );
    }
}
