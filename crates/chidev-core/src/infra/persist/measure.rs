//! Measurements as the store keeps them: a run is one row (its stamp as a JSON document of the revisions it stood
//! on, its failure and its evidence as JSON) and what it ran are item rows: the mutants of a mutation run, the
//! cases of a simulation. The domain refuses what is not a measurement, so a row that was changed by hand to
//! something impossible is `Corrupt`, never quietly repaired.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entity::measurement::{Measurement, MeasurementDraft};
use crate::domain::repository::StoreError;
use crate::domain::vo::clause::ClauseLabel;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{ContractId, HarnessId, InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    FailureKind, MeasureFailure, MeasureKind, MeasureStamp, MeasureSubject, Mutant, MutantKey,
    MutantOutcome, MutantResult, Operator, Site, Stamp, StepEvidence,
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
    sites: u32,
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
    label: String,
    text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnreachedDoc {
    label: String,
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
}

/// A mutant or a case, as the columns of `measure_items` hold it. A case has no site and no operator (they are
/// empty), its key is the case, its outcome the way the script ended and its detail what it found.
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

fn label_of(text: &str) -> Result<ClauseLabel, StoreError> {
    ClauseLabel::try_from(text).map_err(corrupt)
}

/// The stamp document and the summary document of a run.
fn stamp_and_summary(m: &Measurement) -> Result<(String, String), StoreError> {
    match m.stamp() {
        Stamp::Code(stamp) => {
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
                    sites: m.sites(),
                    killed: counts.killed,
                    survived: counts.survived,
                    stillborn: counts.stillborn,
                    timeout: counts.timeout,
                })?,
            ))
        }
        Stamp::Sim(stamp) => {
            let b = stamp.basis();
            let outcome = m.sim();
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
                    cases: outcome.map_or(0, |o| u32::try_from(o.cases.len()).unwrap_or(u32::MAX)),
                    findings: outcome
                        .map_or(0, |o| u32::try_from(o.found().len()).unwrap_or(u32::MAX)),
                    reruns: outcome
                        .map(|o| {
                            o.reruns
                                .iter()
                                .map(|r| RerunDoc {
                                    case: r.case.to_string(),
                                    result: r.result.word().to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
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
                        label: f.label.to_string(),
                        text: f.text.clone(),
                    })
                    .collect(),
                unreached: c
                    .unreached
                    .iter()
                    .map(|u| UnreachedDoc {
                        label: u.label.to_string(),
                        text: u.text.clone(),
                    })
                    .collect(),
                undecided: c.undecided.clone(),
                schedule: c.schedule.map(|d| d.to_string()),
                trace: c.trace.map(|d| d.to_string()),
            })?,
        });
    }
    Ok((run, items))
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
                            label: label_of(&f.label)?,
                            text: f.text,
                        })
                    })
                    .collect::<Result<_, StoreError>>()?,
                unreached: detail
                    .unreached
                    .into_iter()
                    .map(|u| {
                        Ok(CaseUnreached {
                            label: label_of(&u.label)?,
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
        (MeasureKind::Simulate, "harness") => {
            MeasureSubject::Harness(HarnessId::from_uuid(uuid(&run.subject)?))
        }
        (_, other) => {
            return Err(corrupt(format!("a `{}` run of a `{other}`", kind.word())));
        }
    };
    let (stamp, sites, mutants, sim): (Stamp, u32, Vec<MutantResult>, Option<SimOutcome>) =
        match kind {
            MeasureKind::Mutate => {
                let stamp: StampDoc = read(&run.inputs, "its stamp")?;
                let basis = PinBasis::new(
                    Revision::restore(stamp.interface).ok_or_else(|| corrupt("a revision of 0"))?,
                    unrevs(&stamp.harnesses, HarnessId::from_uuid)?,
                    unrevs(&stamp.contracts, ContractId::from_uuid)?,
                    unrevs(&stamp.used_interfaces, InterfaceId::from_uuid)?,
                );
                let code = Digest::restore(&stamp.code).ok_or_else(|| corrupt("a code digest"))?;
                let stamp = MeasureStamp::new(code, basis, stamp.settings).map_err(corrupt)?;
                let summary: SummaryDoc = read(&run.summary, "its summary")?;
                (stamp.into(), summary.sites, mutants_of(items)?, None)
            }
            MeasureKind::Simulate => {
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
                let stamp = SimStamp::new(basis, stamp.settings).map_err(corrupt)?;
                let summary: SimSummaryDoc = read(&run.summary, "its summary")?;
                (stamp.into(), 0, Vec::new(), Some(sim_of(&summary, items)?))
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
        kind,
        subject,
        stamp,
        at: Timestamp::from_millis(
            u64::try_from(run.started_at).map_err(|_| corrupt("a negative time"))?,
        ),
        sites,
        mutants,
        sim,
        failure,
        evidence,
    })
    .map_err(corrupt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entity::measurement::fixture::sim_draft;
    use crate::domain::vo::id::test_uuid;

    fn sim_measurement() -> Measurement {
        let case = |n: u32, label: u32| CaseRun {
            case: SimCase::Seed(n),
            exit: CaseExit::Normal,
            findings: vec![CaseFinding {
                step: 3,
                label: ClauseLabel::try_from(format!("C{label}").as_str()).unwrap(),
                text: "capacity 0 accepted: \"x\"".into(),
            }],
            unreached: vec![CaseUnreached {
                label: ClauseLabel::try_from("C9").unwrap(),
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
        assert!(
            bad_case(|i| i.detail = i.detail.replace("\"label\":\"C4\"", "\"label\":\"X\""))
                .is_err()
        );
        let mut stamp = run.clone();
        stamp.inputs =
            "{\"harness\":0,\"contracts\":[],\"interfaces\":[],\"settings\":\"s\"}".into();
        assert!(decode(&stamp, &items).is_err());
        let mut summary = run.clone();
        summary.summary = summary.summary.replace("trace-differs", "other");
        assert!(decode(&summary, &items).is_err());
    }
}
