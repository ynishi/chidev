//! What a check says about itself, before it runs: shaped after SARIF's `reportingDescriptor` (an id, a short
//! description, a help text, a default configuration), so that a writer of SARIF has one place to read it from.

use std::borrow::Cow;

use super::level::{Hook, Level};
use crate::domain::vo::finding::RuleId;

/// Who runs a check. The three share the descriptor, the [`Finding`](super::Finding), the level and the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Executor {
    /// Rust code of this crate, a pure function of the [`super::CheckView`].
    Builtin,
    /// A project-registered program, executed through [`crate::app::ports::external::ExternalExecutor`].
    /// Its rule definition is [`super::external::ExternalRule`]; process registration, protocol transport,
    /// output bounds and cancellation belong to [`crate::infra::external`].
    ///
    /// Advisory checks are capped at warning; a gate is mandatory at error and fails closed on any
    /// execution or protocol failure. Successful candidate changes keep the result and execution evidence
    /// atomically. Refused changes return their attempts without persisting them. At `put` the findings of a
    /// successful run are held to what the put introduced, by their subjects, when the saved result of the rule
    /// describes the model before the put; a failed run refuses a gate whatever was saved.
    ///
    /// [`crate::app::App::check_run`] explicitly executes and records check-hook programs.
    /// Reading check/status uses saved results and never starts a program. The program has the caller's
    /// permissions, not a sandbox; its executable checksum records identity, not confinement.
    External,
    /// A rule written as data (`checks/judged/*.toml`) that the agent driving chidev judges: chidev keeps the
    /// rule and the results, hands the agent the rule and a view of the model, and records what the agent
    /// reports. It runs at no hook, so it can never be mandatory: its result arrives after the use case.
    Judged,
}

impl Executor {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Executor::Builtin => "builtin",
            Executor::External => "external",
            Executor::Judged => "judged",
        }
    }

    /// The executor a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<Executor> {
        [Executor::Builtin, Executor::External, Executor::Judged]
            .into_iter()
            .find(|e| e.word() == word)
    }
}

/// What a check speaks about. It is not stored: [`Descriptor::kind`] reads it from the hooks, by whether the check
/// runs at `put` (see [`Hook::Put`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// It runs at `put` and speaks about what an edit made stale or took out of place (a citation behind its
    /// target, a Pin whose basis moved, a keep that moved). At `put` it is held to what the edit introduced; at
    /// the confirmation hooks the whole model is judged.
    Change,
    /// It does not run at `put` and speaks about the state of the model: something is not there, not settled or
    /// not measured yet (a Clause no Case cites, a Context without an Interface). It is judged at the
    /// confirmation hooks and on demand.
    State,
}

impl CheckKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            CheckKind::Change => "change",
            CheckKind::State => "state",
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamDefault {
    /// true or false.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A text.
    Str(Cow<'static, str>),
}

impl ParamDefault {
    /// The kind of the parameter.
    pub fn kind(&self) -> ParamKind {
        match self {
            ParamDefault::Bool(_) => ParamKind::Bool,
            ParamDefault::Int(_) => ParamKind::Int,
            ParamDefault::Str(_) => ParamKind::Str,
        }
    }
}

/// A parameter a check takes: its name, a line on what it does, and the value it has when no policy sets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// The name, as a policy writes it.
    pub name: Cow<'static, str>,
    /// What it does, in a line.
    pub summary: Cow<'static, str>,
    /// The value when nothing sets it; also the parameter's kind.
    pub default: ParamDefault,
}

/// A check described: what it is called, what it finds, where it runs, how serious it is by default.
///
/// The texts and hooks are `Cow`s so that one type serves a builtin check, whose descriptor is a `static` that
/// borrows them, and a judged rule, whose descriptor is read from a file and owns them: the registry and the
/// policy take `&Descriptor` of either without leaking the rule's strings to make them `'static`. The parameters
/// are borrowed for builtins and owned for project-registered external checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    /// The rule it applies; a finding and a change request that addresses it are filed under this id.
    pub id: RuleId,
    /// What it finds, in a line.
    pub summary: Cow<'static, str>,
    /// Why it matters and what to do about a finding.
    pub help: Cow<'static, str>,
    /// The moments it runs at.
    pub hooks: Cow<'static, [Hook]>,
    /// The level it has when no policy says otherwise.
    pub default_level: Level,
    /// Whether a policy may not lower it below `error` or turn it off (as rustc's `forbid`): what the check finds
    /// must never be kept. At `put` only what the put introduced counts: a finding that was there before it does
    /// not refuse an edit that did not make it.
    pub mandatory: bool,
    /// The parameters it takes.
    pub params: Cow<'static, [ParamSpec]>,
    /// Who runs it.
    pub executor: Executor,
}

impl Descriptor {
    /// What it speaks about, read from its hooks so that there is one source. A check that runs at `put` speaks
    /// about what an edit made stale and is held there to what the edit introduced ([`CheckKind::Change`]); one
    /// that does not speaks about the state of the model and is judged at the confirmation hooks
    /// ([`CheckKind::State`]).
    pub fn kind(&self) -> CheckKind {
        if self.runs_at(Hook::Put) {
            CheckKind::Change
        } else {
            CheckKind::State
        }
    }

    /// The parameter named `name`.
    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Whether it runs at `hook`.
    pub fn runs_at(&self, hook: Hook) -> bool {
        self.hooks.contains(&hook)
    }
}
