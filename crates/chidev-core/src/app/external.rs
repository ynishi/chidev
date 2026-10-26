//! External check orchestration: candidate input, validation, policy and atomic recording.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::commit::AskTrigger;
use super::model_view::{ModelView, ViewSelection};
use super::ports::external::*;
use super::ports::{AssetName, IdSource, Invocation, Outcome, Store, StoreRevision};
use super::{App, AppError, Found, input};
use crate::domain::check::external::ExternalRule;
use crate::domain::check::{
    CheckView, DIGEST_MOMENTS_KEY, Executor, Finding, Level, Moment, ParamValue,
};
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
    /// Whether the subject is at a typed position of the model the program was sent. An id is accepted only there.
    fn contains(&self, s: &ExternalSubject) -> bool {
        let slice = self.slice();
        // Matched on the kind, not its word, so that a kind added to the domain is a case to decide here; the words a
        // reply may give (`subject_kinds`) are the kinds this does not refuse.
        match SubjectKind::from_word(&s.kind) {
            Some(SubjectKind::ChangeRequest) => slice.has_change_request(&s.id),
            Some(SubjectKind::Context) => slice.has_context(&s.id),
            Some(SubjectKind::Interface) => slice.has_interface(&s.id),
            Some(SubjectKind::Item) => slice.has_item(&s.id),
            Some(SubjectKind::Clause) => slice.has_clause(&s.id),
            Some(SubjectKind::Case) => slice.has_case(&s.id),
            Some(SubjectKind::Project) | None => false,
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
        let names = self.names_in_model(p.contexts(), pending).await?;
        let selection = ViewSelection {
            contexts: None,
            sections: rule.reads().iter().copied().collect(),
        };
        Ok(ExternalModelView::new(
            ModelView::build(0, p, crs, &names).select(&selection),
        ))
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
        let mut stamp = json!({"process":rule.config_digest(), "reads":rule.reads().iter().map(|s|s.word()).collect::<Vec<_>>(), "gate":rule.gate(),
            "level":self.registry.level_of(&d.id).map(Level::word), "params":params});
        // The moments are counted by their digest key, under the key the first digests used: a word that is
        // renamed must not make a saved result stale (see `Moment::digest_key`).
        stamp[DIGEST_MOMENTS_KEY] =
            json!(d.moments.iter().map(|m| m.digest_key()).collect::<Vec<_>>());
        fingerprint(&stamp)
    }

    pub(super) async fn run_external(
        &self,
        moment: Moment,
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
            if policy.level == Level::Off || !d.runs_at(moment) {
                continue;
            }
            let model = self
                .external_model(rule, view.project, view.change_requests, pending)
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
                protocol: REQUEST_PROTOCOL,
                moment: moment.word().into(),
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
                if !REPLY_PROTOCOLS.contains(&reply.protocol) {
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
                            Some(l) => finding_levels()
                                .find(|level| level.word() == l)
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

    /// What an edit is held to among the `reports` of the external checks run on the state after it: for a
    /// rule that ran, the findings its saved report does not have, when that report describes the state `before`
    /// (the same input and configuration, as [`App::external_standing`] tells), so that a finding that was there
    /// before the edit neither refuses it nor is reported again. Otherwise there is nothing to compare with and all
    /// the findings are judged. A failed run is never narrowed, and a failed saved report is no baseline: whether
    /// the program ran is not something a put leaves as it was. A finding is the same when its subject and its
    /// message are: the protocol gives no typed reason, so the message is the only mark of what makes it hold.
    pub(super) async fn judged_externals(
        &self,
        before: &CheckView<'_>,
        reports: &[ExternalReport],
    ) -> Result<Vec<ExternalReport>, AppError> {
        if reports.is_empty() {
            return Ok(Vec::new());
        }
        let saved = self.store.external_reports().await?;
        let mut judged = Vec::with_capacity(reports.len());
        for r in reports {
            let mut narrowed = r.clone();
            let rule = self
                .registry
                .external()
                .iter()
                .find(|rule| rule.descriptor().id.as_str() == r.rule);
            if let (Some(rule), None, Some(base)) =
                (rule, &r.failure, saved.iter().find(|s| s.rule == r.rule))
                && base.failure.is_none()
                && self
                    .describes(base, rule, before.project, before.change_requests)
                    .await?
            {
                // The protocol gives a finding no typed grounds, so its message is the only mark of the fact that
                // makes it hold: a finding is known when the base has one about the same subject with the same
                // message. Comparing the subject alone would make every finding about the whole Project one.
                let known: Vec<(Option<Subject>, &str)> = base
                    .findings
                    .iter()
                    .map(|f| (f.subject.as_ref().and_then(subject), f.message.as_str()))
                    .collect();
                narrowed.findings.retain(|f| {
                    !known.contains(&(f.subject.as_ref().and_then(subject), f.message.as_str()))
                });
            }
            judged.push(narrowed);
        }
        Ok(judged)
    }

    /// Whether the saved `report` of `rule` describes the model `p` with the change requests `crs`: the same input
    /// and the same configuration of the rule as it is registered now.
    async fn describes(
        &self,
        report: &ExternalReport,
        rule: &ExternalRule,
        p: &Project,
        crs: &[ChangeRequest],
    ) -> Result<bool, AppError> {
        Ok(report.config_stamp == self.external_config_stamp(rule)?
            && report.input_stamp == fingerprint(&self.external_model(rule, p, crs, &[]).await?)?)
    }

    pub(super) fn external_invocations(
        &self,
        reports: &[ExternalReport],
        moment: Moment,
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
                    moment,
                    trigger: None,
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
                let current = self.describes(r, rule, p, crs).await?;
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

    /// Explicitly runs check-moment programs and persists their results and invocations. Unlike a change gate,
    /// errors are recorded and returned, since this operation changes no asset. No automatic retries occur.
    pub async fn check_run(&mut self) -> Result<ExternalChecked, AppError> {
        let at = self.store.revision().await?;
        self.commit_ask(AskTrigger::CheckRun, at).await
    }
}
