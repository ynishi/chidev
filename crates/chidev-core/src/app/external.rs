//! External check orchestration: candidate input, validation, policy and atomic recording.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::ports::external::*;
use super::ports::{AssetName, Changes, IdSource, Invocation, Outcome, Store, StoreRevision};
use super::{App, AppError, Found, ModelView, input};
use crate::domain::check::external::ExternalRule;
use crate::domain::check::{CheckView, Executor, Finding, Hook, Level, ParamValue};
use crate::domain::entity::{change_request::ChangeRequest, project::Project};
use crate::domain::vo::{
    digest::Digest,
    finding::{RuleId, Subject, SubjectKind},
};

/// Whether a saved execution still describes this input and configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalState {
    /// No saved execution of this rule exists.
    Unrun,
    /// The input and rule configuration are unchanged; this does not mean the check passed.
    Current,
    /// The input or configuration changed since the execution.
    Stale,
}

impl ExternalState {
    /// Stable output word.
    pub fn word(self) -> &'static str {
        match self {
            Self::Unrun => "unrun",
            Self::Current => "current",
            Self::Stale => "stale",
        }
    }
}

/// Freshness of the last saved external execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStanding {
    /// Configured rule.
    pub rule: RuleId,
    /// Whether its saved execution describes the current input.
    pub state: ExternalState,
}

/// An explicitly requested run: results were recorded even if a gate found errors.
#[derive(Debug)]
pub struct ExternalChecked {
    /// Builtin, external and saved judged findings.
    pub findings: Vec<Found>,
    /// External attempts recorded by this run.
    pub external: Vec<ExternalReport>,
}

fn fingerprint(value: &impl Serialize) -> Result<String, AppError> {
    Ok(Digest::of(&serde_json::to_vec(value).map_err(input)?).to_string())
}

fn subject(s: &ExternalSubject) -> Option<Subject> {
    Subject::from_parts(
        SubjectKind::from_word(&s.kind)?,
        Uuid::parse_str(&s.id).ok()?,
    )
}

impl ExternalModelView {
    fn contains(&self, s: &ExternalSubject) -> bool {
        // An id is accepted only at a typed subject position in the restricted model.
        let matches = |v: &Value| v.get("id").and_then(Value::as_str) == Some(s.id.as_str());
        match s.kind.as_str() {
            "change_request" => self
                .change_requests
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|xs| xs.iter().any(matches)),
            "context" => self.contexts.iter().any(matches),
            "interface" | "item" | "clause" | "case" => self.contexts.iter().any(|c| {
                let (section, children) = match s.kind.as_str() {
                    "interface" => ("interfaces", None),
                    "item" => ("interfaces", Some("items")),
                    "clause" => ("contracts", Some("clauses")),
                    _ => ("harnesses", Some("cases")),
                };
                c.get(section)
                    .and_then(Value::as_array)
                    .is_some_and(|assets| {
                        assets.iter().any(|asset| {
                            children.map_or_else(
                                || matches(asset),
                                |key| {
                                    asset
                                        .get(key)
                                        .and_then(Value::as_array)
                                        .is_some_and(|xs| xs.iter().any(matches))
                                },
                            )
                        })
                    })
            }),
            _ => false,
        }
    }
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    async fn external_model(
        &self,
        rule: &ExternalRule,
        p: &Project,
        crs: &[ChangeRequest],
        pending: &[AssetName],
    ) -> Result<ExternalModelView, AppError> {
        let mut names = BTreeMap::new();
        for c in p.contexts() {
            let ids = c
                .contracts()
                .map(|x| x.id().as_uuid())
                .chain(c.interfaces().map(|x| x.id().as_uuid()))
                .chain(c.harnesses().map(|x| x.id().as_uuid()));
            for id in ids {
                if let Some(n) = pending.iter().find(|n| n.id == id) {
                    names.insert(id, n.name.clone());
                } else if let Some(n) = self.store.asset_name(id).await? {
                    names.insert(id, n);
                }
            }
        }
        let mut full = serde_json::to_value(ModelView::build(0, p, crs, &names)).map_err(input)?;
        let contexts = full["contexts"]
            .as_array_mut()
            .ok_or_else(|| input("invalid internal model view"))?;
        for c in contexts.iter_mut() {
            for key in ["contracts", "interfaces", "harnesses"] {
                if !rule.reads().iter().any(|r| r == key) {
                    c.as_object_mut()
                        .ok_or_else(|| input("invalid context view"))?
                        .remove(key);
                }
            }
        }
        Ok(ExternalModelView {
            version: super::VIEW_VERSION,
            contexts: contexts.clone(),
            change_requests: rule
                .reads()
                .iter()
                .any(|r| r == "change_requests")
                .then(|| full["change_requests"].clone()),
        })
    }

    fn external_config_stamp(&self, rule: &ExternalRule) -> Result<String, AppError> {
        let d = rule.descriptor();
        let params = self
            .registry
            .policy()
            .of(&d.id)
            .map(|r| {
                r.params
                    .iter()
                    .map(|(k, v)| {
                        let value = match v {
                            ParamValue::Bool(v) => json!(v),
                            ParamValue::Int(v) => json!(v),
                            ParamValue::Str(v) => json!(v),
                        };
                        (k.to_owned(), value)
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        fingerprint(
            &json!({"process":rule.config_digest(), "reads":rule.reads(), "gate":rule.gate(),
            "level":self.registry.level_of(&d.id).map(Level::word), "hooks":d.hooks.iter().map(|h|h.word()).collect::<Vec<_>>(), "params":params}),
        )
    }

    pub(super) async fn run_external(
        &self,
        hook: Hook,
        at: StoreRevision,
        view: &CheckView<'_>,
        pending: &[AssetName],
    ) -> Result<Vec<ExternalReport>, AppError> {
        let mut reports = Vec::new();
        for rule in self.registry.external() {
            let d = rule.descriptor();
            let Some(policy) = self.registry.policy().of(&d.id) else {
                continue;
            };
            if policy.level == Level::Off || !d.runs_at(hook) {
                continue;
            }
            let model = self
                .external_model(rule, view.after, view.change_requests, pending)
                .await?;
            let params = policy
                .params
                .iter()
                .map(|(key, value)| {
                    (
                        key.to_owned(),
                        match value {
                            ParamValue::Bool(v) => json!(v),
                            ParamValue::Int(v) => json!(v),
                            ParamValue::Str(v) => json!(v),
                        },
                    )
                })
                .collect();
            let call = ExternalCall {
                protocol: 1,
                hook: hook.word().into(),
                rules: vec![d.id.to_string()],
                read_at: at.0,
                params,
                model: model.clone(),
            };
            let attempt = self.external.execute(&d.id, call).await;
            let effective = if rule.gate() {
                Level::Error
            } else {
                policy.level.min(Level::Warning)
            };
            let validated = attempt.result.and_then(|reply| {
                let invalid = |message: &str| ExternalFailure {
                    kind: "protocol".into(),
                    message: message.into(),
                };
                if reply.protocol != 1 {
                    return Err(invalid("unsupported response protocol"));
                }
                reply
                    .findings
                    .into_iter()
                    .map(|f| {
                        if f.rule != d.id.as_str() {
                            return Err(invalid("response names an unrequested rule"));
                        }
                        if f.message.trim().is_empty() {
                            return Err(invalid("finding has an empty message"));
                        }
                        if let Some(s) = &f.subject
                            && (subject(s).is_none() || !model.contains(s))
                        {
                            return Err(invalid(
                                "finding subject is absent from the supplied model",
                            ));
                        }
                        let level = match f.level.as_deref() {
                            None => effective,
                            Some(l) => Level::from_word(l)
                                .filter(|l| *l != Level::Off)
                                .ok_or_else(|| invalid("invalid finding level"))?,
                        };
                        Ok(ExternalStoredFinding {
                            subject: f.subject,
                            message: f.message,
                            level: if rule.gate() {
                                Level::Error
                            } else {
                                level.min(effective)
                            }
                            .word()
                            .into(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            });
            let (findings, failure) = match validated {
                Ok(found) => (found, None),
                Err(e) => (
                    vec![ExternalStoredFinding {
                        subject: None,
                        message: format!("external check failed: {e}"),
                        level: effective.word().into(),
                    }],
                    Some(e.to_string()),
                ),
            };
            reports.push(ExternalReport {
                rule: d.id.to_string(),
                input_stamp: fingerprint(&model)?,
                config_stamp: self.external_config_stamp(rule)?,
                findings,
                evidence: attempt.evidence,
                failure,
            });
        }
        Ok(reports)
    }

    pub(super) fn external_invocations(
        &self,
        reports: &[ExternalReport],
        hook: Hook,
        at: StoreRevision,
    ) -> Vec<Invocation> {
        reports
            .iter()
            .filter_map(|r| {
                let rule = RuleId::try_from(r.rule.as_str()).ok()?;
                let count = |level: &str| {
                    r.findings
                        .iter()
                        .filter(|f| f.level == level)
                        .count()
                        .try_into()
                        .unwrap_or(u32::MAX)
                };
                Some(Invocation {
                    check: rule,
                    hook,
                    read_at: at,
                    notes: count("note"),
                    warnings: count("warning"),
                    errors: count("error"),
                    outcome: if r.failure.is_some() {
                        Outcome::Failed
                    } else if r.findings.is_empty() {
                        Outcome::Ok
                    } else {
                        Outcome::Found
                    },
                    executor: Executor::External,
                    judge: None,
                    external: Some(r.evidence.clone()),
                    rule_digest: None,
                })
            })
            .collect()
    }

    pub(super) async fn external_findings(
        &self,
        reports: &[ExternalReport],
        p: &Project,
        crs: &[ChangeRequest],
        pending: &[AssetName],
        stale: bool,
    ) -> Result<Vec<Found>, AppError> {
        let mut out = Vec::new();
        for r in reports {
            let Some(rule) = self
                .registry
                .external()
                .iter()
                .find(|rule| rule.descriptor().id.as_str() == r.rule)
            else {
                continue;
            };
            for f in &r.findings {
                let subject = f.subject.as_ref().and_then(subject);
                let finding = Finding::new(rule.descriptor(), subject, &f.message).at_level(
                    Level::from_word(&f.level)
                        .ok_or_else(|| input("invalid stored external severity"))?,
                );
                // A deleted subject still identifies the historical finding; rendering must not fail.
                let about = match subject {
                    Some(s) => Some(
                        self.describe_subject(p, crs, s, pending)
                            .await
                            .unwrap_or_else(|_| format!("{} {}", s.kind().word(), s.as_uuid())),
                    ),
                    None => None,
                };
                out.push(Found {
                    rule: finding.rule().clone(),
                    level: finding.level(),
                    state: finding.state_in(crs),
                    message: finding.message().into(),
                    help: rule.descriptor().help.to_string(),
                    about,
                    subject,
                    stale,
                    judgment: None,
                });
            }
        }
        Ok(out)
    }

    pub(super) async fn external_standing(
        &self,
        p: &Project,
        crs: &[ChangeRequest],
        _at: StoreRevision,
    ) -> Result<(Vec<Found>, Vec<ExternalStanding>), AppError> {
        let reports = self.store.external_reports().await?;
        let mut found = Vec::new();
        let mut standing = Vec::new();
        for rule in self.registry.external() {
            let id = &rule.descriptor().id;
            if self.registry.level_of(id) == Some(Level::Off) {
                continue;
            }
            let report = reports.iter().find(|r| r.rule == id.as_str());
            let state = if let Some(r) = report {
                let current = r.input_stamp
                    == fingerprint(&self.external_model(rule, p, crs, &[]).await?)?
                    && r.config_stamp == self.external_config_stamp(rule)?;
                found.extend(
                    self.external_findings(std::slice::from_ref(r), p, crs, &[], !current)
                        .await?,
                );
                if current {
                    ExternalState::Current
                } else {
                    ExternalState::Stale
                }
            } else {
                ExternalState::Unrun
            };
            standing.push(ExternalStanding {
                rule: id.clone(),
                state,
            });
        }
        Ok((found, standing))
    }

    /// Explicitly runs check-hook programs and persists their results and invocations. Unlike a change gate,
    /// errors are recorded and returned, since this operation changes no asset. No automatic retries occur.
    pub async fn check_run(&mut self) -> Result<ExternalChecked, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let pins = self.store.pins().await?;
        let crs = self.store.change_requests().await?;
        let measured = self.measured(&p).await?;
        let view = CheckView {
            before: None,
            after: &p,
            pins: &pins,
            change_requests: &crs,
            measurements: &measured.measurements,
            measure_settings: &measured.settings,
            sim_settings: &measured.sim_settings,
        };
        let mut run = self.registry.run(Hook::Check, at, &view);
        let external = self.run_external(Hook::Check, at, &view, &[]).await?;
        let mut findings = self.found(&run.findings, &p, &crs, &[]).await?;
        findings.extend(
            self.external_findings(&external, &p, &crs, &[], false)
                .await?,
        );
        findings.extend(self.judged_standing(&p, &crs, at).await?.0);
        run.invocations
            .extend(self.external_invocations(&external, Hook::Check, at));
        self.store
            .commit(
                at,
                Changes {
                    invocations: run.invocations,
                    external: external.clone(),
                    ..Changes::default()
                },
            )
            .await?;
        Ok(ExternalChecked { findings, external })
    }
}
