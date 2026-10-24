//! Measuring: the operation that runs a measure over an Interface's pinned code and keeps what it observed, and the
//! reading of what is kept. A measure takes minutes and makes facts, so it is an operation of its own and never a
//! hook; the checks (`measure.failed`, `mutation.*`) only judge the record it leaves.

use std::collections::BTreeMap;

use super::ports::external::ExternalExecutor;
use super::ports::measure::{HarnessText, MutateRequest};
use super::ports::{AssetKind, Changes, IdSource, Store, StoreRevision};
use super::{App, AppError, context_id};
use crate::domain::entity::measurement::{Counts, Measurement, MeasurementDraft};
use crate::domain::entity::project::Project;
use crate::domain::repository::StoreError;
use crate::domain::service::measure::{Standing, standing};
use crate::domain::service::pin::current_basis;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{InterfaceId, MeasurementId};
use crate::domain::vo::measure::{MeasureKind, MeasureStamp};

/// How many times a commit that lost a race with another writer is tried again. A measurement stands on a stamp, not
/// on the store's revision, so a write in between does not make it wrong: it is kept, and a stale stamp would show.
const COMMIT_TRIES: usize = 5;

/// The latest measurements, and what the settings are now for the Interfaces the runner can say it for.
#[derive(Debug, Default)]
pub(super) struct Measured {
    pub(super) measurements: Vec<Measurement>,
    pub(super) settings: BTreeMap<InterfaceId, String>,
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
    /// How many mutants ended each way.
    pub counts: Counts,
    /// Each mutant that was run.
    pub mutants: Vec<MutantLine>,
    /// What the commands did.
    pub evidence: Vec<EvidenceLine>,
}

/// Where the latest measurement of an Interface stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementStanding {
    /// The Context's name.
    pub context: String,
    /// The Interface's name.
    pub interface: String,
    /// What was measured.
    pub kind: &'static str,
    /// The measurement's id.
    pub id: String,
    /// The stamp's digest.
    pub stamp: String,
    /// `current` while the Pin, what it stands on and the settings are as measured; `stale` once any moved.
    pub state: &'static str,
    /// `ok`, or `failed` when the run did not finish.
    pub outcome: &'static str,
    /// Why it failed, when it did.
    pub failure: Option<String>,
    /// How many mutants ended each way.
    pub counts: Counts,
    /// When the run began, in milliseconds since the epoch.
    pub at: u64,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// The latest measurements, and the settings now of the Interfaces they are of. An Interface the runner cannot
    /// say the settings of (no runner, no layout, a provider that is gone) is absent from the settings.
    pub(super) async fn measured(&self, p: &Project) -> Result<Measured, AppError> {
        let measurements = self.store.latest_measurements().await?;
        let mut settings = BTreeMap::new();
        for m in &measurements {
            let Some(context) = p.contexts().find(|c| c.interface(m.subject()).is_some()) else {
                continue;
            };
            let interface = self.name_of(m.subject().as_uuid()).await?;
            if let Ok(s) = self
                .runner
                .settings(&context.name().to_string(), &interface)
                .await
            {
                settings.insert(m.subject(), s);
            }
        }
        Ok(Measured {
            measurements,
            settings,
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
        let p = self.project().await?;
        let ctx = context_id(&p, context)?;
        let iface = InterfaceId::from_uuid(
            self.asset(ctx, context, AssetKind::Interface, interface)
                .await?,
        );
        let pin = self
            .store
            .pin_of(iface)
            .await?
            .ok_or_else(|| AppError::NotPinned {
                context: context.to_string(),
                interface: interface.to_string(),
            })?;
        let code = self
            .store
            .pin_code(pin.id())
            .await?
            .filter(|c| Digest::of(c.as_bytes()) == pin.code())
            .ok_or_else(|| AppError::PinCodeNotKept {
                interface: interface.to_string(),
            })?;
        let basis = current_basis(&p, iface).ok_or_else(|| AppError::NotPinned {
            context: context.to_string(),
            interface: interface.to_string(),
        })?;
        let mut harnesses = Vec::with_capacity(basis.harnesses().len());
        for id in basis.harnesses().keys() {
            let Some(h) = p
                .contexts()
                .flat_map(|c| c.harnesses())
                .find(|h| h.id() == *id)
            else {
                continue;
            };
            harnesses.push(HarnessText {
                name: self.name_of(id.as_uuid()).await?,
                source: h.source().as_str().to_string(),
            });
        }
        let settings = self.runner.settings(context, interface).await?;
        let stamp = MeasureStamp::new(pin.code(), basis, settings)?;
        let id = MeasurementId::from_uuid(self.ids.next_uuid());
        let at = self.clock.now();
        let request = MutateRequest {
            run: id.to_string(),
            context: context.to_string(),
            interface: interface.to_string(),
            code,
            harnesses,
        };
        let report = match kind {
            MeasureKind::Mutate => self.runner.mutate(&request).await?,
        };
        let measurement = Measurement::new(MeasurementDraft {
            id,
            kind,
            subject: iface,
            stamp,
            at,
            sites: report.sites,
            mutants: report.mutants,
            failure: report.failure,
            evidence: report.evidence,
        })?;
        self.keep_measurement(&measurement).await?;
        Ok(self.measured_words(&measurement))
    }

    /// Keeps `measurement` alone, trying again when another writer got in between.
    async fn keep_measurement(&self, measurement: &Measurement) -> Result<(), AppError> {
        for _ in 1..COMMIT_TRIES {
            let at: StoreRevision = self.store.revision().await?;
            let changes = Changes {
                measurements: vec![measurement.clone()],
                ..Changes::default()
            };
            match self.store.commit(at, changes).await {
                Err(StoreError::Conflict) => continue,
                other => return Ok(other?),
            }
        }
        let at = self.store.revision().await?;
        let changes = Changes {
            measurements: vec![measurement.clone()],
            ..Changes::default()
        };
        Ok(self.store.commit(at, changes).await?)
    }

    fn measured_words(&self, m: &Measurement) -> MeasureDone {
        MeasureDone {
            id: m.id().to_string(),
            kind: m.kind().word(),
            stamp: m.stamp().fingerprint().to_string(),
            failure: m.failure().map(ToString::to_string),
            sites: m.sites(),
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
        }
    }

    /// Where the latest measurement of each Interface stands, in the order of the Interfaces.
    pub(super) async fn measurement_standings(
        &self,
        p: &Project,
        measured: &Measured,
    ) -> Result<Vec<MeasurementStanding>, AppError> {
        let mut out = Vec::new();
        for c in p.contexts() {
            for i in c.interfaces() {
                for m in measured
                    .measurements
                    .iter()
                    .filter(|m| m.subject() == i.id())
                {
                    let state = match self.store.pin_of(i.id()).await? {
                        Some(pin) => standing(
                            m,
                            p,
                            &pin,
                            measured.settings.get(&i.id()).map(String::as_str),
                        ),
                        None => Standing::Stale,
                    };
                    out.push(MeasurementStanding {
                        context: c.name().to_string(),
                        interface: self.name_of(i.id().as_uuid()).await?,
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
                        at: m.at().millis(),
                    });
                }
            }
        }
        Ok(out)
    }
}
