//! Diagnostics: what a refusal, an input that cannot be read, or a finding says to whoever drives chidev, as
//! structure. A [`Diagnostic`] has the stable `code` (the id of the rule), the `message` (the rule's template filled
//! with names), the things it is about (named), the `field` of the input it is about, the typed `values`, the
//! rule's `help`, `suggestions` (the operations that would mend it, each with how far they may be trusted) and
//! `did_you_mean` (the names that exist and are close to one that does not).
//!
//! The domain gives a [`Violation`] (a rule id, ids, typed values, typed remedies); this layer is the one that has
//! the names (the store's), so it writes the sentence, turns the remedies into operations, and computes the
//! suggestions of a name that does not exist. An input, a CLI or an MCP server only lists the diagnostics.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value as Json, json};

use super::ports::external::ExternalExecutor;
use super::ports::{AssetName, IdSource, Store};
use super::{App, AppError, Found, TargetInput};
use crate::domain::check::Level;
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::project::Project;
use crate::domain::rule::{
    self, APP_UNMAPPED, CR_TITLE_TAKEN, INPUT_INVALID, INPUT_MISSING_FIELD, INPUT_UNKNOWN_FIELD,
    INPUT_UNKNOWN_NAME, INPUT_UNKNOWN_OP, INPUT_WRONG_TYPE, PROJECT_ASSET_RETIRED, PROJECT_NONE,
    Rule, RuleKind,
};
use crate::domain::violation::{Applicability, Party, Remedy, Value, Violation};
use crate::domain::vo::finding::RuleId;
use crate::domain::vo::id::ChangeRequestId;
use crate::domain::vo::target::Target;

/// The names of what a violation is about, as the application found them: the words a person knows each by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names(BTreeMap<Party, String>);

impl Names {
    /// Records the words for `party`.
    pub fn insert(&mut self, party: Party, words: String) {
        self.0.insert(party, words);
    }

    /// The words for `party`, when they are known.
    pub fn get(&self, party: &Party) -> Option<&str> {
        self.0.get(party).map(String::as_str)
    }
}

/// A thing a diagnostic is about: its kind, and the words it is known by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagSubject {
    /// `case`, `contract`, `change_request`, `context`, ...
    pub kind: &'static str,
    /// The words, as `status` shows it.
    pub name: String,
}

/// An operation that would mend a diagnostic, and how far it may be trusted. `op` is the operation as a JSON
/// object an agent can send (with `"<..>"` where a value is to be filled in), or, when no single operation does
/// it, a sentence saying what to do.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    /// The operation, or a description.
    pub op: Json,
    /// `machine` (apply as it stands), `review` (look at it first) or `placeholder` (fill the `<..>` in).
    pub applicability: &'static str,
}

/// One thing to tell: a refusal, an input that cannot be read, or a finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    /// The id of the rule; stable.
    pub code: String,
    /// `error`, `warning` or `note`.
    pub level: &'static str,
    /// The sentence, from the rule's template and the names.
    pub message: String,
    /// What it is about, named, in order.
    pub subjects: Vec<DiagSubject>,
    /// Where in the input it is about (`for`, `puts[0].context`), when it is about the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The typed values of the violation, by key. A key is never removed from a rule, only added.
    pub values: BTreeMap<String, Json>,
    /// Why the rule holds and how to mend a violation of it.
    pub help: String,
    /// Operations that mend it.
    pub suggestions: Vec<Suggestion>,
    /// Names that exist and are close to the one that does not.
    pub did_you_mean: Vec<String>,
}

impl Diagnostic {
    /// A diagnostic of `rule` about `subjects`, with `values` (a JSON value for each key of the rule) and
    /// `shown` (the same values as the words the template says).
    fn of_rule(
        rule: &Rule,
        subjects: Vec<DiagSubject>,
        values: Vec<(&'static str, Json)>,
        shown: &[(&str, String)],
    ) -> Diagnostic {
        let names: Vec<String> = subjects.iter().map(|s| s.name.clone()).collect();
        Diagnostic {
            code: rule.id.to_string(),
            level: rule.level().word(),
            message: fill(rule.message, &names, shown),
            subjects,
            field: None,
            values: values
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            help: rule.help.to_string(),
            suggestions: Vec::new(),
            did_you_mean: Vec::new(),
        }
    }

    /// A diagnostic of an input rule (`input.*`) about the value `name`.
    fn input(rule: RuleId, kind: Option<&str>, name: &str) -> Diagnostic {
        let Some(r) = rule::rule(rule.as_str()) else {
            return Diagnostic::app_unmapped(name);
        };
        let mut values = vec![("name", json!(name))];
        let mut shown = vec![("name", name.to_string())];
        if let Some(kind) = kind {
            values.push(("kind", json!(kind)));
            shown.push(("kind", kind.to_string()));
        }
        Diagnostic::of_rule(r, Vec::new(), values, &shown)
    }

    /// A refusal that has no rule yet, said as the application said it.
    fn app_unmapped(text: &str) -> Diagnostic {
        match rule::rule(APP_UNMAPPED.as_str()) {
            Some(r) => Diagnostic::of_rule(
                r,
                Vec::new(),
                vec![("error", json!(text))],
                &[("error", text.to_string())],
            ),
            None => Diagnostic {
                code: APP_UNMAPPED.to_string(),
                level: Level::Error.word(),
                message: text.to_string(),
                subjects: Vec::new(),
                field: None,
                values: BTreeMap::new(),
                help: String::new(),
                suggestions: Vec::new(),
                did_you_mean: Vec::new(),
            },
        }
    }

    /// The finding `f` as a diagnostic: its rule is the code, its level and message are its own, what it is about
    /// is the subject, and the help is its rule's. `state` and `stale` are values.
    pub fn of_found(f: &Found) -> Diagnostic {
        let mut values = BTreeMap::new();
        values.insert("state".to_string(), json!(f.state.word()));
        values.insert("stale".to_string(), json!(f.stale));
        Diagnostic {
            code: f.rule.to_string(),
            level: f.level.word(),
            message: f.message.clone(),
            subjects: match (&f.about, f.subject) {
                (Some(name), Some(s)) => vec![DiagSubject {
                    kind: s.kind().word(),
                    name: name.clone(),
                }],
                (Some(name), None) => vec![DiagSubject {
                    kind: "project",
                    name: name.clone(),
                }],
                (None, _) => Vec::new(),
            },
            field: None,
            values,
            help: f.help.clone(),
            suggestions: Vec::new(),
            did_you_mean: Vec::new(),
        }
    }

    /// The diagnostic of an input that could not be read, from the message `serde_json` gave and the `input` that
    /// was read (to say where). Serde has no structure for these, so the messages it has for an unknown variant, an
    /// unknown field, a missing field and a wrong type are read here, once; any other becomes `input.invalid`.
    pub fn of_unreadable(message: &str, input: Option<&Json>) -> Diagnostic {
        let plain = strip_position(message);
        if let Some((name, expected)) = quoted_after(plain, "unknown variant ") {
            let is_op = input
                .and_then(|i| i.get("op"))
                .and_then(Json::as_str)
                .is_some_and(|op| op == name);
            let mut d = Diagnostic::input(
                if is_op {
                    INPUT_UNKNOWN_OP
                } else {
                    INPUT_INVALID
                },
                None,
                &name,
            );
            if !is_op {
                d.message = format!("`{name}` is not one of the values this field takes");
                d.values = BTreeMap::from([("detail".to_string(), json!(plain))]);
            }
            d.did_you_mean = did_you_mean(&name, &expected);
            d.field = input.and_then(|i| locate_field(i, &name));
            return d;
        }
        if let Some((name, expected)) = quoted_after(plain, "unknown field ") {
            let mut d = Diagnostic::input(INPUT_UNKNOWN_FIELD, None, &name);
            d.did_you_mean = did_you_mean(&name, &expected);
            d.field = input.and_then(|i| locate_key(i, &name));
            return d;
        }
        if let Some((name, _)) = quoted_after(plain, "missing field ") {
            return Diagnostic::input(INPUT_MISSING_FIELD, None, &name);
        }
        if plain.starts_with("invalid type") {
            return match rule::rule(INPUT_WRONG_TYPE.as_str()) {
                Some(r) => Diagnostic::of_rule(
                    r,
                    Vec::new(),
                    vec![("detail", json!(plain))],
                    &[("detail", plain.to_string())],
                ),
                None => Diagnostic::app_unmapped(plain),
            };
        }
        match rule::rule(INPUT_INVALID.as_str()) {
            Some(r) => {
                let detail = format!("not an operation: {plain}");
                Diagnostic::of_rule(
                    r,
                    Vec::new(),
                    vec![("detail", json!(detail))],
                    &[("detail", detail.clone())],
                )
            }
            None => Diagnostic::app_unmapped(plain),
        }
    }

    /// The same, with `field` set from where `values.name` stands in `input`, when it was not set and the name is
    /// a value somewhere in the input. The use cases know the name that is wrong, not which field it came from;
    /// the reader of the input does.
    pub fn located_in(mut self, input: &Json) -> Diagnostic {
        if self.field.is_none()
            && let Some(name) = self.values.get("name").and_then(Json::as_str)
        {
            self.field = locate_field(input, name);
        }
        self
    }
}

/// `text` without the position serde_json appends (` at line 1 column 9`).
fn strip_position(text: &str) -> &str {
    match text.rfind(" at line ") {
        Some(k) if text[k..].contains(" column ") => &text[..k],
        _ => text,
    }
}

/// The first backticked word after `lead` in `text`, and the backticked words after `expected` (the names serde
/// lists as the ones that would be right).
fn quoted_after(text: &str, lead: &str) -> Option<(String, Vec<String>)> {
    let rest = text.strip_prefix(lead)?;
    let mut words = rest.split('`').skip(1).step_by(2).map(str::to_string);
    let first = words.next()?;
    let expected = match rest.find("expected") {
        Some(k) => rest[k..]
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect(),
        None => Vec::new(),
    };
    Some((first, expected))
}

/// The message of `template` with its holes filled: `{s0}`, `{s1}` ... the subjects (in backticks), `{subjects}`
/// all of them, `{others}` all but the first, and `{key}` the words of that value. A hole nothing fills is `?`.
fn fill(template: &str, subjects: &[String], shown: &[(&str, String)]) -> String {
    let quote = |s: &str| format!("`{s}`");
    let joined = |from: usize| {
        subjects
            .iter()
            .skip(from)
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let hole = &after[..close];
        let words = match hole {
            "subjects" => joined(0),
            "others" => joined(1),
            _ => match hole.strip_prefix('s').and_then(|n| n.parse::<usize>().ok()) {
                Some(n) => subjects
                    .get(n)
                    .map_or_else(|| "?".to_string(), |s| quote(s)),
                None => shown
                    .iter()
                    .find(|(k, _)| *k == hole)
                    .map_or_else(|| "?".to_string(), |(_, v)| v.clone()),
            },
        };
        out.push_str(&words);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The names in `candidates` that `name` may have meant, nearest first, at most three. A name that differs only
/// in case is the one meant, and alone; otherwise those within a third of the length of `name` (at least one
/// edit), as rustc counts: the edit distance is the number of insertions, deletions and substitutions.
pub fn did_you_mean(name: &str, candidates: &[String]) -> Vec<String> {
    let lower = name.to_lowercase();
    if let Some(same) = candidates
        .iter()
        .find(|c| c.as_str() != name && c.to_lowercase() == lower)
    {
        return vec![same.clone()];
    }
    let limit = name.chars().count().max(3) / 3;
    let mut near: Vec<(usize, &String)> = candidates
        .iter()
        .filter(|c| c.as_str() != name)
        .map(|c| (edit_distance(&lower, &c.to_lowercase()), c))
        .filter(|(d, _)| *d <= limit)
        .collect();
    near.sort();
    near.dedup();
    near.into_iter().take(3).map(|(_, c)| c.clone()).collect()
}

/// The number of single-character edits between `a` and `b`.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if ca == cb {
                diagonal
            } else {
                1 + diagonal.min(above).min(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}

/// Where in `input` a string equal to `needle` stands, as a path (`for`, `puts[0].context`); the first in the
/// order of the object's keys, depth first.
pub fn locate_field(input: &Json, needle: &str) -> Option<String> {
    fn walk(v: &Json, needle: &str, path: &str) -> Option<String> {
        match v {
            Json::String(s) if s == needle && !path.is_empty() => Some(path.to_string()),
            Json::Object(fields) => fields.iter().find_map(|(k, v)| {
                let next = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                walk(v, needle, &next)
            }),
            Json::Array(items) => items
                .iter()
                .enumerate()
                .find_map(|(i, v)| walk(v, needle, &format!("{path}[{i}]"))),
            _ => None,
        }
    }
    walk(input, needle, "")
}

/// Where in `input` a field named `key` stands, as a path.
fn locate_key(input: &Json, key: &str) -> Option<String> {
    fn walk(v: &Json, key: &str, path: &str) -> Option<String> {
        match v {
            Json::Object(fields) => fields.iter().find_map(|(k, v)| {
                let next = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if k == key {
                    Some(next)
                } else {
                    walk(v, key, &next)
                }
            }),
            Json::Array(items) => items
                .iter()
                .enumerate()
                .find_map(|(i, v)| walk(v, key, &format!("{path}[{i}]"))),
            _ => None,
        }
    }
    walk(input, key, "")
}

/// Whether an operation (or a description) still has a hole for a value to be filled in: a string `<..>`.
fn has_hole(v: &Json) -> bool {
    match v {
        Json::String(s) => s.contains('<') && s.contains('>'),
        Json::Array(items) => items.iter().any(has_hole),
        Json::Object(fields) => fields.values().any(has_hole),
        _ => false,
    }
}

/// A suggestion: `op` with how far it may be trusted, which is `placeholder` whenever it has a hole to fill.
fn suggest(op: Json, how: Applicability) -> Suggestion {
    let how = if has_hole(&op) && how != Applicability::Placeholder {
        Applicability::Placeholder
    } else {
        how
    };
    Suggestion {
        op,
        applicability: how.word(),
    }
}

/// The words for the kind of thing a party is.
fn party_kind(p: &Party) -> &'static str {
    match p {
        Party::Target(t) => t.word(),
        Party::ChangeRequest(_) => "change_request",
        Party::PolicyLine(_) => "policy_line",
        Party::Id(_) => "asset",
    }
}

/// A rule, listed: its id, what kind it is, the level it is held to, and what it protects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleCard {
    /// The id.
    pub id: String,
    /// `invariant`, `check` or `input`.
    pub kind: &'static str,
    /// `error` for an invariant or an input rule; the policy's level for a check.
    pub level: &'static str,
    /// What it protects, in a line.
    pub summary: String,
}

/// A rule, explained: what `rules` lists, with the help, the keys of the values a violation carries and an
/// example of an operation that violates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleExplanation {
    /// The id.
    pub id: String,
    /// `invariant`, `check` or `input`.
    pub kind: &'static str,
    /// `error` for an invariant or an input rule; the policy's level for a check.
    pub level: &'static str,
    /// What it protects, in a line.
    pub summary: String,
    /// Why it holds and how to mend a violation.
    pub help: String,
    /// The keys of the values a violation of it carries (a key is added, never removed).
    pub values: Vec<&'static str>,
    /// An operation that violates it, for a rule that is not a check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// Who runs it, for a check: `builtin`, `judged` or `external`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executor: Option<&'static str>,
    /// The moments a check runs at.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<&'static str>,
    /// Whether a policy may not lower a check below `error`.
    pub mandatory: bool,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Every rule: the invariants and the input rules, then the checks with the level the policy gives each.
    pub fn rules(&self) -> Vec<RuleCard> {
        let mut out: Vec<RuleCard> = rule::rules()
            .iter()
            .map(|r| RuleCard {
                id: r.id.to_string(),
                kind: r.kind.word(),
                level: r.level().word(),
                summary: r.summary.to_string(),
            })
            .collect();
        out.extend(self.registry.list().into_iter().map(|c| RuleCard {
            id: c.descriptor.id.to_string(),
            kind: RuleKind::Check.word(),
            level: c.level.word(),
            summary: c.descriptor.summary.to_string(),
        }));
        out
    }

    /// The rule `id`, explained.
    pub fn explain(&self, id: &str) -> Result<RuleExplanation, AppError> {
        if let Some(r) = rule::rule(id) {
            return Ok(RuleExplanation {
                id: r.id.to_string(),
                kind: r.kind.word(),
                level: r.level().word(),
                summary: r.summary.to_string(),
                help: r.help.to_string(),
                values: r.values.to_vec(),
                example: Some(r.example.to_string()),
                executor: None,
                hooks: Vec::new(),
                mandatory: false,
            });
        }
        let found = self
            .registry
            .list()
            .into_iter()
            .find(|c| c.descriptor.id.as_str() == id)
            .ok_or_else(|| AppError::UnknownRule(id.to_string()))?;
        let d = found.descriptor;
        Ok(RuleExplanation {
            id: d.id.to_string(),
            kind: RuleKind::Check.word(),
            level: found.level.word(),
            summary: d.summary.to_string(),
            help: d.help.to_string(),
            values: vec!["state", "stale"],
            example: None,
            executor: Some(d.executor.word()),
            hooks: d.hooks.iter().map(|h| h.word()).collect(),
            mandatory: d.mandatory,
        })
    }

    /// The diagnostics of a refusal, in the order found: a rule broken for each problem (a change set that has
    /// several gives several), an input whose name does not exist with the names that do, a finding that refused a
    /// change as the finding it is. The names come from the store; if it cannot be read, the diagnostic is
    /// still given, with the ids.
    pub async fn diagnose(&self, error: &AppError) -> Vec<Diagnostic> {
        let mut flat = Vec::new();
        flatten(error, &mut flat);
        let mut out = Vec::new();
        for e in flat {
            out.extend(self.diagnose_one(e).await);
        }
        out
    }

    async fn diagnose_one(&self, e: &AppError) -> Vec<Diagnostic> {
        match e {
            AppError::Violated { violations, names } => {
                let mut out = Vec::with_capacity(violations.len());
                for v in violations {
                    out.push(self.diagnostic_of(v, names.clone()).await);
                }
                out
            }
            AppError::Project(pe) => {
                let mut out = Vec::new();
                for v in pe.violations() {
                    out.push(self.diagnostic_of(&v, Names::default()).await);
                }
                out
            }
            AppError::Pin(pe) => vec![self.diagnostic_of(&pe.violation(), Names::default()).await],
            AppError::ChangeRequestOf { id, title, error } => {
                let mut names = Names::default();
                names.insert(Party::ChangeRequest(*id), title.clone());
                vec![self.diagnostic_of(&error.violation(*id), names).await]
            }
            AppError::ChangeRequest(error) => {
                vec![
                    self.diagnostic_of(&Violation::unmapped(error), Names::default())
                        .await,
                ]
            }
            AppError::NoProject => {
                let v = Violation::of(PROJECT_NONE)
                    .with_remedy(Remedy::InitProject, Applicability::Machine);
                vec![self.diagnostic_of(&v, Names::default()).await]
            }
            AppError::TitleTaken(title) => match rule::rule(CR_TITLE_TAKEN.as_str()) {
                Some(r) => vec![Diagnostic::of_rule(
                    r,
                    vec![DiagSubject {
                        kind: "change_request",
                        name: title.clone(),
                    }],
                    Vec::new(),
                    &[],
                )],
                None => vec![Diagnostic::app_unmapped(&e.to_string())],
            },
            AppError::AssetRetired {
                context,
                kind,
                name,
            } => match rule::rule(PROJECT_ASSET_RETIRED.as_str()) {
                Some(r) => vec![Diagnostic::of_rule(
                    r,
                    vec![
                        DiagSubject {
                            kind: "context",
                            name: context.clone(),
                        },
                        DiagSubject {
                            kind,
                            name: name.clone(),
                        },
                    ],
                    vec![("kind", json!(kind))],
                    &[("kind", (*kind).to_string())],
                )],
                None => vec![Diagnostic::app_unmapped(&e.to_string())],
            },
            AppError::UnknownContext(name) => {
                let names = self.context_names().await;
                vec![self.unknown_name("context", name, &names, Vec::new())]
            }
            AppError::UnknownAsset {
                context,
                kind,
                name,
            } => {
                let names = self.asset_names(context, kind).await;
                let scope = vec![DiagSubject {
                    kind: "context",
                    name: context.clone(),
                }];
                vec![self.unknown_name(kind, name, &names, scope)]
            }
            AppError::UnknownItem {
                context,
                interface,
                signature,
            } => {
                let names = self.member_names(context, "interface", interface).await;
                let scope = vec![DiagSubject {
                    kind: "interface",
                    name: interface.clone(),
                }];
                vec![self.unknown_name("item", signature, &names, scope)]
            }
            AppError::UnknownClause {
                context,
                contract,
                label,
            } => {
                let names = self.member_names(context, "contract", contract).await;
                let scope = vec![DiagSubject {
                    kind: "contract",
                    name: contract.clone(),
                }];
                vec![self.unknown_name("clause", label, &names, scope)]
            }
            AppError::UnknownCase {
                context,
                harness,
                name,
            } => {
                let names = self.member_names(context, "harness", harness).await;
                let scope = vec![DiagSubject {
                    kind: "harness",
                    name: harness.clone(),
                }];
                vec![self.unknown_name("case", name, &names, scope)]
            }
            AppError::UnknownChangeRequest(title) => {
                let names = self.change_request_titles().await;
                vec![self.unknown_name("change request", title, &names, Vec::new())]
            }
            AppError::UnknownRule(id) => {
                let mut names: Vec<String> =
                    rule::rules().iter().map(|r| r.id.to_string()).collect();
                names.extend(
                    self.registry
                        .list()
                        .iter()
                        .map(|c| c.descriptor.id.to_string()),
                );
                vec![self.unknown_name("rule", id, &names, Vec::new())]
            }
            AppError::ContextNotGiven => {
                vec![Diagnostic::input(INPUT_MISSING_FIELD, None, "context")]
            }
            AppError::Input(text) => match rule::rule(INPUT_INVALID.as_str()) {
                Some(r) => vec![Diagnostic::of_rule(
                    r,
                    Vec::new(),
                    vec![("detail", json!(text))],
                    &[("detail", text.clone())],
                )],
                None => vec![Diagnostic::app_unmapped(text)],
            },
            AppError::Refused(found) => found.iter().map(Diagnostic::of_found).collect(),
            AppError::ExternalRefused { findings, .. } => {
                findings.iter().map(Diagnostic::of_found).collect()
            }
            AppError::Many(_) => Vec::new(),
            AppError::AlreadyInitialized
            | AppError::NotPinned { .. }
            | AppError::PinCodeNotKept { .. }
            | AppError::Runner(_)
            | AppError::Measurement(_)
            | AppError::Store(_)
            | AppError::RuleVersionMismatch { .. }
            | AppError::NotJudged { .. }
            | AppError::RuleOff(_) => vec![Diagnostic::app_unmapped(&e.to_string())],
        }
    }

    /// The diagnostic of a name that does not exist: its kind and name, the close names that do, and the operation
    /// that would make it.
    fn unknown_name(
        &self,
        kind: &str,
        name: &str,
        existing: &[String],
        scope: Vec<DiagSubject>,
    ) -> Diagnostic {
        let mut d = Diagnostic::input(INPUT_UNKNOWN_NAME, Some(kind), name);
        d.subjects = scope;
        d.did_you_mean = did_you_mean(name, existing);
        let make = match kind {
            "context" => Some((
                json!({"op": "context_add", "name": name}),
                Applicability::Review,
            )),
            "change request" => Some((
                json!({"op": "cr_open", "title": name, "intent": "change_behavior"}),
                Applicability::Review,
            )),
            "contract" | "interface" | "harness" => {
                let context = d.subjects.first().map_or("<context>", |s| s.name.as_str());
                let op = match kind {
                    "contract" => json!({"op": "contract_put", "context": context, "name": name,
                        "title": "<title>", "clauses": ["<clauses>"]}),
                    "interface" => json!({"op": "interface_put", "context": context, "name": name,
                        "source": "<code>", "items": ["<items>"]}),
                    _ => json!({"op": "harness_put", "context": context, "name": name,
                        "source": "<code>", "cases": ["<cases>"]}),
                };
                Some((op, Applicability::Placeholder))
            }
            "item" => Some((
                json!(format!(
                    "put the interface again with the item `{name}` (interface_put)"
                )),
                Applicability::Review,
            )),
            "clause" => Some((
                json!(format!(
                    "put the contract again with the clause `{name}` (contract_put)"
                )),
                Applicability::Review,
            )),
            "case" => Some((
                json!(format!(
                    "put the harness again with the case `{name}` (harness_put)"
                )),
                Applicability::Review,
            )),
            _ => None,
        };
        if let Some((op, how)) = make {
            d.suggestions.push(suggest(op, how));
        }
        d
    }

    /// The names of the Contexts.
    async fn context_names(&self) -> Vec<String> {
        match self.project().await {
            Ok(p) => p.contexts().map(|c| c.name().to_string()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The names of the assets of `kind` in the Context `context`.
    async fn asset_names(&self, context: &str, kind: &str) -> Vec<String> {
        let Ok(p) = self.project().await else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for c in p.contexts().filter(|c| c.name().as_str() == context) {
            let ids: Vec<uuid::Uuid> = match kind {
                "contract" => c.contracts().map(|k| k.id().as_uuid()).collect(),
                "interface" => c.interfaces().map(|k| k.id().as_uuid()).collect(),
                "harness" => c.harnesses().map(|k| k.id().as_uuid()).collect(),
                _ => Vec::new(),
            };
            for id in ids {
                if let Ok(name) = self.name_of(id).await {
                    out.push(name);
                }
            }
        }
        out
    }

    /// What the asset `asset` of `kind` in the Context `context` has in it: the signatures of an Interface, the
    /// labels of a Contract, the names of the Cases of a Harness.
    async fn member_names(&self, context: &str, kind: &str, asset: &str) -> Vec<String> {
        let Ok(p) = self.project().await else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for c in p.contexts().filter(|c| c.name().as_str() == context) {
            match kind {
                "interface" => {
                    for i in c.interfaces() {
                        if self
                            .name_of(i.id().as_uuid())
                            .await
                            .is_ok_and(|n| n == asset)
                        {
                            out.extend(i.items().iter().map(|it| it.signature().to_string()));
                        }
                    }
                }
                "contract" => {
                    for k in c.contracts() {
                        if self
                            .name_of(k.id().as_uuid())
                            .await
                            .is_ok_and(|n| n == asset)
                        {
                            out.extend(
                                k.clauses()
                                    .iter()
                                    .map(|cl| cl.content().label().to_string()),
                            );
                        }
                    }
                }
                _ => {
                    for h in c.harnesses() {
                        if self
                            .name_of(h.id().as_uuid())
                            .await
                            .is_ok_and(|n| n == asset)
                        {
                            out.extend(h.cases().iter().map(|k| k.name().to_string()));
                        }
                    }
                }
            }
        }
        out
    }

    /// The titles of the change requests.
    async fn change_request_titles(&self) -> Vec<String> {
        match self.store.change_requests().await {
            Ok(crs) => crs.iter().map(|c| c.title().to_string()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The words for a party: how the Project describes a target (what a change set makes is in `words`, and
    /// the names it is about to give in `pending`), a change request by its title, a line by its id.
    pub(super) async fn words_of(
        &self,
        p: &Project,
        words: &BTreeMap<Target, String>,
        pending: &[AssetName],
        party: Party,
    ) -> Result<String, AppError> {
        match party {
            Party::Target(t) => match words.get(&t) {
                Some(w) => Ok(w.clone()),
                None => self.describe_in(p, t, pending).await,
            },
            Party::ChangeRequest(id) => Ok(self
                .store
                .change_requests()
                .await?
                .iter()
                .find(|c| c.id() == id)
                .map_or_else(|| id.to_string(), |c| c.title().to_string())),
            Party::PolicyLine(line) => Ok(line.to_string()),
            Party::Id(id) => match p.contexts().find(|c| c.id().as_uuid() == id) {
                Some(c) => Ok(c.name().to_string()),
                None => self.name_in(id, pending).await,
            },
        }
    }

    /// The diagnostic of the violation `v`: its sentence from the rule's template and the names (`names` has the
    /// ones already known; the rest are looked up), its values, and the operations its remedies come to.
    async fn diagnostic_of(&self, v: &Violation, mut names: Names) -> Diagnostic {
        let project = self.project().await.ok();
        let crs = self.store.change_requests().await.unwrap_or_default();
        if let Some(p) = &project {
            for party in v.parties() {
                if names.get(&party).is_none()
                    && let Ok(words) = self.words_of(p, &BTreeMap::new(), &[], party).await
                {
                    names.insert(party, words);
                }
            }
        }
        let say = |party: &Party| -> String {
            names
                .get(party)
                .map_or_else(|| format!("{party:?}"), str::to_string)
        };
        let subjects: Vec<DiagSubject> = v
            .subjects()
            .iter()
            .map(|s| DiagSubject {
                kind: party_kind(s),
                name: say(s),
            })
            .collect();
        let mut values = Vec::new();
        let mut shown = Vec::new();
        for (key, value) in v.values() {
            let (json, words) = match value {
                Value::Count(n) => (json!(n), n.to_string()),
                Value::Word(w) => (json!(w), (*w).to_string()),
                Value::Revision(r) => (json!(r.number()), r.number().to_string()),
                Value::Text(t) => (json!(t), t.clone()),
                Value::Parties(parties) => {
                    let said: Vec<String> = parties.iter().map(&say).collect();
                    let words = said
                        .iter()
                        .map(|s| format!("`{s}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    (json!(said), words)
                }
            };
            values.push((*key, json));
            shown.push((*key, words));
        }
        let mut d = match rule::rule(v.rule().as_str()) {
            Some(r) => Diagnostic::of_rule(r, subjects, values, &shown),
            None => {
                let mut d = Diagnostic::app_unmapped(v.rule().as_str());
                d.subjects = subjects;
                d
            }
        };
        if let Some(p) = &project {
            for (remedy, how) in v.remedies() {
                d.suggestions
                    .extend(self.suggestions_of(p, &crs, &names, v, remedy, *how).await);
            }
        }
        d
    }

    /// The operations a remedy comes to.
    async fn suggestions_of(
        &self,
        p: &Project,
        crs: &[ChangeRequest],
        names: &Names,
        v: &Violation,
        remedy: &Remedy,
        how: Applicability,
    ) -> Vec<Suggestion> {
        let title = |id: &ChangeRequestId| -> String {
            names
                .get(&Party::ChangeRequest(*id))
                .map(str::to_string)
                .or_else(|| {
                    crs.iter()
                        .find(|c| c.id() == *id)
                        .map(|c| c.title().to_string())
                })
                .unwrap_or_else(|| id.to_string())
        };
        let by = json!({"person": "<who>"});
        match remedy {
            Remedy::OpenChangeRequest => vec![suggest(
                json!({"op": "cr_open", "title": "<title>", "intent": "change_behavior"}),
                how,
            )],
            Remedy::ResolveParts { parts } => parts
                .iter()
                .map(|c| {
                    suggest(
                        json!({"op": "cr_resolve", "title": title(c), "resolution": "done",
                            "reason": "<why>", "by": by}),
                        how,
                    )
                })
                .collect(),
            Remedy::AnswerCrux { change_request } => vec![suggest(
                json!({"op": "cr_decide", "title": title(change_request),
                    "choice": "<one of its options>", "basis": {"arbitrary": "<why>"}, "by": by}),
                how,
            )],
            Remedy::ReopenChangeRequest { change_request } => vec![suggest(
                json!({"op": "cr_reopen", "title": title(change_request),
                    "reason": "<why the work goes on>", "by": by}),
                how,
            )],
            Remedy::SupersedeCrux { change_request } => vec![suggest(
                json!({"op": "cr_open", "title": "<title>", "intent": "change_behavior",
                    "crux": {"kind": "ambiguity", "options": [{"choice": "<the new choice>"}],
                        "supersedes": title(change_request)}}),
                how,
            )],
            Remedy::PutPolicyLine { line } => {
                let mut lines: Vec<Json> = p
                    .policy()
                    .lines()
                    .iter()
                    .map(|l| json!({"id": l.id().to_string(), "text": l.text().to_string()}))
                    .collect();
                lines.push(json!({"id": line.to_string(), "text": "<the line>"}));
                vec![suggest(json!({"op": "policy_put", "lines": lines}), how)]
            }
            Remedy::RetireReferrersFirst { referrers } => {
                let mut seen = Vec::new();
                for r in referrers {
                    if let Ok(Some(TargetInput::Case {
                        context, harness, ..
                    })) = self.name_target(p, *r).await
                        && !seen.contains(&(context.clone(), harness.clone()))
                    {
                        seen.push((context, harness));
                    }
                }
                seen.into_iter()
                    .map(|(context, harness)| {
                        suggest(
                            json!({"op": "harness_retire", "context": context, "harness": harness}),
                            how,
                        )
                    })
                    .collect()
            }
            Remedy::ChangeUsesOf { contexts } => contexts
                .iter()
                .filter_map(|c| p.context(*c))
                .map(|c| {
                    let retired = v
                        .subjects()
                        .first()
                        .and_then(|s| names.get(s))
                        .unwrap_or("<the context>");
                    suggest(
                        json!({"op": "context_uses", "context": c.name().to_string(),
                            "uses": [format!("<its uses without {retired}>")]}),
                        how,
                    )
                })
                .collect(),
            Remedy::PutAgainWithoutCitation { case, .. } => {
                match self.name_target(p, *case).await {
                    Ok(Some(TargetInput::Case {
                        context, harness, ..
                    })) => vec![suggest(
                        json!({"op": "harness_put", "context": context, "name": harness,
                            "source": "<its code>", "cases": ["<its cases, without the citation>"]}),
                        how,
                    )],
                    _ => Vec::new(),
                }
            }
            Remedy::KeepCited { cited } => {
                let what = names
                    .get(&Party::Target(*cited))
                    .map_or_else(|| format!("the {}", cited.word()), str::to_string);
                vec![suggest(
                    json!(format!(
                        "keep `{what}`: put it in the same change set, or make the context use the one that owns it"
                    )),
                    how,
                )]
            }
            Remedy::NameOnce { .. } => vec![suggest(
                json!(
                    "name the thing once in the change set: merge the entries, or drop the one that is not wanted"
                ),
                how,
            )],
            Remedy::PutAnotherContract => vec![suggest(
                json!(
                    "put another contract in the same change set as the retirement, or keep this one"
                ),
                how,
            )],
            Remedy::CheckTheInterface { interface } => {
                match self.name_target(p, *interface).await {
                    Ok(Some(TargetInput::Interface { context, interface })) => vec![suggest(
                        json!({"op": "harness_put", "context": context, "name": "<harness>",
                            "source": "<tests>", "cases": [{"name": "<case>", "lines": [1, 1],
                                "items": [{"interface": interface, "signature": "<signature>"}]}]}),
                        how,
                    )],
                    _ => Vec::new(),
                }
            }
            Remedy::InitProject => vec![suggest(json!({"op": "init"}), how)],
            Remedy::AddContext => {
                vec![suggest(json!({"op": "context_add", "name": "<name>"}), how)]
            }
        }
    }
}

/// `error` and, for a `Many`, every problem in it, in order.
fn flatten<'a>(error: &'a AppError, out: &mut Vec<&'a AppError>) {
    match error {
        AppError::Many(all) => all.iter().for_each(|e| flatten(e, out)),
        other => out.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(of: &[&str]) -> Vec<String> {
        of.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_name_one_letter_off_has_the_right_name_suggested() {
        let titles = names(&["Round halves up", "Retire the legacy parser", "Fix pinning"]);
        assert_eq!(
            did_you_mean("Round halfs up", &titles),
            names(&["Round halves up"])
        );
        assert_eq!(did_you_mean("Fix pining", &titles), names(&["Fix pinning"]));
        assert!(did_you_mean("Something else entirely", &titles).is_empty());
    }

    #[test]
    fn a_name_that_differs_only_in_case_is_the_one_meant_and_alone() {
        let all = names(&["Core", "Cord", "core2"]);
        assert_eq!(did_you_mean("core", &all), names(&["Core"]));
    }

    #[test]
    fn the_nearest_come_first_at_most_three_and_the_name_itself_never() {
        let all = names(&["stock", "stack", "stuck", "stic", "stick", "other"]);
        assert_eq!(
            did_you_mean("stick", &all),
            names(&["stack", "stic", "stock"])
        );
        assert!(!did_you_mean("stick", &all).contains(&"stick".to_string()));
    }

    #[test]
    fn a_short_name_is_allowed_one_edit() {
        let all = names(&["id", "ad"]);
        assert_eq!(did_you_mean("ix", &all), names(&["id"]));
        assert!(did_you_mean("zzz", &all).is_empty());
    }

    #[test]
    fn the_edit_distance_counts_insertions_deletions_and_substitutions() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
    }

    #[test]
    fn the_holes_of_a_template_are_filled_with_the_subjects_and_the_values() {
        let subjects = names(&["Parent", "A", "B"]);
        let shown = [("count", "2".to_string())];
        assert_eq!(
            fill(
                "{s0} has {count} part(s) open: {others}; {subjects}; {nothing}",
                &subjects,
                &shown
            ),
            "`Parent` has 2 part(s) open: `A`, `B`; `Parent`, `A`, `B`; ?"
        );
    }

    #[test]
    fn the_field_a_name_came_from_is_found_in_the_input() {
        let op =
            json!({"op": "change_set", "for": "Round halfs up", "puts": [{"context": "core"}]});
        assert_eq!(locate_field(&op, "Round halfs up"), Some("for".to_string()));
        assert_eq!(
            locate_field(&op, "core"),
            Some("puts[0].context".to_string())
        );
        assert_eq!(locate_field(&op, "nowhere"), None);
        assert_eq!(
            locate_key(&op, "context"),
            Some("puts[0].context".to_string())
        );
    }

    #[test]
    fn serde_messages_become_input_rules_with_the_names_that_would_be_right() {
        let op = json!({"op": "cr_opn", "title": "x"});
        let d = Diagnostic::of_unreadable(
            "unknown variant `cr_opn`, expected one of `init`, `cr_open`, `cr_decide` at line 1 column 15",
            Some(&op),
        );
        assert_eq!(d.code, "input.unknown-op");
        assert_eq!(d.did_you_mean, names(&["cr_open"]));
        assert_eq!(d.field.as_deref(), Some("op"));

        let op = json!({"op": "context_add", "name": "a", "usess": []});
        let d = Diagnostic::of_unreadable(
            "unknown field `usess`, expected `name` or `uses` at line 1 column 9",
            Some(&op),
        );
        assert_eq!(d.code, "input.unknown-field");
        assert_eq!(d.did_you_mean, names(&["uses"]));
        assert_eq!(d.field.as_deref(), Some("usess"));

        let d = Diagnostic::of_unreadable("missing field `name` at line 1 column 20", None);
        assert_eq!(d.code, "input.missing-field");
        assert_eq!(d.message, "the operation lacks the field `name`");

        let d = Diagnostic::of_unreadable(
            "invalid type: sequence, expected a string at line 1 column 32",
            None,
        );
        assert_eq!(d.code, "input.wrong-type");

        let d = Diagnostic::of_unreadable("expected value at line 1 column 1", None);
        assert_eq!(d.code, "input.invalid");
        assert_eq!(d.message, "not an operation: expected value");
    }

    #[test]
    fn a_rule_id_names_one_rule_across_the_invariants_the_input_rules_and_the_builtin_checks() {
        // `rules` lists them as one registry and `explain` looks the invariants up first, so an id in two lists
        // would hide the check behind the invariant
        let mut ids: Vec<String> = rule::rules().iter().map(|r| r.id.to_string()).collect();
        ids.extend(
            crate::domain::check::builtin::all()
                .iter()
                .map(|c| c.descriptor().id.to_string()),
        );
        let distinct: std::collections::BTreeSet<&String> = ids.iter().collect();
        assert_eq!(distinct.len(), ids.len(), "an id is used twice: {ids:?}");
    }

    #[test]
    fn a_suggestion_with_a_hole_is_a_placeholder_whatever_it_was_given() {
        assert_eq!(
            suggest(json!({"op": "x", "a": ["<fill>"]}), Applicability::Machine).applicability,
            "placeholder"
        );
        assert_eq!(
            suggest(json!({"op": "init"}), Applicability::Machine).applicability,
            "machine"
        );
        assert_eq!(
            suggest(
                json!({"op": "cr_open", "title": "T"}),
                Applicability::Review
            )
            .applicability,
            "review"
        );
    }
}
