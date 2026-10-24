//! What a check says about itself, before it runs: shaped after SARIF's `reportingDescriptor` (an id, a short
//! description, a help text, a default configuration), so that a writer of SARIF has one place to read it from.

use super::level::{Hook, Level};
use crate::domain::vo::finding::RuleId;

/// Who runs a check. Only the checks that come with the code exist so far; a check run as a process, and a rule an
/// agent judges, are other executors that share the descriptor, the finding and the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Executor {
    /// Rust code of this crate, a pure function of the [`super::CheckView`].
    Builtin,
}

impl Executor {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Executor::Builtin => "builtin",
        }
    }
}

/// The type of a parameter. Parameters are scalars, so that a policy file stays flat and hashes the same each time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// true or false.
    Bool,
    /// A whole number.
    Int,
    /// A text.
    Str,
}

impl ParamKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            ParamKind::Bool => "bool",
            ParamKind::Int => "int",
            ParamKind::Str => "str",
        }
    }
}

/// The default of a parameter, as a `static` descriptor can hold it. Its variant is the parameter's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamDefault {
    /// true or false.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A text.
    Str(&'static str),
}

impl ParamDefault {
    /// The kind of the parameter.
    pub fn kind(self) -> ParamKind {
        match self {
            ParamDefault::Bool(_) => ParamKind::Bool,
            ParamDefault::Int(_) => ParamKind::Int,
            ParamDefault::Str(_) => ParamKind::Str,
        }
    }
}

/// A parameter a check takes: its name, a line on what it does, and the value it has when no policy sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    /// The name, as a policy writes it.
    pub name: &'static str,
    /// What it does, in a line.
    pub summary: &'static str,
    /// The value when nothing sets it; also the parameter's kind.
    pub default: ParamDefault,
}

/// A check described: what it is called, what it finds, where it runs, how serious it is by default.
#[derive(Debug)]
pub struct Descriptor {
    /// The rule it applies; a finding and a change request that addresses it are filed under this id.
    pub id: RuleId,
    /// What it finds, in a line.
    pub summary: &'static str,
    /// Why it matters and what to do about a finding.
    pub help: &'static str,
    /// The moments it runs at.
    pub hooks: &'static [Hook],
    /// The level it has when no policy says otherwise.
    pub default_level: Level,
    /// Whether a policy may not lower it below `error` or turn it off (as rustc's `forbid`): what the check finds
    /// must never be kept.
    pub mandatory: bool,
    /// The parameters it takes.
    pub params: &'static [ParamSpec],
    /// Who runs it.
    pub executor: Executor,
}

impl Descriptor {
    /// The parameter named `name`.
    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Whether it runs at `hook`.
    pub fn runs_at(&self, hook: Hook) -> bool {
        self.hooks.contains(&hook)
    }
}
