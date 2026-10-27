//! The judged rules in use: what an agent is given to judge from (`check_rules`), what it reports back
//! (`finding_report`), and how what it reported stands beside the findings of the builtin checks.
//!
//! chidev does not judge. It keeps the rules and the results; an agent asks for the rules and a view of the model,
//! judges, and reports each rule as a whole, which replaces what was reported for that rule before. A report is not
//! a use case that changes the model, so it runs no check and is never refused by one; it is kept with the
//! store's one commit, with the record of the run.

use std::cmp::min;

use super::check::{Found, Judgment};
use super::commit::{RecordChanges, RecordTrigger};
use super::model_view::ModelView;
use super::ports::{
    IdSource, Invocation, JudgedFinding, JudgedReport, Outcome, Store, StoreRevision,
};
use super::{App, AppError, SubjectInput, input};
use crate::domain::check::{Executor, Finding, JudgedRule, Level, Moment};
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::crux::Crux;
use crate::domain::entity::project::Project;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{RuleId, SubjectKind};

/// Whether what an agent reported for a judged rule can still be relied on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgedState {
    /// No agent has reported it: it was never judged.
    Never,
    /// Reported on this rule definition, and the part of the model it judges is as it was.
    Current,
    /// Reported on an unknown or different rule definition, a changed model scope, or an older model: judge again.
    Stale,
}

impl JudgedState {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            JudgedState::Never => "never",
            JudgedState::Current => "current",
            JudgedState::Stale => "stale",
        }
    }
}

/// A judged rule an agent is asked to judge, with the level it is held to and where its judging stands.
#[derive(Debug, Clone)]
pub struct RuleToJudge {
    /// The rule: the question and what answers it.
    pub rule: JudgedRule,
    /// Definition version to echo in the report.
    pub rule_digest: Digest,
    /// The level the policy gives it; a finding is reported at it or below.
    pub level: Level,
    /// Whether it was judged, and whether that still holds.
    pub state: JudgedState,
    /// Who last reported it and on what; none when it never was.
    pub judgment: Option<Judgment>,
}

/// What `check_rules` gives an agent: the rules to judge and the model to judge them on.
#[derive(Debug, Clone)]
pub struct RulesView {
    /// The revision of the store the view was read at; a report names it as `read_at`.
    pub store_revision: StoreRevision,
    /// The rules, in the order of the registry; those that are off are left out.
    pub rules: Vec<RuleToJudge>,
    /// The model.
    pub model: ModelView,
}

/// A judged rule that is not up to date: what `status` tells an agent to judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleStanding {
    /// The rule.
    pub rule: RuleId,
    /// What it finds, in a line.
    pub summary: String,
    /// What kind of thing it judges.
    pub subject: SubjectKind,
    /// `Never` or `Stale`.
    pub state: JudgedState,
}

/// A finding an agent reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedFinding {
    /// What it is about; none: the Project as a whole.
    pub subject: Option<SubjectInput>,
    /// What is wrong, in words.
    pub message: String,
    /// A level below the rule's, to say it matters less; none: the rule's.
    pub level: Option<Level>,
}

/// What an agent reports for one judged rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportInput {
    /// The rule (`judged.vague-clause`).
    pub rule: String,
    /// The `rule_digest` check_rules gave for the rule, echoed back: it names the definition judged. Required.
    pub rule_digest: String,
    /// The agent that judged.
    pub judge: String,
    /// The store revision of the view it judged from.
    pub read_at: u64,
    /// Every finding of the rule; none: it found nothing. It replaces what was reported before.
    pub findings: Vec<ReportedFinding>,
}

/// What a report did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reported {
    /// The rule.
    pub rule: RuleId,
    /// The definition version the saved report was judged on.
    pub rule_digest: Digest,
    /// Whether the saved report is current or needs judgment again.
    pub state: JudgedState,
    /// How many findings it now has.
    pub findings: usize,
    /// How many findings it had before, which the report replaced.
    pub replaced: usize,
    /// Whether the model had changed since the agent read the view: the findings were kept, and show as stale.
    pub on_older_revision: bool,
    /// The revision the store has with the report in it.
    pub store_revision: StoreRevision,
}

/// Where a report stands against the view of the model now.
fn state_of(rule: &JudgedRule, report: Option<&JudgedReport>, view: &ModelView) -> JudgedState {
    match report {
        None => JudgedState::Never,
        Some(r)
            if r.rule_digest != Some(rule.digest())
                || r.on_older_revision
                || view.scope_stamp(rule.subject()) != r.scope =>
        {
            JudgedState::Stale
        }
        Some(_) => JudgedState::Current,
    }
}

impl<S, G, E> App<S, G, E>
where
    S: Store,
    G: IdSource,
    E: super::ports::external::ExternalExecutor,
{
    /// The judged rules to judge at `moment` (all when none), or the one named `rule`, with the level each is held
    /// to and a view of the model to judge them on. Rules the policy turned off are left out.
    pub async fn check_rules(
        &self,
        moment: Option<Moment>,
        rule: Option<&str>,
    ) -> Result<RulesView, AppError> {
        let at = self.store.revision().await?;
        let wanted = rule
            .map(|r| RuleId::try_from(r).map_err(input))
            .transpose()?;
        let named = wanted.as_ref().map(|id| self.judged_rule(id)).transpose()?;
        let p = self.project().await?;
        let (crs, cruxes) = self.requests_and_cruxes().await?;
        let reports = self.store.judged_reports().await?;
        let model = self.model_view(&p, &crs, &cruxes, at).await?;
        let mut rules = Vec::new();
        for r in self.registry.judged() {
            if named.is_some_and(|n| n.id() != r.id())
                || moment.is_some_and(|h| !r.descriptor().runs_at(h))
            {
                continue;
            }
            let level = self.registry.level_of(r.id()).unwrap_or(Level::Off);
            if level == Level::Off {
                if named.is_some() {
                    return Err(AppError::RuleOff(r.id().to_string()));
                }
                continue;
            }
            let report = reports.iter().find(|x| x.rule == *r.id());
            rules.push(RuleToJudge {
                rule: r.clone(),
                rule_digest: r.digest(),
                level,
                state: state_of(r, report, &model),
                judgment: report.map(|x| Judgment {
                    judge: x.judge.clone(),
                    read_at: x.read_at,
                    on_older_revision: x.on_older_revision,
                    rule_digest: x.rule_digest,
                }),
            });
        }
        Ok(RulesView {
            store_revision: at,
            rules,
            model,
        })
    }

    /// Keeps what an agent reports for a judged rule, replacing every finding of the rule reported before (an empty
    /// report says the rule was judged and found nothing). Refused: a rule that does not exist, that is not judged
    /// or that the policy turned off; a judge with no name; a `read_at` the store has not reached; a subject that
    /// is not of the kind the rule judges, or that does not exist, or that is given twice; an empty message; a
    /// level above the rule's (the policy decides how much a rule may matter, the agent only that a finding
    /// matters less) or `off`. A `read_at` older than the last change of the model is accepted, and the report is
    /// marked as made on an older revision, so that its findings show as stale: the agent judged a model that is no
    /// longer there. The rule_digest is required and must identify this Registry's definition; a missing,
    /// malformed or mismatching digest refuses the report without replacing the previous judgment or
    /// recording a run.
    /// Registry definitions are fixed for this App; changing files does not reload them.
    pub async fn finding_report(&mut self, report: &ReportInput) -> Result<Reported, AppError> {
        let at = self.store.revision().await?;
        let model_at = self.store.model_revision().await?;
        let id = RuleId::try_from(report.rule.as_str()).map_err(input)?;
        let rule = self.judged_rule(&id)?.clone();
        let policy_level = self.registry.level_of(&id).unwrap_or(Level::Off);
        if policy_level == Level::Off {
            return Err(AppError::RuleOff(id.to_string()));
        }
        let rule_digest = Digest::restore(&report.rule_digest).ok_or_else(|| {
            AppError::Input("rule_digest is required: 64 lower-case hexadecimal digits".into())
        })?;
        let expected = rule.digest();
        if rule_digest != expected {
            let reported = rule_digest;
            return Err(AppError::RuleVersionMismatch {
                rule: id.to_string(),
                expected,
                reported,
            });
        }
        let judge = report.judge.trim();
        if judge.is_empty() {
            return Err(AppError::Input("a report names the judge".to_string()));
        }
        if report.read_at > at.0 {
            return Err(AppError::Input(format!(
                "read_at is {}, but the store is at {}",
                report.read_at, at.0
            )));
        }
        let read_at = StoreRevision(report.read_at);
        let p = self.project().await?;
        let (crs, cruxes) = self.requests_and_cruxes().await?;
        let view = self.model_view(&p, &crs, &cruxes, at).await?;
        let mut kept: Vec<JudgedFinding> = Vec::with_capacity(report.findings.len());
        for f in &report.findings {
            let subject = match &f.subject {
                None => None,
                Some(s) => Some(self.subject_of(&p, s).await?),
            };
            if !rule.admits(subject) {
                return Err(AppError::Input(format!(
                    "the rule `{id}` judges {} findings; this one is about {}",
                    rule.subject().word(),
                    SubjectKind::of(subject).word()
                )));
            }
            if kept.iter().any(|k| k.subject == subject) {
                return Err(AppError::Input(format!(
                    "the rule `{id}` has two findings about the same subject; give one"
                )));
            }
            let message = f.message.trim();
            if message.is_empty() {
                return Err(AppError::Input("a finding says what is wrong".to_string()));
            }
            match f.level {
                Some(Level::Off) => {
                    return Err(AppError::Input(
                        "a finding is at note, warning or error; to say there is none, leave it out"
                            .to_string(),
                    ));
                }
                Some(l) if l > policy_level => {
                    return Err(AppError::Input(format!(
                        "a finding of `{id}` is at {l}, above the {policy_level} the policy holds the rule to"
                    )));
                }
                _ => {}
            }
            let stamp = view
                .stamp_of(subject)
                .ok_or_else(|| AppError::Input("the subject is not in the model".to_string()))?;
            kept.push(JudgedFinding {
                subject,
                message: message.to_string(),
                level: f.level,
                stamp,
            });
        }
        let before = self
            .store
            .judged_reports()
            .await?
            .into_iter()
            .find(|r| r.rule == id)
            .map_or(0, |r| r.findings.len());
        let on_older_revision = read_at < model_at;
        let held = |f: &JudgedFinding| min(policy_level, f.level.unwrap_or(policy_level));
        let count = |l: Level| {
            u32::try_from(kept.iter().filter(|f| held(f) == l).count()).unwrap_or(u32::MAX)
        };
        let invocation = Invocation {
            check: id.clone(),
            moment: Moment::Check,
            trigger: None,
            read_at,
            notes: count(Level::Note),
            warnings: count(Level::Warning),
            errors: count(Level::Error),
            outcome: if kept.is_empty() {
                Outcome::Ok
            } else {
                Outcome::Found
            },
            executor: Executor::Judged,
            judge: Some(judge.to_string()),
            external: None,
            rule_digest: Some(rule_digest),
        };
        let stored = StoreRevision(at.0 + 1);
        let stored_report = JudgedReport {
            rule: id.clone(),
            rule_digest: Some(rule_digest),
            judge: judge.to_string(),
            read_at,
            reported_at: stored,
            on_older_revision,
            scope: view.scope_stamp(rule.subject()),
            findings: kept,
        };
        let findings = stored_report.findings.len();
        let state = state_of(&rule, Some(&stored_report), &view);
        let record = RecordChanges {
            judged: vec![stored_report],
            invocations: vec![invocation],
            ..RecordChanges::default()
        };
        self.commit_record(RecordTrigger::ReportFinding, at, record)
            .await?;
        Ok(Reported {
            rule: id,
            rule_digest,
            state,
            findings,
            replaced: before,
            on_older_revision,
            store_revision: stored,
        })
    }

    /// The findings agents reported, as callers see findings, and the judged rules that are not up to date.
    /// Reported findings are the standing record, so they show whatever moments their rules belong to (the
    /// moments say when an agent is asked to judge, not when a result is shown). A rule the policy turned off shows none.
    pub(super) async fn judged_standing(
        &self,
        p: &Project,
        crs: &[ChangeRequest],
        cruxes: &[Crux],
        at: StoreRevision,
    ) -> Result<(Vec<Found>, Vec<RuleStanding>), AppError> {
        if self.registry.judged().is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let reports = self.store.judged_reports().await?;
        let view = self.model_view(p, crs, cruxes, at).await?;
        let mut findings = Vec::new();
        let mut meta = Vec::new();
        let mut standing = Vec::new();
        for rule in self.registry.judged() {
            let level = self.registry.level_of(rule.id()).unwrap_or(Level::Off);
            if level == Level::Off {
                continue;
            }
            let report = reports.iter().find(|r| r.rule == *rule.id());
            let state = state_of(rule, report, &view);
            if state != JudgedState::Current {
                standing.push(RuleStanding {
                    rule: rule.id().clone(),
                    summary: rule.descriptor().summary.to_string(),
                    subject: rule.subject(),
                    state,
                });
            }
            let Some(r) = report else { continue };
            for f in &r.findings {
                let held = min(level, f.level.unwrap_or(level));
                findings.push(
                    Finding::new(rule.descriptor(), f.subject, f.message.clone()).at_level(held),
                );
                let stale = r.rule_digest != Some(rule.digest())
                    || r.on_older_revision
                    || view.stamp_of(f.subject) != Some(f.stamp);
                meta.push((
                    stale,
                    Judgment {
                        judge: r.judge.clone(),
                        read_at: r.read_at,
                        on_older_revision: r.on_older_revision,
                        rule_digest: r.rule_digest,
                    },
                ));
            }
        }
        let mut found = self.found(&findings, p, crs, &[]).await?;
        for (f, (stale, judgment)) in found.iter_mut().zip(meta) {
            f.stale = stale;
            f.judgment = Some(judgment);
        }
        Ok((found, standing))
    }

    /// The judged rule `id`; an error says what it is when it is not one.
    fn judged_rule(&self, id: &RuleId) -> Result<&JudgedRule, AppError> {
        match self.registry.judged_rule(id) {
            Some(r) => Ok(r),
            None => Err(match self.registry.executor_of(id) {
                Some(e) => AppError::NotJudged {
                    rule: id.to_string(),
                    executor: e.word(),
                },
                None => AppError::UnknownRule(id.to_string()),
            }),
        }
    }

    /// The view of the model, with the names of the assets, read when the store stood at `at`.
    async fn model_view(
        &self,
        p: &Project,
        crs: &[ChangeRequest],
        cruxes: &[Crux],
        at: StoreRevision,
    ) -> Result<ModelView, AppError> {
        let names = self.names_in_model(p.contexts(), &[]).await?;
        Ok(ModelView::build(at.0, p, crs, cruxes, &names))
    }
}
