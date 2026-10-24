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
    /// A program of the project's own, run as a process; **designed, not implemented**: no check has this executor
    /// yet and nothing in the code starts a process. The design, so that the descriptor, the finding and the
    /// policy need no change when it comes:
    ///
    /// - **Manifest.** A table `[external."ext.<id>"]` in `checks.toml` gives the `command` and its `args`, the
    ///   `hooks` it runs at, a `timeout` in seconds, `reads` (which parts of the model it is sent: `contracts`,
    ///   `interfaces`, `harnesses`, `change_requests`; it is sent only those) and `gate` (default `false`). The
    ///   id starts with `ext.`, as a judged rule's starts with `judged.`, so the three executors never share one.
    /// - **Protocol.** The program is started once per hook with the request on its stdin, one JSON document:
    ///   `{ "protocol": 1, "hook": "put", "rules": ["ext.x"], "model": <the versioned view of the model> }`. It
    ///   answers on its stdout with `{ "protocol": 1, "findings": [{ "rule", "subject": <by id>, "message",
    ///   "level"? }] }`. The view is the one `check_rules` gives an agent (a version of its own, not the stored
    ///   documents' schema, so that the store and the programs can change apart); `params` of the policy are sent
    ///   with it. A finding of a rule not in `rules`, or about a subject that is not in the view, is malformed.
    /// - **Advisory by default.** What it finds is held at `warning` at most, whatever level it names or the
    ///   policy gives: a program the project merely installed must not refuse a use case. `gate = true` promotes
    ///   it: its findings are held at `error`, and it is **fail-closed**: a timeout, a crash, a non-zero exit or
    ///   an answer that is not the protocol is itself a finding at `error` (about the Project), so a gate that
    ///   cannot run is never read as a gate that passed.
    /// - **Recorded.** The SHA-256 of the command's file goes in the [`Invocation`] beside the model revision
    ///   read, so a run can be told apart from one made by another version of the program.
    /// - **Not sandboxed.** The program runs with the person's own rights, as a `git` hook does; the checksum
    ///   records which program ran and does not limit it. It is registered only by naming it in `checks.toml`.
    ///
    /// [`Invocation`]: crate::app::ports::Invocation
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
///
/// The texts and hooks are `Cow`s so that one type serves a builtin check, whose descriptor is a `static` that
/// borrows them, and a judged rule, whose descriptor is read from a file and owns them: the registry and the
/// policy take `&Descriptor` of either without leaking the rule's strings to make them `'static`. The parameters
/// stay `&'static`: only a builtin has any.
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
