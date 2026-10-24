//! Measurements as the store keeps them: a run is one row (its stamp as a JSON document of the revisions it stood
//! on, its failure and its evidence as JSON) and its mutants are item rows. The domain refuses what is not a
//! measurement, so a row that was changed by hand to something impossible is `Corrupt`, never quietly repaired.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entity::measurement::{Measurement, MeasurementDraft};
use crate::domain::repository::StoreError;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{ContractId, HarnessId, InterfaceId, MeasurementId};
use crate::domain::vo::measure::{
    FailureKind, MeasureFailure, MeasureKind, MeasureStamp, Mutant, MutantKey, MutantOutcome,
    MutantResult, Operator, Site, StepEvidence,
};
use crate::domain::vo::pin_basis::PinBasis;
use crate::domain::vo::revision::Revision;
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

/// The stamp, as the run row keeps it.
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

/// The run's counts, for a reader of the row that does not want the items.
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
struct DetailDoc {
    from: String,
    to: String,
}

/// A run, as the columns of `measure_runs` hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunRow {
    pub id: String,
    pub kind: String,
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

/// A mutant, as the columns of `measure_items` hold it.
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

/// The rows a measurement is kept as.
pub(crate) fn encode(m: &Measurement) -> Result<(RunRow, Vec<ItemRow>), StoreError> {
    let b = m.stamp().basis();
    let counts = m.counts();
    let run = RunRow {
        id: m.id().as_uuid().to_string(),
        kind: m.kind().word().to_string(),
        subject: m.subject().as_uuid().to_string(),
        stamp: m.stamp().fingerprint().to_string(),
        inputs: write(&StampDoc {
            code: m.stamp().code().to_string(),
            interface: b.interface().number(),
            harnesses: revs(b.harnesses(), |i: HarnessId| i.as_uuid()),
            contracts: revs(b.contracts(), |i: ContractId| i.as_uuid()),
            used_interfaces: revs(b.used_interfaces(), |i: InterfaceId| i.as_uuid()),
            settings: m.stamp().settings().to_string(),
        })?,
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
        summary: write(&SummaryDoc {
            sites: m.sites(),
            killed: counts.killed,
            survived: counts.survived,
            stillborn: counts.stillborn,
            timeout: counts.timeout,
        })?,
    };
    let items = m
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
    Ok((run, items))
}

/// The measurement the rows are, if the domain agrees they are one.
pub(crate) fn decode(run: &RunRow, items: &[ItemRow]) -> Result<Measurement, StoreError> {
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
    let mutants = items
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
        .collect::<Result<Vec<_>, StoreError>>()?;
    let uuid =
        |text: &str| Uuid::parse_str(text).map_err(|_| corrupt(format!("`{text}` is not an id")));
    Measurement::new(MeasurementDraft {
        id: MeasurementId::from_uuid(uuid(&run.id)?),
        kind: MeasureKind::from_word(&run.kind)
            .ok_or_else(|| corrupt(format!("kind `{}`", run.kind)))?,
        subject: InterfaceId::from_uuid(uuid(&run.subject)?),
        stamp,
        at: Timestamp::from_millis(
            u64::try_from(run.started_at).map_err(|_| corrupt("a negative time"))?,
        ),
        sites: summary.sites,
        mutants,
        failure,
        evidence,
    })
    .map_err(corrupt)
}
