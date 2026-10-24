//! Running the checks: the registry (the checks and the policy they run under), the runner that applies a hook to a
//! view, and the findings as a use case hands them to its caller. Pure: the view is built by the use case, and
//! what a run leaves to be kept (its invocations) goes out through `Changes`.

use super::ports::{Invocation, Outcome, StoreRevision};
use crate::domain::check::builtin;
use crate::domain::check::{
    Check, CheckView, Descriptor, Finding, FindingState, Hook, Layer, Level, Params, Policy,
    PolicyError,
};
use crate::domain::vo::finding::{RuleId, Subject};

/// The checks that can run and the policy they run under. Built once for a run of the program: the policy is
/// resolved when it is built, so a bad policy file is an error before any use case runs.
pub struct Registry {
    checks: Vec<&'static dyn Check>,
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
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// A check with the level and parameters it runs with, for listing.
#[derive(Debug, Clone, Copy)]
pub struct CheckInfo<'a> {
    /// What the check says about itself.
    pub descriptor: &'static Descriptor,
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
            policy: Policy::resolve(&descriptors, layers)?,
        })
    }

    /// The builtin checks under `layers`.
    pub fn builtin(layers: &[Layer]) -> Result<Registry, PolicyError> {
        Registry::new(builtin::all(), layers)
    }

    /// The builtin checks at their default levels.
    pub fn defaults() -> Registry {
        let checks = builtin::all();
        Registry {
            checks: checks.to_vec(),
            policy: Policy::defaults(checks.iter().map(|c| c.descriptor())),
        }
    }

    /// The policy.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Every check with the level and parameters it runs with, in order.
    pub fn list(&self) -> Vec<CheckInfo<'_>> {
        self.checks
            .iter()
            .filter_map(|c| {
                let d = c.descriptor();
                let r = self.policy.of(&d.id)?;
                Some(CheckInfo {
                    descriptor: d,
                    level: r.level,
                    params: &r.params,
                })
            })
            .collect()
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
}

/// What a use case did and what the checks found while it did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked<T> {
    /// What the use case returns.
    pub value: T,
    /// Every finding of the checks that ran at its hook, except those at `off`.
    pub findings: Vec<Found>,
}
