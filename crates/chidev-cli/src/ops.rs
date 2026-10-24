//! The operations `chidev exec` reads, one JSON object per line, and the results it prints. This is where the JSON
//! form is turned into the inputs of the use cases and back; nothing past it sees JSON.

use std::io::{self, BufRead, Write};

use chidev_core::app::ports::external::ExternalExecutor;
use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, CruxInput, CruxKind,
    DecisionInput, FindingInput, Found, Intent, ItemInput, ItemRef, ModelView, OptionInput,
    ParamValue, PinState, ReasonedInput, ReportInput, ReportedFinding, RuleStanding, RuleToJudge,
    SubjectInput,
};
use chidev_core::domain::check::{Hook, Level};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One operation. `op` names it; the other fields are its arguments.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    /// Makes the project.
    Init,
    /// Adds a Context.
    ContextAdd {
        name: String,
        #[serde(default)]
        uses: Vec<String>,
    },
    /// Sets the Contexts a Context uses.
    ContextUses { context: String, uses: Vec<String> },
    /// Bases a Context on its Contracts.
    BaseOnContract { context: String },
    /// Makes or revises a Contract.
    ContractPut {
        context: String,
        name: String,
        title: String,
        clauses: Vec<ClauseDto>,
    },
    /// Makes or revises an Interface.
    InterfacePut {
        context: String,
        name: String,
        source: String,
        items: Vec<ItemDto>,
    },
    /// Makes or revises a Harness.
    HarnessPut {
        context: String,
        name: String,
        source: String,
        cases: Vec<CaseDto>,
    },
    /// Pins the code of an Interface.
    Pin {
        context: String,
        interface: String,
        code: String,
    },
    /// Opens a change request.
    CrOpen(Box<OpenDto>),
    /// Decides the Crux of a change request: exactly one of `choice`, `wording` and `unspecified`.
    CrDecide {
        title: String,
        #[serde(default)]
        choice: Option<String>,
        #[serde(default)]
        wording: Option<String>,
        #[serde(default)]
        unspecified: Option<Vec<String>>,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Closes the record of a change request whose change is applied.
    CrResolve { title: String },
    /// Where the project stands.
    Status,
    /// Runs every check on the project as it is; nothing is kept.
    Check,
    /// Explicitly executes external check-hook programs and records this run.
    CheckRun,
    /// Lists every check with the level it runs at.
    CheckList,
    /// The judged rules to judge (those of `hook`, or the one `rule`; all when neither is given), with a view of
    /// the model to judge them on. The agent that drives chidev judges; chidev keeps the results.
    CheckRules {
        #[serde(default)]
        hook: Option<String>,
        #[serde(default)]
        rule: Option<String>,
    },
    /// What an agent judged of one rule: replaces every finding the rule had.
    FindingReport(Box<ReportDto>),
}

/// The fields of `finding_report`.
#[derive(Debug, Deserialize)]
struct ReportDto {
    rule: String,
    /// The rule digest returned by check_rules, echoed back. Required: a missing or null one is refused.
    rule_digest: String,
    /// The agent that judged.
    judge: String,
    /// The store revision of the view it judged from (`store_revision` of `check_rules`).
    read_at: u64,
    /// Every finding of the rule; empty: it found nothing.
    findings: Vec<ReportedFindingDto>,
}

/// A finding in a report.
#[derive(Debug, Deserialize)]
struct ReportedFindingDto {
    /// What it is about; none: the Project as a whole.
    #[serde(default)]
    subject: Option<SubjectDto>,
    message: String,
    /// `note`, `warning` or `error`, not above the rule's level; none: the rule's.
    #[serde(default)]
    level: Option<String>,
}

/// What a finding is about, tagged by its kind: `{ "clause": { ... } }`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum SubjectDto {
    Context(String),
    Interface {
        context: String,
        interface: String,
    },
    Item(ItemRefDto),
    Clause(ClauseRefDto),
    Case {
        context: String,
        harness: String,
        name: String,
    },
    ChangeRequest(String),
}

impl From<SubjectDto> for SubjectInput {
    fn from(s: SubjectDto) -> SubjectInput {
        match s {
            SubjectDto::Context(name) => SubjectInput::Context(name),
            SubjectDto::Interface { context, interface } => {
                SubjectInput::Interface { context, interface }
            }
            SubjectDto::Item(r) => SubjectInput::Item(ItemRef::from(r)),
            SubjectDto::Clause(r) => SubjectInput::Clause(ClauseRef::from(r)),
            SubjectDto::Case {
                context,
                harness,
                name,
            } => SubjectInput::Case {
                context,
                harness,
                name,
            },
            SubjectDto::ChangeRequest(title) => SubjectInput::ChangeRequest(title),
        }
    }
}

/// A level a person wrote.
fn level_of(word: &str) -> Result<Level, AppError> {
    Level::from_word(word).ok_or_else(|| {
        AppError::Input(format!(
            "the level `{word}` is not one; it is off, note, warning or error"
        ))
    })
}

impl TryFrom<ReportDto> for ReportInput {
    type Error = AppError;

    fn try_from(r: ReportDto) -> Result<ReportInput, AppError> {
        Ok(ReportInput {
            rule: r.rule,
            rule_digest: r.rule_digest,
            judge: r.judge,
            read_at: r.read_at,
            findings: r
                .findings
                .into_iter()
                .map(|f| {
                    Ok(ReportedFinding {
                        subject: f.subject.map(SubjectInput::from),
                        message: f.message,
                        level: f.level.as_deref().map(level_of).transpose()?,
                    })
                })
                .collect::<Result<_, AppError>>()?,
        })
    }
}

/// The fields of `cr_open`, boxed: they are far larger than any other operation's.
#[derive(Debug, Deserialize)]
struct OpenDto {
    title: String,
    #[serde(default)]
    situation: Option<String>,
    intent: IntentDto,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    items: Vec<ItemRefDto>,
    #[serde(default)]
    clauses: Vec<ClauseRefDto>,
    /// The question for a person; none: the machine resolves it.
    #[serde(default)]
    crux: Option<CruxDto>,
    /// The findings it deals with.
    #[serde(default)]
    addresses: Vec<AddressDto>,
}

/// A finding a change request addresses. New and legacy input forms normalize at the CLI boundary.
#[derive(Debug, Deserialize)]
#[serde(try_from = "AddressWire")]
struct AddressDto {
    rule: String,
    subject: Option<SubjectDto>,
}

/// Preserve presence separately from null so mixed selectors and duplicate null fields are refused.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddressWire {
    rule: String,
    #[serde(default, deserialize_with = "present")]
    subject: Option<Option<SubjectDto>>,
    #[serde(default, deserialize_with = "present")]
    item: Option<Option<ItemRefDto>>,
    #[serde(default, deserialize_with = "present")]
    clause: Option<Option<ClauseRefDto>>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl TryFrom<AddressWire> for AddressDto {
    type Error = String;

    fn try_from(a: AddressWire) -> Result<Self, Self::Error> {
        let selectors = usize::from(a.subject.is_some())
            + usize::from(a.item.is_some())
            + usize::from(a.clause.is_some());
        if selectors > 1 {
            return Err(
                "an address accepts only one of subject, item and clause, including null fields"
                    .into(),
            );
        }
        let subject = a
            .subject
            .flatten()
            .or_else(|| a.item.flatten().map(SubjectDto::Item))
            .or_else(|| a.clause.flatten().map(SubjectDto::Clause));
        Ok(Self {
            rule: a.rule,
            subject,
        })
    }
}

impl From<AddressDto> for FindingInput {
    fn from(a: AddressDto) -> FindingInput {
        FindingInput {
            rule: a.rule,
            subject: a.subject.map(SubjectInput::from),
        }
    }
}

/// A finding in a result.
#[derive(Debug, Serialize)]
struct FindingOut<'a> {
    rule: &'a str,
    level: &'static str,
    /// `open`, `addressed` or `decided`.
    state: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    about: Option<&'a str>,
    /// A saved finding whose input or judged rule changed, or whose judged rule version is unknown.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stale: bool,
    /// Who judged it and on what; only a judged finding has it.
    #[serde(skip_serializing_if = "Option::is_none")]
    judged: Option<JudgmentOut<'a>>,
}

/// Who judged a finding and on what.
#[derive(Debug, Serialize)]
struct JudgmentOut<'a> {
    judge: &'a str,
    /// The rule version judged; null only for an old database row (shown stale).
    rule_digest: Option<String>,
    read_at: u64,
    on_older_revision: bool,
}

fn findings_out(found: &[Found]) -> Vec<FindingOut<'_>> {
    found
        .iter()
        .map(|f| FindingOut {
            rule: f.rule.as_str(),
            level: f.level.word(),
            state: f.state.word(),
            message: &f.message,
            about: f.about.as_deref(),
            stale: f.stale,
            judged: f.judgment.as_ref().map(|j| JudgmentOut {
                judge: &j.judge,
                rule_digest: j.rule_digest.map(|digest| digest.to_string()),
                read_at: j.read_at.0,
                on_older_revision: j.on_older_revision,
            }),
        })
        .collect()
}

/// The result of `check`.
#[derive(Debug, Serialize)]
struct CheckOut<'a> {
    op: &'static str,
    findings: Vec<FindingOut<'a>>,
}

/// A parameter's value in a listing.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ParamOut {
    Bool(bool),
    Int(i64),
    Str(String),
}

/// A check in the result of `check_list`.
#[derive(Debug, Serialize)]
struct CheckEntryOut {
    id: String,
    summary: String,
    level: &'static str,
    default_level: &'static str,
    mandatory: bool,
    executor: &'static str,
    hooks: Vec<&'static str>,
    params: std::collections::BTreeMap<String, ParamOut>,
}

/// The result of `check_list`: the checks, and a fingerprint of the policy they run under.
#[derive(Debug, Serialize)]
struct CheckListOut {
    op: &'static str,
    policy: String,
    checks: Vec<CheckEntryOut>,
}

/// A judged rule in the result of `check_rules`: everything an agent needs to judge it.
#[derive(Debug, Serialize)]
struct RuleOut<'a> {
    id: &'a str,
    /// The current parsed rule definition, echoed back by finding_report.
    rule_digest: String,
    summary: &'a str,
    /// The kind of thing it judges: a finding names a subject of this kind (`project`: none).
    subject: &'static str,
    hooks: Vec<&'static str>,
    /// The level the policy holds it to; a finding is at it or below.
    level: &'static str,
    default_level: &'static str,
    question: &'a str,
    #[serde(rename = "true")]
    true_when: &'a str,
    #[serde(rename = "false")]
    false_when: &'a str,
    fix: &'a str,
    when: &'a [String],
    /// `never`, `current`, or `stale` (the rule/model changed, or the reported rule version is unknown).
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    judged: Option<JudgmentOut<'a>>,
}

impl<'a> RuleOut<'a> {
    fn of(r: &'a RuleToJudge) -> RuleOut<'a> {
        let d = r.rule.descriptor();
        RuleOut {
            id: d.id.as_str(),
            rule_digest: r.rule_digest.to_string(),
            summary: &d.summary,
            subject: r.rule.subject().word(),
            hooks: d.hooks.iter().map(|h| h.word()).collect(),
            level: r.level.word(),
            default_level: d.default_level.word(),
            question: r.rule.question(),
            true_when: r.rule.true_when(),
            false_when: r.rule.false_when(),
            fix: r.rule.fix(),
            when: r.rule.when(),
            state: r.state.word(),
            judged: r.judgment.as_ref().map(|j| JudgmentOut {
                judge: &j.judge,
                rule_digest: j.rule_digest.map(|digest| digest.to_string()),
                read_at: j.read_at.0,
                on_older_revision: j.on_older_revision,
            }),
        }
    }
}

/// The result of `check_rules`: the rules, and the model they are judged on (read at `store_revision`).
#[derive(Debug, Serialize)]
struct CheckRulesOut<'a> {
    op: &'static str,
    store_revision: u64,
    rules: Vec<RuleOut<'a>>,
    model: &'a ModelView,
}

/// The result of `finding_report`.
#[derive(Debug, Serialize)]
struct ReportOut<'a> {
    op: &'static str,
    rule: &'a str,
    rule_digest: String,
    state: &'static str,
    findings: usize,
    /// How many findings the report replaced.
    replaced: usize,
    on_older_revision: bool,
    store_revision: u64,
}

/// A judged rule that is never reported or stale, in the result of `status`.
#[derive(Debug, Serialize)]
struct StandingOut<'a> {
    rule: &'a str,
    summary: &'a str,
    subject: &'static str,
    /// `never` or `stale`.
    state: &'static str,
}

fn standing_out(standing: &[RuleStanding]) -> Vec<StandingOut<'_>> {
    standing
        .iter()
        .map(|s| StandingOut {
            rule: s.rule.as_str(),
            summary: &s.summary,
            subject: s.subject.word(),
            state: s.state.word(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IntentDto {
    ChangeBehavior,
    KeepBehavior,
}

impl From<IntentDto> for Intent {
    fn from(i: IntentDto) -> Intent {
        match i {
            IntentDto::ChangeBehavior => Intent::ChangeBehavior,
            IntentDto::KeepBehavior => Intent::KeepBehavior,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CruxDto {
    kind: KindDto,
    options: Vec<OptionDto>,
    #[serde(default)]
    default: Option<ReasonedDto>,
    #[serde(default)]
    recommended: Option<ReasonedDto>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KindDto {
    Ambiguity,
    Conflict,
    Model,
    Disagreement,
}

impl From<KindDto> for CruxKind {
    fn from(k: KindDto) -> CruxKind {
        match k {
            KindDto::Ambiguity => CruxKind::Ambiguity,
            KindDto::Conflict => CruxKind::Conflict,
            KindDto::Model => CruxKind::Model,
            KindDto::Disagreement => CruxKind::Disagreement,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OptionDto {
    choice: String,
    #[serde(default)]
    change: Option<String>,
    #[serde(default)]
    consequence: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReasonedDto {
    choice: String,
    #[serde(default)]
    reason: Option<String>,
}

impl From<ReasonedDto> for ReasonedInput {
    fn from(r: ReasonedDto) -> ReasonedInput {
        ReasonedInput {
            choice: r.choice,
            reason: r.reason,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClauseDto {
    label: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ItemDto {
    signature: String,
    lines: [usize; 2],
    #[serde(default)]
    replaces: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemRefDto {
    #[serde(default)]
    context: Option<String>,
    interface: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClauseRefDto {
    #[serde(default)]
    context: Option<String>,
    contract: String,
    label: String,
}

#[derive(Debug, Deserialize)]
struct CaseDto {
    name: String,
    lines: [usize; 2],
    #[serde(default)]
    items: Vec<ItemRefDto>,
    #[serde(default)]
    clauses: Vec<ClauseRefDto>,
}

impl From<ItemRefDto> for ItemRef {
    fn from(d: ItemRefDto) -> ItemRef {
        ItemRef {
            context: d.context,
            interface: d.interface,
            signature: d.signature,
        }
    }
}

impl From<ClauseRefDto> for ClauseRef {
    fn from(d: ClauseRefDto) -> ClauseRef {
        ClauseRef {
            context: d.context,
            contract: d.contract,
            label: d.label,
        }
    }
}

/// Runs every line of `input` against `app` and writes one result line per operation to `out`. A blank line is
/// skipped. A line that fails (bad JSON or a refused operation) gets a result with `"ok": false` and the run goes
/// on. Returns whether every operation succeeded; an error is only a failure to read or write.
pub async fn run<S, G, E>(
    app: &mut App<S, G, E>,
    input: impl BufRead,
    out: &mut impl Write,
) -> io::Result<bool>
where
    S: Store,
    G: IdSource,
    E: ExternalExecutor,
{
    let mut all_ok = true;
    for (k, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let result = match serde_json::from_str::<Op>(&line) {
            Ok(op) => apply(app, op).await,
            Err(e) => Err(AppError::Input(format!("not an operation: {e}"))),
        };
        let mut result = match result {
            Ok(Value::Object(mut fields)) => {
                fields.insert("ok".to_string(), json!(true));
                Value::Object(fields)
            }
            Ok(other) => json!({ "ok": true, "result": other }),
            Err(AppError::ExternalRefused { findings, external }) => {
                all_ok = false;
                json!({ "ok": false, "error": "refused by external gate(s)", "findings": findings_out(&findings), "external": external })
            }
            Err(AppError::Refused(found)) => {
                all_ok = false;
                json!({ "ok": false, "error": AppError::Refused(found.clone()).to_string(), "findings": findings_out(&found) })
            }
            Err(error) => {
                all_ok = false;
                json!({ "ok": false, "error": error.to_string() })
            }
        };
        result["line"] = json!(k + 1);
        writeln!(out, "{result}")?;
    }
    Ok(all_ok)
}

/// A typed result as the JSON object the run prints.
fn to_value(out: &impl Serialize) -> Result<Value, AppError> {
    serde_json::to_value(out).map_err(|e| AppError::Input(format!("cannot write the result: {e}")))
}

async fn apply<S, G, E>(app: &mut App<S, G, E>, op: Op) -> Result<Value, AppError>
where
    S: Store,
    G: IdSource,
    E: ExternalExecutor,
{
    Ok(match op {
        Op::CheckRun => {
            let result = app.check_run().await?;
            json!({ "op": "check_run", "findings": findings_out(&result.findings), "external": result.external })
        }
        Op::Init => json!({ "op": "init", "project": app.init().await?.to_string() }),
        Op::ContextAdd { name, uses } => {
            json!({ "op": "context_add", "context": app.add_context(&name, &uses).await?.to_string() })
        }
        Op::ContextUses { context, uses } => {
            app.set_uses(&context, &uses).await?;
            json!({ "op": "context_uses" })
        }
        Op::BaseOnContract { context } => {
            app.base_on_contract(&context).await?;
            json!({ "op": "base_on_contract" })
        }
        Op::ContractPut {
            context,
            name,
            title,
            clauses,
        } => {
            let clauses: Vec<ClauseInput> = clauses
                .into_iter()
                .map(|c| ClauseInput {
                    label: c.label,
                    text: c.text,
                })
                .collect();
            let done = app.put_contract(&context, &name, &title, &clauses).await?;
            json!({ "op": "contract_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
        }
        Op::InterfacePut {
            context,
            name,
            source,
            items,
        } => {
            let items: Vec<ItemInput> = items
                .into_iter()
                .map(|i| ItemInput {
                    signature: i.signature,
                    lines: (i.lines[0], i.lines[1]),
                    replaces: i.replaces,
                })
                .collect();
            let done = app.put_interface(&context, &name, &source, &items).await?;
            json!({ "op": "interface_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
        }
        Op::HarnessPut {
            context,
            name,
            source,
            cases,
        } => {
            let cases: Vec<CaseInput> = cases
                .into_iter()
                .map(|c| CaseInput {
                    name: c.name,
                    lines: (c.lines[0], c.lines[1]),
                    items: c.items.into_iter().map(ItemRef::from).collect(),
                    clauses: c.clauses.into_iter().map(ClauseRef::from).collect(),
                })
                .collect();
            let done = app.put_harness(&context, &name, &source, &cases).await?;
            json!({ "op": "harness_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
        }
        Op::Pin {
            context,
            interface,
            code,
        } => {
            let done = app.pin(&context, &interface, &code).await?;
            json!({ "op": "pin", "pin": done.value.to_string(), "findings": findings_out(&done.findings) })
        }
        Op::CrOpen(open) => {
            let OpenDto {
                title,
                situation,
                intent,
                parent,
                issuer,
                items,
                clauses,
                crux,
                addresses,
            } = *open;
            let crux = crux.map(|q| CruxInput {
                kind: q.kind.into(),
                options: q
                    .options
                    .into_iter()
                    .map(|o| OptionInput {
                        choice: o.choice,
                        change: o.change,
                        consequence: o.consequence,
                    })
                    .collect(),
                default: q.default.map(ReasonedInput::from),
                recommended: q.recommended.map(ReasonedInput::from),
            });
            let input = ChangeRequestInput {
                title,
                situation,
                intent: intent.into(),
                parent,
                issuer,
                items: items.into_iter().map(ItemRef::from).collect(),
                clauses: clauses.into_iter().map(ClauseRef::from).collect(),
                crux,
                addresses: addresses.into_iter().map(FindingInput::from).collect(),
            };
            let id = app.open_change_request(&input).await?;
            json!({ "op": "cr_open", "cr": id.to_string() })
        }
        Op::CrDecide {
            title,
            choice,
            wording,
            unspecified,
            reason,
        } => {
            let decision = match (choice, wording, unspecified) {
                (Some(choice), None, None) => {
                    DecisionInput::Chosen(ReasonedInput { choice, reason })
                }
                (None, Some(wording), None) => DecisionInput::Worded { wording, reason },
                (None, None, Some(keep)) => DecisionInput::Unspecified { keep },
                _ => {
                    return Err(AppError::Input(
                        "give exactly one of `choice`, `wording` and `unspecified`".to_string(),
                    ));
                }
            };
            app.decide_change_request(&title, &decision).await?;
            json!({ "op": "cr_decide" })
        }
        Op::CrResolve { title } => {
            let done = app.resolve_change_request(&title).await?;
            json!({ "op": "cr_resolve", "findings": findings_out(&done.findings) })
        }
        Op::Status => {
            let s = app.status().await?;
            let pins: Vec<Value> = s
                .pins
                .iter()
                .map(|p| {
                    let (state, why) = match &p.state {
                        PinState::Unpinned => ("unpinned", Vec::new()),
                        PinState::Fresh => ("fresh", Vec::new()),
                        PinState::Gone => ("gone", Vec::new()),
                        PinState::Stale(why) => ("stale", why.clone()),
                    };
                    json!({ "context": p.context, "interface": p.interface, "state": state, "why": why })
                })
                .collect();
            let stale: Vec<Value> = s
                .stale_cites
                .iter()
                .map(|c| json!({ "case": c.case, "target": c.target, "cited": c.cited, "now": c.now }))
                .collect();
            let crs: Vec<Value> = s
                .change_requests
                .iter()
                .map(|c| {
                    json!({
                        "title": c.title, "intent": c.intent, "parent": c.parent, "issuer": c.issuer,
                        "crux": c.crux, "state": c.state, "decision": c.decision,
                    })
                })
                .collect();
            let open: Vec<Value> = s
                .unapplied
                .iter()
                .map(|u| json!({ "cr": u.cr, "decision": u.decision, "unchanged": u.unchanged, "gone": u.gone }))
                .collect();
            let judged = to_value(&standing_out(&s.judged))?;
            let external: Vec<Value> = s
                .external
                .iter()
                .map(|r| json!({"rule":r.rule.as_str(),"state":r.state.word()}))
                .collect();
            json!({ "op": "status", "pins": pins, "stale_cites": stale, "change_requests": crs, "unapplied": open, "findings": findings_out(&s.findings), "judged": judged, "external": external })
        }
        Op::Check => {
            let found = app.check().await?;
            to_value(&CheckOut {
                op: "check",
                findings: findings_out(&found),
            })?
        }
        Op::CheckRules { hook, rule } => {
            let hook = hook
                .as_deref()
                .map(|w| {
                    Hook::from_word(w).ok_or_else(|| {
                        AppError::Input(format!(
                            "the hook `{w}` is not one; it is put, pin, resolve or check"
                        ))
                    })
                })
                .transpose()?;
            let view = app.check_rules(hook, rule.as_deref()).await?;
            to_value(&CheckRulesOut {
                op: "check_rules",
                store_revision: view.store_revision.0,
                rules: view.rules.iter().map(RuleOut::of).collect(),
                model: &view.model,
            })?
        }
        Op::FindingReport(report) => {
            let done = app.finding_report(&ReportInput::try_from(*report)?).await?;
            to_value(&ReportOut {
                op: "finding_report",
                rule: done.rule.as_str(),
                rule_digest: done.rule_digest.to_string(),
                state: done.state.word(),
                findings: done.findings,
                replaced: done.replaced,
                on_older_revision: done.on_older_revision,
                store_revision: done.store_revision.0,
            })?
        }
        Op::CheckList => {
            let registry = app.registry();
            let checks = registry
                .list()
                .into_iter()
                .map(|c| CheckEntryOut {
                    id: c.descriptor.id.to_string(),
                    summary: c.descriptor.summary.to_string(),
                    level: c.level.word(),
                    default_level: c.descriptor.default_level.word(),
                    mandatory: c.descriptor.mandatory,
                    executor: c.descriptor.executor.word(),
                    hooks: c.descriptor.hooks.iter().map(|h| h.word()).collect(),
                    params: c
                        .params
                        .iter()
                        .map(|(name, v)| {
                            let out = match v {
                                ParamValue::Bool(b) => ParamOut::Bool(*b),
                                ParamValue::Int(i) => ParamOut::Int(*i),
                                ParamValue::Str(s) => ParamOut::Str(s.clone()),
                            };
                            (name.to_string(), out)
                        })
                        .collect(),
                })
                .collect();
            to_value(&CheckListOut {
                op: "check_list",
                policy: registry.policy().hash().to_string(),
                checks,
            })?
        }
    })
}
