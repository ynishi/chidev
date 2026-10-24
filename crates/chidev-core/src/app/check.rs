//! Running the checks: the registry (the checks and the policy they run under), the runner that applies a hook to a
//! view, and the findings as a use case hands them to its caller. Pure: the view is built by the use case, and
//! what a run leaves to be kept (its invocations) goes out through `Changes`.

use super::ports::{Invocation, Outcome, StoreRevision};
use crate::domain::check::builtin;
use thiserror::Error;

use crate::domain::check::{
    Check, CheckView, Descriptor, Executor, Finding, FindingState, Hook, JudgedRule, Layer, Level,
    Params, Policy, PolicyError,
};
use crate::domain::vo::finding::{RuleId, Subject};

/// Why the checks and the policy make no registry.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    /// The policy cannot be resolved over the checks.
    #[error(transparent)]
    Policy(#[from] PolicyError),
    /// Two checks have this id; a finding and a change request are filed under it, so it names one.
    #[error("two checks have the id `{0}`")]
    DuplicateRule(RuleId),
}

/// The checks that can run and the policy they run under. Built once for a run of the program: the policy is
/// resolved when it is built, so a bad policy file is an error before any use case runs. The builtin checks are
/// code and run in the hooks; the judged rules are data read from the project's files, held by the same policy
/// and run by no one here: an agent judges them and reports.
pub struct Registry {
    checks: Vec<&'static dyn Check>,
    judged: Vec<JudgedRule>,
    policy: Policy,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field(
                "checks",
                &self
                    .checks
                    .iter()
                    .map(|c| c.descriptor().id.as_str())
                    .chain(self.judged.iter().map(|r| r.id().as_str()))
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// A check with the level and parameters it runs with, for listing.
#[derive(Debug, Clone, Copy)]
pub struct CheckInfo<'a> {
    /// What the check says about itself.
    pub descriptor: &'a Descriptor,
    /// The level it runs at.
    pub level: Level,
    /// The parameters it runs with.
    pub params: &'a Params,
}

/// What one hook did over a view.
#[derive(Debug, Default)]
pub struct Run {
    /// Every finding, at the level the policy gives, in the order of the checks. A check at `off` did not run.
    pub findings: Vec<Finding>,
    /// One record per check that ran.
    pub invocations: Vec<Invocation>,
    /// The findings of mandatory checks at `error`: they refuse the use case.
    pub refusing: Vec<Finding>,
}

impl Registry {
    /// `checks` under the policy the `layers` make (project, then person) on top of their descriptors' defaults.
    pub fn new(checks: &[&'static dyn Check], layers: &[Layer]) -> Result<Registry, PolicyError> {
        let descriptors: Vec<&Descriptor> = checks.iter().map(|c| c.descriptor()).collect();
        Ok(Registry {
            checks: checks.to_vec(),
            judged: Vec::new(),
            policy: Policy::resolve(&descriptors, layers)?,
        })
    }

    /// `checks` and the `judged` rules under the policy the `layers` make. A judged rule is held by the policy as
    /// a builtin is (a family `judged.` or the rule's own id), so a person can turn it off or change its level.
    pub fn with_judged(
        checks: &[&'static dyn Check],
        judged: Vec<JudgedRule>,
        layers: &[Layer],
    ) -> Result<Registry, RegistryError> {
        let mut seen: Vec<&RuleId> = checks.iter().map(|c| &c.descriptor().id).collect();
        for r in &judged {
            if seen.contains(&r.id()) {
                return Err(RegistryError::DuplicateRule(r.id().clone()));
            }
            seen.push(r.id());
        }
        let descriptors: Vec<&Descriptor> = checks
            .iter()
            .map(|c| c.descriptor())
            .chain(judged.iter().map(JudgedRule::descriptor))
            .collect();
        let policy = Policy::resolve(&descriptors, layers)?;
        Ok(Registry {
            checks: checks.to_vec(),
            judged,
            policy,
        })
    }

    /// The builtin checks under `layers`.
    pub fn builtin(layers: &[Layer]) -> Result<Registry, PolicyError> {
        Registry::new(builtin::all(), layers)
    }

    /// The builtin checks and the `judged` rules under `layers`.
    pub fn builtin_with_judged(
        judged: Vec<JudgedRule>,
        layers: &[Layer],
    ) -> Result<Registry, RegistryError> {
        Registry::with_judged(builtin::all(), judged, layers)
    }

    /// The builtin checks at their default levels.
    pub fn defaults() -> Registry {
        let checks = builtin::all();
        Registry {
            checks: checks.to_vec(),
            judged: Vec::new(),
            policy: Policy::defaults(checks.iter().map(|c| c.descriptor())),
        }
    }

    /// The policy.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Every check with the level and parameters it runs with, in order: the builtin checks, then the judged
    /// rules.
    pub fn list(&self) -> Vec<CheckInfo<'_>> {
        self.checks
            .iter()
            .map(|c| c.descriptor())
            .chain(self.judged.iter().map(JudgedRule::descriptor))
            .filter_map(|d| {
                let r = self.policy.of(&d.id)?;
                Some(CheckInfo {
                    descriptor: d,
                    level: r.level,
                    params: &r.params,
                })
            })
            .collect()
    }

    /// The judged rules, in the order they were given.
    pub fn judged(&self) -> &[JudgedRule] {
        &self.judged
    }

    /// The judged rule `id`.
    pub fn judged_rule(&self, id: &RuleId) -> Option<&JudgedRule> {
        self.judged.iter().find(|r| r.id() == id)
    }

    /// Who runs the check `id`; none when there is no such check.
    pub fn executor_of(&self, id: &RuleId) -> Option<Executor> {
        self.checks
            .iter()
            .map(|c| c.descriptor())
            .chain(self.judged.iter().map(JudgedRule::descriptor))
            .find(|d| d.id == *id)
            .map(|d| d.executor)
    }

    /// The level the policy gives the check `id`.
    pub fn level_of(&self, id: &RuleId) -> Option<Level> {
        self.policy.of(id).map(|r| r.level)
    }

    /// Runs the checks that run at `hook` and are not off over `view`, which the store stood at `read_at` for.
    pub fn run(&self, hook: Hook, read_at: StoreRevision, view: &CheckView<'_>) -> Run {
        let mut run = Run::default();
        for check in &self.checks {
            let d = check.descriptor();
            let Some(resolved) = self.policy.of(&d.id) else {
                continue;
            };
            if resolved.level == Level::Off || !d.runs_at(hook) {
                continue;
            }
            let found: Vec<Finding> = check
                .run(view, &resolved.params)
                .into_iter()
                .map(|f| f.at_level(resolved.level))
                .collect();
            let mut invocation = Invocation {
                check: d.id.clone(),
                hook,
                read_at,
                notes: 0,
                warnings: 0,
                errors: 0,
                outcome: if found.is_empty() {
                    Outcome::Ok
                } else {
                    Outcome::Found
                },
                executor: Executor::Builtin,
                judge: None,
            };
            for f in &found {
                match f.level() {
                    Level::Note => invocation.notes += 1,
                    Level::Warning => invocation.warnings += 1,
                    Level::Error => invocation.errors += 1,
                    Level::Off => {}
                }
            }
            if d.mandatory && resolved.level == Level::Error {
                run.refusing.extend(found.iter().cloned());
            }
            run.findings.extend(found);
            run.invocations.push(invocation);
        }
        run
    }
}

/// A finding as a use case hands it to its caller: with the state it is in and the thing it is about in the words
/// the person knows it by (the store's names, which the domain does not have).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The rule.
    pub rule: RuleId,
    /// How serious.
    pub level: Level,
    /// Whether a change request deals with it.
    pub state: FindingState,
    /// What is wrong.
    pub message: String,
    /// What it is about, in words; none for the Project as a whole.
    pub about: Option<String>,
    /// What it is about, by id.
    pub subject: Option<Subject>,
    /// Whether it was judged on a model that has since changed (its subject changed or is gone, or the agent judged
    /// an older model than the one it was kept against): a judged finding may no longer hold. Separate from
    /// `state`, which is about the change requests that deal with it: a decided finding can be stale too. A
    /// builtin finding is run on the model as it is, so it is never stale.
    pub stale: bool,
    /// Who judged it and on what, for a finding of a judged rule; none for a builtin.
    pub judgment: Option<Judgment>,
}

/// Who judged a finding and on which revision of the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgment {
    /// The agent.
    pub judge: String,
    /// The revision of the store the view it judged from was read at.
    pub read_at: StoreRevision,
    /// Whether the model had changed since it read the view when it reported.
    pub on_older_revision: bool,
}

/// What a use case did and what the checks found while it did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked<T> {
    /// What the use case returns.
    pub value: T,
    /// Every finding of the checks that ran at its hook, except those at `off`.
    pub findings: Vec<Found>,
}
