//! The operations `chidev exec` reads, one JSON object per line, and the results it prints. This is where the JSON
//! form is turned into the inputs of the use cases and back; nothing past it sees JSON.

use std::io::{self, BufRead, Write};

use chidev_core::app::ports::external::ExternalExecutor;
use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, AssumptionInput, BasisInput, BasisStatus, CaseInput, ChangeRequestInput,
    ClauseInput, ClauseRef, CruxInput, CruxKind, DecisionInput, DecisionMakerInput, EvidenceInput,
    EvidenceStatus, ExpectInput, ExpectKind, FindingInput, Found, Importance, Intent, ItemInput,
    ItemRef, ModelView, OpenedExpect, OptionInput, ParamValue, PinState, PolicyLineInput,
    ReasonedInput, RecommendedInput, ReportInput, ReportedFinding, RuleStanding, RuleToJudge,
    SubjectInput, TargetInput,
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
    /// Replaces the decision Policy (the numbered lines `P1`, `P2`, ... that settle a choice without asking a
    /// decision maker) with `lines`, whole.
    PolicyPut { lines: Vec<PolicyLineDto> },
    /// Decides the Crux of a change request, once: exactly one of `choice`, `wording` and `unspecified`, the `basis`
    /// it rests on (a Policy line or an arbitrary reason), and who decides (`by`).
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
        basis: BasisDto,
        by: MakerDto,
    },
    /// Appends an Assumption to a change request that is not resolved.
    CrAssume {
        title: String,
        statement: String,
        reason: String,
        #[serde(default)]
        evidence: Vec<EvidenceDto>,
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
    /// The targets it expects to change.
    #[serde(default)]
    change: Vec<RefDto>,
    /// The targets it expects to go.
    #[serde(default)]
    remove: Vec<RefDto>,
    /// The targets it expects to stay as they are.
    #[serde(default)]
    keep: Vec<RefDto>,
    /// The choice for a decision maker, besides the expectations; none: nothing to decide.
    #[serde(default)]
    crux: Option<CruxDto>,
    /// The interpretations settled from the repository and general knowledge while raising it.
    #[serde(default)]
    assumptions: Vec<AssumptionDto>,
    /// The findings it deals with.
    #[serde(default)]
    addresses: Vec<AddressDto>,
}

/// An Assumption: what is assumed, why it holds, and what it stands on.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssumptionDto {
    statement: String,
    reason: String,
    #[serde(default)]
    evidence: Vec<EvidenceDto>,
}

/// What an Assumption stands on: a place in the repository, or general knowledge.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum EvidenceDto {
    Repo {
        path: String,
        #[serde(default)]
        lines: Option<(usize, usize)>,
    },
    Knowledge {
        reference: String,
    },
}

impl From<EvidenceDto> for EvidenceInput {
    fn from(e: EvidenceDto) -> EvidenceInput {
        match e {
            EvidenceDto::Repo { path, lines } => EvidenceInput::Repo { path, lines },
            EvidenceDto::Knowledge { reference } => EvidenceInput::Knowledge { reference },
        }
    }
}

impl From<AssumptionDto> for AssumptionInput {
    fn from(a: AssumptionDto) -> AssumptionInput {
        AssumptionInput {
            statement: a.statement,
            reason: a.reason,
            evidence: a.evidence.into_iter().map(EvidenceInput::from).collect(),
        }
    }
}

/// What a decision rests on: `{"policy": "P1"}` or `{"arbitrary": "<reason>"}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BasisDto {
    Policy(String),
    Arbitrary(String),
}

impl From<BasisDto> for BasisInput {
    fn from(b: BasisDto) -> BasisInput {
        match b {
            BasisDto::Policy(line) => BasisInput::Policy(line),
            BasisDto::Arbitrary(reason) => BasisInput::Arbitrary(reason),
        }
    }
}

/// Who decides: `{"role": "<name>"}` or `{"person": "<name>"}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MakerDto {
    Role(String),
    Person(String),
}

impl From<MakerDto> for DecisionMakerInput {
    fn from(m: MakerDto) -> DecisionMakerInput {
        match m {
            MakerDto::Role(name) => DecisionMakerInput::Role(name),
            MakerDto::Person(name) => DecisionMakerInput::Person(name),
        }
    }
}

/// One line of the decision Policy.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyLineDto {
    id: String,
    text: String,
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

/// A target by name, in the shape a reference to it is written in: `{"project": true}`, `{"context": "core"}`,
/// `{"clause": {"context": ..., "contract": ..., "label": ...}}`, and so on.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum TargetOut {
    Project(bool),
    Context(String),
    Contract {
        context: String,
        contract: String,
    },
    Interface {
        context: String,
        interface: String,
    },
    Harness {
        context: String,
        harness: String,
    },
    Item {
        context: Option<String>,
        interface: String,
        signature: String,
    },
    Clause {
        context: Option<String>,
        contract: String,
        label: String,
    },
    Case {
        context: String,
        harness: String,
        name: String,
    },
}

impl From<TargetInput> for TargetOut {
    fn from(t: TargetInput) -> TargetOut {
        match t {
            TargetInput::Project => TargetOut::Project(true),
            TargetInput::Context(name) => TargetOut::Context(name),
            TargetInput::Contract { context, contract } => {
                TargetOut::Contract { context, contract }
            }
            TargetInput::Interface { context, interface } => {
                TargetOut::Interface { context, interface }
            }
            TargetInput::Harness { context, harness } => TargetOut::Harness { context, harness },
            TargetInput::Item(r) => TargetOut::Item {
                context: r.context,
                interface: r.interface,
                signature: r.signature,
            },
            TargetInput::Clause(r) => TargetOut::Clause {
                context: r.context,
                contract: r.contract,
                label: r.label,
            },
            TargetInput::Case {
                context,
                harness,
                name,
            } => TargetOut::Case {
                context,
                harness,
                name,
            },
        }
    }
}

/// An expectation a change request recorded, in the result of `cr_open`.
#[derive(Debug, Serialize)]
struct OpenedExpectOut {
    /// `change`, `remove` or `keep`.
    kind: &'static str,
    target: TargetOut,
    /// The revision the target had when the change request was opened.
    recorded: u64,
}

impl From<OpenedExpect> for OpenedExpectOut {
    fn from(e: OpenedExpect) -> OpenedExpectOut {
        OpenedExpectOut {
            kind: e.kind.word(),
            target: e.target.into(),
            recorded: e.recorded.number(),
        }
    }
}

/// The result of `cr_open`.
#[derive(Debug, Serialize)]
struct CrOpenOut {
    op: &'static str,
    cr: String,
    expects: Vec<OpenedExpectOut>,
}

/// A Pin in the result of `status`.
#[derive(Debug, Serialize)]
struct PinOut<'a> {
    context: &'a str,
    interface: &'a str,
    state: &'static str,
    why: &'a [String],
}

/// A stale citation in the result of `status`.
#[derive(Debug, Serialize)]
struct StaleCiteOut<'a> {
    case: &'a str,
    target: &'a str,
    cited: u64,
    now: u64,
}

/// What a decision rests on, in the result of `status`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum BasisOut<'a> {
    Policy { line: &'a str, policy_revision: u64 },
    Arbitrary { reason: &'a str },
}

/// Who decided, in the result of `status`.
#[derive(Debug, Serialize)]
struct MakerOut<'a> {
    kind: &'static str,
    name: &'a str,
}

/// The answer to a Crux, in the result of `status`: the decision, what it rests on, who gave it, and when
/// (milliseconds since the epoch).
#[derive(Debug, Serialize)]
struct AnswerOut<'a> {
    decision: &'a str,
    basis: BasisOut<'a>,
    by: MakerOut<'a>,
    at: u64,
}

/// What an Assumption stands on, in the result of `status`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceOut<'a> {
    Repo {
        path: &'a str,
        lines: Option<(usize, usize)>,
    },
    Knowledge {
        reference: &'a str,
    },
}

/// An Assumption of a change request, in the result of `status`.
#[derive(Debug, Serialize)]
struct AssumptionOut<'a> {
    statement: &'a str,
    reason: &'a str,
    evidence: Vec<EvidenceOut<'a>>,
}

/// A change request in the result of `status`.
#[derive(Debug, Serialize)]
struct CrOut<'a> {
    title: &'a str,
    intent: &'static str,
    parent: Option<&'a str>,
    issuer: Option<&'a str>,
    crux: Option<&'static str>,
    supersedes: Option<&'a str>,
    state: &'static str,
    answer: Option<AnswerOut<'a>>,
    assumptions: Vec<AssumptionOut<'a>>,
}

/// A change request the definitions have not followed, in the result of `status`.
#[derive(Debug, Serialize)]
struct UnappliedOut<'a> {
    cr: &'a str,
    decision: Option<&'a str>,
    unchanged: &'a [String],
    gone: &'a [String],
    not_removed: &'a [String],
}

/// A change request that expected targets to stay as they are, and they moved, in the result of `status`.
#[derive(Debug, Serialize)]
struct KeepMovedOut<'a> {
    cr: &'a str,
    moved: &'a [String],
}

/// What an option of an undecided Crux expects of one target.
#[derive(Debug, Serialize)]
struct AwaitingExpectOut<'a> {
    kind: &'static str,
    target: &'a str,
}

/// An option of an undecided Crux.
#[derive(Debug, Serialize)]
struct AwaitingOptionOut<'a> {
    choice: &'a str,
    wording: Option<&'a str>,
    expects: Vec<AwaitingExpectOut<'a>>,
    consequence: Option<&'a str>,
}

/// The recommendation of an undecided Crux, with how sure (0 to 100).
#[derive(Debug, Serialize)]
struct RecommendedOut<'a> {
    choice: &'a str,
    reason: Option<&'a str>,
    confidence: Option<u8>,
}

/// A change request whose Crux has no decision yet, in the result of `status`.
#[derive(Debug, Serialize)]
struct AwaitingOut<'a> {
    cr: &'a str,
    kind: &'static str,
    options: Vec<AwaitingOptionOut<'a>>,
    recommended: Option<RecommendedOut<'a>>,
    importance: Option<&'static str>,
}

/// One line of the decision Policy.
#[derive(Debug, Serialize)]
struct PolicyLineOut<'a> {
    id: &'a str,
    text: &'a str,
}

/// The decision Policy, in the result of `status`.
#[derive(Debug, Serialize)]
struct PolicyOut<'a> {
    revision: u64,
    lines: Vec<PolicyLineOut<'a>>,
}

/// The result of `policy_put`.
#[derive(Debug, Serialize)]
struct PolicyPutOut<'a> {
    op: &'static str,
    revision: u64,
    findings: Vec<FindingOut<'a>>,
}

/// The result of an operation that gives nothing back but its name.
#[derive(Debug, Serialize)]
struct DoneOut {
    op: &'static str,
}

/// The freshness of a saved external execution, in the result of `status`.
#[derive(Debug, Serialize)]
struct ExternalOut<'a> {
    rule: &'a str,
    state: &'static str,
}

/// The result of `status`.
#[derive(Debug, Serialize)]
struct StatusOut<'a> {
    op: &'static str,
    pins: Vec<PinOut<'a>>,
    stale_cites: Vec<StaleCiteOut<'a>>,
    change_requests: Vec<CrOut<'a>>,
    unapplied: Vec<UnappliedOut<'a>>,
    keep_moved: Vec<KeepMovedOut<'a>>,
    awaiting: Vec<AwaitingOut<'a>>,
    policy: PolicyOut<'a>,
    findings: Vec<FindingOut<'a>>,
    judged: Vec<StandingOut<'a>>,
    external: Vec<ExternalOut<'a>>,
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

/// The choice for a decision maker. There is no `default`: work that depends on the answer waits for it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CruxDto {
    kind: KindDto,
    options: Vec<OptionDto>,
    #[serde(default)]
    recommended: Option<RecommendedDto>,
    #[serde(default)]
    importance: Option<ImportanceDto>,
    /// The title of the change request whose answered Crux this one replaces.
    #[serde(default)]
    supersedes: Option<String>,
}

/// The option recommended, why, and how sure (0 to 100).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecommendedDto {
    choice: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    confidence: Option<u64>,
}

impl From<RecommendedDto> for RecommendedInput {
    fn from(r: RecommendedDto) -> RecommendedInput {
        RecommendedInput {
            choice: r.choice,
            reason: r.reason,
            confidence: r.confidence,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ImportanceDto {
    Low,
    Normal,
    High,
}

impl From<ImportanceDto> for Importance {
    fn from(i: ImportanceDto) -> Importance {
        match i {
            ImportanceDto::Low => Importance::Low,
            ImportanceDto::Normal => Importance::Normal,
            ImportanceDto::High => Importance::High,
        }
    }
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
    /// The exact new wording the option makes of the definitions; none: it words none. (Called `change` before the
    /// lists of references took that name.)
    #[serde(default)]
    wording: Option<String>,
    /// The targets that must change when it is chosen, besides what the change request expects.
    #[serde(default)]
    change: Vec<RefDto>,
    /// The targets that must go when it is chosen.
    #[serde(default)]
    remove: Vec<RefDto>,
    /// The targets that must stay as they are when it is chosen.
    #[serde(default)]
    keep: Vec<RefDto>,
    #[serde(default)]
    consequence: Option<String>,
}

/// A reference to a target of any kind. The Project is `{"project": true}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RefDto {
    Project(bool),
    Context(String),
    Contract(ContractRefDto),
    Interface(InterfaceRefDto),
    Harness(HarnessRefDto),
    Item(ItemRefDto),
    Clause(ClauseRefDto),
    Case(CaseRefDto),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractRefDto {
    context: String,
    contract: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InterfaceRefDto {
    context: String,
    interface: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessRefDto {
    context: String,
    harness: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseRefDto {
    context: String,
    harness: String,
    name: String,
}

impl TryFrom<RefDto> for TargetInput {
    type Error = AppError;

    fn try_from(r: RefDto) -> Result<TargetInput, AppError> {
        Ok(match r {
            RefDto::Project(true) => TargetInput::Project,
            RefDto::Project(false) => {
                return Err(AppError::Input(
                    "`{\"project\": false}` names nothing; write `{\"project\": true}`".to_string(),
                ));
            }
            RefDto::Context(name) => TargetInput::Context(name),
            RefDto::Contract(c) => TargetInput::Contract {
                context: c.context,
                contract: c.contract,
            },
            RefDto::Interface(i) => TargetInput::Interface {
                context: i.context,
                interface: i.interface,
            },
            RefDto::Harness(h) => TargetInput::Harness {
                context: h.context,
                harness: h.harness,
            },
            RefDto::Item(i) => TargetInput::Item(ItemRef::from(i)),
            RefDto::Clause(c) => TargetInput::Clause(ClauseRef::from(c)),
            RefDto::Case(c) => TargetInput::Case {
                context: c.context,
                harness: c.harness,
                name: c.name,
            },
        })
    }
}

/// The expectations the lists of references `change`, `remove` and `keep` write, in that order.
fn expects_of(
    change: Vec<RefDto>,
    remove: Vec<RefDto>,
    keep: Vec<RefDto>,
) -> Result<Vec<ExpectInput>, AppError> {
    let mut out = Vec::with_capacity(change.len() + remove.len() + keep.len());
    for (kind, refs) in [
        (ExpectKind::Change, change),
        (ExpectKind::Remove, remove),
        (ExpectKind::Keep, keep),
    ] {
        for r in refs {
            out.push(ExpectInput {
                kind,
                target: TargetInput::try_from(r)?,
            });
        }
    }
    Ok(out)
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
                change,
                remove,
                keep,
                crux,
                assumptions,
                addresses,
            } = *open;
            let crux = crux
                .map(|q| {
                    Ok::<_, AppError>(CruxInput {
                        kind: q.kind.into(),
                        options: q
                            .options
                            .into_iter()
                            .map(|o| {
                                Ok(OptionInput {
                                    choice: o.choice,
                                    change: o.wording,
                                    expects: expects_of(o.change, o.remove, o.keep)?,
                                    consequence: o.consequence,
                                })
                            })
                            .collect::<Result<_, AppError>>()?,
                        recommended: q.recommended.map(RecommendedInput::from),
                        importance: q.importance.map(Importance::from),
                        supersedes: q.supersedes,
                    })
                })
                .transpose()?;
            let input = ChangeRequestInput {
                title,
                situation,
                intent: intent.into(),
                parent,
                issuer,
                expects: expects_of(change, remove, keep)?,
                crux,
                assumptions: assumptions.into_iter().map(AssumptionInput::from).collect(),
                addresses: addresses.into_iter().map(FindingInput::from).collect(),
            };
            let opened = app.open_change_request(&input).await?;
            to_value(&CrOpenOut {
                op: "cr_open",
                cr: opened.id.to_string(),
                expects: opened
                    .expects
                    .into_iter()
                    .map(OpenedExpectOut::from)
                    .collect(),
            })?
        }
        Op::PolicyPut { lines } => {
            let lines: Vec<PolicyLineInput> = lines
                .into_iter()
                .map(|l| PolicyLineInput {
                    id: l.id,
                    text: l.text,
                })
                .collect();
            let done = app.put_policy(&lines).await?;
            to_value(&PolicyPutOut {
                op: "policy_put",
                revision: done.value.revision.number(),
                findings: findings_out(&done.findings),
            })?
        }
        Op::CrAssume {
            title,
            statement,
            reason,
            evidence,
        } => {
            let assumption = AssumptionInput::from(AssumptionDto {
                statement,
                reason,
                evidence,
            });
            app.assume(&title, &assumption).await?;
            to_value(&DoneOut { op: "cr_assume" })?
        }
        Op::CrDecide {
            title,
            choice,
            wording,
            unspecified,
            reason,
            basis,
            by,
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
            app.decide_change_request(&title, &decision, &basis.into(), &by.into())
                .await?;
            to_value(&DoneOut { op: "cr_decide" })?
        }
        Op::CrResolve { title } => {
            let done = app.resolve_change_request(&title).await?;
            json!({ "op": "cr_resolve", "findings": findings_out(&done.findings) })
        }
        Op::Status => {
            let s = app.status().await?;
            to_value(&StatusOut {
                op: "status",
                pins: s
                    .pins
                    .iter()
                    .map(|p| {
                        let (state, why): (_, &[String]) = match &p.state {
                            PinState::Unpinned => ("unpinned", &[]),
                            PinState::Fresh => ("fresh", &[]),
                            PinState::Gone => ("gone", &[]),
                            PinState::Stale(why) => ("stale", why),
                        };
                        PinOut {
                            context: &p.context,
                            interface: &p.interface,
                            state,
                            why,
                        }
                    })
                    .collect(),
                stale_cites: s
                    .stale_cites
                    .iter()
                    .map(|c| StaleCiteOut {
                        case: &c.case,
                        target: &c.target,
                        cited: c.cited,
                        now: c.now,
                    })
                    .collect(),
                change_requests: s
                    .change_requests
                    .iter()
                    .map(|c| CrOut {
                        title: &c.title,
                        intent: c.intent,
                        parent: c.parent.as_deref(),
                        issuer: c.issuer.as_deref(),
                        crux: c.crux,
                        supersedes: c.supersedes.as_deref(),
                        state: c.state,
                        answer: c.answer.as_ref().map(|a| AnswerOut {
                            decision: &a.decision,
                            basis: match &a.basis {
                                BasisStatus::Policy {
                                    line,
                                    policy_revision,
                                } => BasisOut::Policy {
                                    line,
                                    policy_revision: *policy_revision,
                                },
                                BasisStatus::Arbitrary { reason } => BasisOut::Arbitrary { reason },
                            },
                            by: MakerOut {
                                kind: a.by_kind,
                                name: &a.by,
                            },
                            at: a.at,
                        }),
                        assumptions: c
                            .assumptions
                            .iter()
                            .map(|a| AssumptionOut {
                                statement: &a.statement,
                                reason: &a.reason,
                                evidence: a
                                    .evidence
                                    .iter()
                                    .map(|e| match e {
                                        EvidenceStatus::Repo { path, lines } => EvidenceOut::Repo {
                                            path,
                                            lines: *lines,
                                        },
                                        EvidenceStatus::Knowledge { reference } => {
                                            EvidenceOut::Knowledge { reference }
                                        }
                                    })
                                    .collect(),
                            })
                            .collect(),
                    })
                    .collect(),
                unapplied: s
                    .unapplied
                    .iter()
                    .map(|u| UnappliedOut {
                        cr: &u.cr,
                        decision: u.decision.as_deref(),
                        unchanged: &u.unchanged,
                        gone: &u.gone,
                        not_removed: &u.not_removed,
                    })
                    .collect(),
                keep_moved: s
                    .keep_moved
                    .iter()
                    .map(|k| KeepMovedOut {
                        cr: &k.cr,
                        moved: &k.moved,
                    })
                    .collect(),
                awaiting: s
                    .awaiting
                    .iter()
                    .map(|a| AwaitingOut {
                        cr: &a.cr,
                        kind: a.kind,
                        options: a
                            .options
                            .iter()
                            .map(|o| AwaitingOptionOut {
                                choice: &o.choice,
                                wording: o.wording.as_deref(),
                                expects: o
                                    .expects
                                    .iter()
                                    .map(|e| AwaitingExpectOut {
                                        kind: e.kind,
                                        target: &e.target,
                                    })
                                    .collect(),
                                consequence: o.consequence.as_deref(),
                            })
                            .collect(),
                        recommended: a.recommended.as_ref().map(|r| RecommendedOut {
                            choice: &r.choice,
                            reason: r.reason.as_deref(),
                            confidence: r.confidence,
                        }),
                        importance: a.importance,
                    })
                    .collect(),
                policy: PolicyOut {
                    revision: s.policy.revision,
                    lines: s
                        .policy
                        .lines
                        .iter()
                        .map(|l| PolicyLineOut {
                            id: &l.id,
                            text: &l.text,
                        })
                        .collect(),
                },
                findings: findings_out(&s.findings),
                judged: standing_out(&s.judged),
                external: s
                    .external
                    .iter()
                    .map(|r| ExternalOut {
                        rule: r.rule.as_str(),
                        state: r.state.word(),
                    })
                    .collect(),
            })?
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
