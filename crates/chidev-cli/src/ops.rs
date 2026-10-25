//! The operations `chidev exec` reads, one JSON object per line, and the results it prints. This is where the JSON
//! form is turned into the inputs of the use cases and back; nothing past it sees JSON.

use std::io::{self, BufRead, Write};
use std::path::{Component, Path};

use chidev_core::app::ports::external::ExternalExecutor;
use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, AssumptionInput, BasisInput, BasisStatus, CaseInput, ChangeRequestInput,
    ChangeSetInput, Checked, ClauseInput, ClauseRef, ContractPut, CruxInput, CruxKind,
    DecisionInput, DecisionMakerInput, Diagnostic, EvidenceInput, EvidenceStatus, FindingInput,
    Found, HarnessKind, HarnessPut, Importance, Intent, InterfacePut, ItemInput, ItemRef,
    ItemsInput, ModelSlice, ModelView, OpenedExpect, OptionInput, ParamValue, PinState,
    PolicyLineInput, PutDone, PutInput, ReasonedInput, RecommendedInput, RenameInput, ReportInput,
    ReportedFinding, ResolutionInput, ResolutionStatus, RetireInput, RetiredStatus, Retirement,
    RuleStanding, RuleToJudge, SubjectInput, TargetInput, TransitionStatus, ViewInput,
};
use chidev_core::domain::check::{Hook, Level};
use chidev_core::domain::vo::measure::MeasureKind;
use chidev_core::domain::vo::model_section::ModelSection;
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One operation. `op` names it; the other fields are its arguments.
#[derive(Debug, Deserialize, JsonSchema)]
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
    /// Makes or revises an Interface. The code is `source`, or the text of the file `source_file` (relative to the
    /// project's root); exactly one is given. `items` may be left out: the provider of the language its
    /// `[[measure.layout]]` gives finds them in the code, and `renames` (`{"from","to"}`) hand the id of the Item
    /// that had the key `from` to the one found under `to`. With `items`, nothing is found and `renames` is refused.
    InterfacePut {
        context: String,
        name: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        source_file: Option<String>,
        #[serde(default)]
        items: Option<Vec<ItemDto>>,
        #[serde(default)]
        renames: Vec<RenameDto>,
    },
    /// Gives the Items of an Interface the keys and lines the provider of its language finds in the code the
    /// Interface keeps (not the working tree). An Item is paired with a declaration when their lines overlap and each
    /// overlaps no other of the other side; the rest stay as they are and are listed in `unmatched`. The ids stay, so
    /// the citations do; `revised` lists the Items whose new lines declare something else, which go to their next
    /// revision. Not a change set: nothing else is put, and a store that moved meanwhile refuses it.
    ItemsRekey { context: String, interface: String },
    /// Makes or revises a Harness. `kind` is `tests` (the default for a new Harness) or `sim`; an existing Harness
    /// keeps its kind when none is given. `builds` (a sim only) names the Interfaces of the Context whose pinned
    /// code the program is built from; a put states all of it. A Case of tests cites an Item at least, a Case of a
    /// sim cites Clauses only. The source is `source`, or the text of the file `source_file`; exactly one is given.
    HarnessPut {
        context: String,
        name: String,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        builds: Vec<String>,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        source_file: Option<String>,
        cases: Vec<CaseDto>,
    },
    /// Puts and retires several assets as one change: the rules of the project are checked once, on the state it
    /// ends in, so a Harness can be put without the Cases that cite a Clause the same change removes, and can cite
    /// a Clause the same change makes. All of it is applied, or none (and the error names every problem). Each
    /// entry of `puts` is tagged with `put` (`contract`, `interface`, `harness`, `uses`, `base_on_contract`) and
    /// has the fields of the operation of that name without `op`; each entry of `retires` is tagged with `retire`
    /// (`context`, `contract`, `interface`, `harness`). `for` is the title of the change request the work is
    /// applied for: the targets the set moves are recorded as that request's work in the same commit, and `status`
    /// reads from such work whether the request's targets were revised or retired. Without `for` the set belongs
    /// to no request.
    ChangeSet {
        #[serde(default, rename = "for")]
        for_change_request: Option<String>,
        #[serde(default)]
        puts: Vec<PutDto>,
        #[serde(default)]
        retires: Vec<RetireDto>,
    },
    /// Retires a Context and everything it owns: it leaves the model, its ids stay reserved, nothing brings it
    /// back. Refused while a Context that stays uses it or a Case that stays cites what it owns.
    ContextRetire { context: String },
    /// Retires a Contract with its Clauses.
    ContractRetire { context: String, contract: String },
    /// Retires an Interface with its Items. Its Pin stays and reads `gone`.
    InterfaceRetire { context: String, interface: String },
    /// Retires a Harness with its Cases.
    HarnessRetire { context: String, harness: String },
    /// Pins the code of an Interface. The code is `code`, or the text of the file `code_file`; exactly one is given.
    Pin {
        context: String,
        interface: String,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        code_file: Option<String>,
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
    /// Closes the record of a change request with a resolution: `resolution` is `"done"`, `"wont_do"`,
    /// `"invalid"`, `{"duplicate":"<title>"}` or `{"superseded":"<title>"}` (the title of a change request that
    /// exists), with the `reason` and who resolves it (`by`). Refused only by explicit states: resolved already,
    /// the Crux unanswered (for `"done"`), a part not resolved, a reference that does not exist. The targets and
    /// the work are not consulted; checks report where they disagree.
    CrResolve {
        title: String,
        resolution: ResolutionDto,
        reason: String,
        by: MakerDto,
    },
    /// Opens a resolved change request again, with the `reason` and who does it (`by`). The resolution stays in
    /// its history.
    CrReopen {
        title: String,
        reason: String,
        by: MakerDto,
    },
    /// Measures, in a copy of the project made under `.chidev/measure/`, and keeps what was observed. `kind`
    /// `mutate` (with `interface`) runs the code the Pin of the Interface accepted against the Harnesses as the
    /// model holds them, once per mutant. `kind` `simulate` (with `harness`, a sim Harness) builds the program from
    /// the code of the Pins of the Interfaces the Harness cites and runs its script once per case. Either takes as
    /// long as the project's build and tests or script take.
    Measure {
        kind: MeasureKindDto,
        context: String,
        #[serde(default)]
        interface: Option<String>,
        #[serde(default)]
        harness: Option<String>,
    },
    /// Lists every rule: the invariants an operation is refused for, the rules an input must meet, and the checks
    /// (with the level the policy gives them). Each has the id a diagnostic carries as its `code`.
    Rules,
    /// Explains one rule by its id (`rule`): what it protects, why and how to mend a violation, the keys of the
    /// values a diagnostic of it carries, and an operation that violates it.
    Explain { rule: String },
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
    /// Reads the part of the model that is asked for: the `sections` named (at least one; nothing is read by
    /// default), of the Contexts named in `contexts` (every Context when it is left out; none when it is `[]`). The
    /// Contexts always come with their `id`, `name`, `basis` and `uses`; the change requests belong to the Project,
    /// so they come when `change_requests` is a section, whatever `contexts` says. The result is the `view`, and the
    /// `store_revision` it was read at.
    View {
        sections: Vec<SectionDto>,
        #[serde(default)]
        contexts: Option<Vec<String>>,
    },
}

/// One entry of the `puts` of a `change_set`: the fields of the operation named by `put`, without `op`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "put", rename_all = "snake_case", deny_unknown_fields)]
enum PutDto {
    /// As `context_add`.
    Context {
        name: String,
        #[serde(default)]
        uses: Vec<String>,
    },
    /// As `context_uses`.
    Uses { context: String, uses: Vec<String> },
    /// As `base_on_contract`.
    BaseOnContract { context: String },
    /// As `contract_put`.
    Contract {
        context: String,
        name: String,
        title: String,
        clauses: Vec<ClauseDto>,
    },
    /// As `interface_put`.
    Interface {
        context: String,
        name: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        source_file: Option<String>,
        #[serde(default)]
        items: Option<Vec<ItemDto>>,
        #[serde(default)]
        renames: Vec<RenameDto>,
    },
    /// As `harness_put`.
    Harness {
        context: String,
        name: String,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        builds: Vec<String>,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        source_file: Option<String>,
        cases: Vec<CaseDto>,
    },
}

/// One entry of the `retires` of a `change_set`: the fields of the retire operation named by `retire`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "retire", rename_all = "snake_case", deny_unknown_fields)]
enum RetireDto {
    /// As `context_retire`.
    Context { context: String },
    /// As `contract_retire`.
    Contract { context: String, contract: String },
    /// As `interface_retire`.
    Interface { context: String, interface: String },
    /// As `harness_retire`.
    Harness { context: String, harness: String },
}

/// The puts of a change set as inputs, the files they name read. Every entry is looked at, so a set with several
/// unreadable files is refused with all of them named, as the rest of the set's problems are.
async fn put_inputs(root: &Path, puts: Vec<PutDto>) -> Result<Vec<PutInput>, AppError> {
    let mut inputs = Vec::with_capacity(puts.len());
    let mut problems = Vec::new();
    for put in puts {
        match put_input(root, put).await {
            Ok(input) => inputs.push(input),
            Err(problem) => problems.push(problem),
        }
    }
    match problems.len() {
        0 => Ok(inputs),
        1 => Err(problems.remove(0)),
        _ => Err(AppError::Many(problems)),
    }
}

/// An entry of the `puts` of a change set as the input of the use case; the text of a `source_file` is read here.
async fn put_input(root: &Path, d: PutDto) -> Result<PutInput, AppError> {
    Ok(match d {
        PutDto::Context { name, uses } => PutInput::Context { name, uses },
        PutDto::Uses { context, uses } => PutInput::Uses { context, uses },
        PutDto::BaseOnContract { context } => PutInput::BaseOnContract { context },
        PutDto::Contract {
            context,
            name,
            title,
            clauses,
        } => PutInput::Contract(ContractPut {
            context,
            name,
            title,
            clauses: clause_inputs(clauses),
        }),
        PutDto::Interface {
            context,
            name,
            source,
            source_file,
            items,
            renames,
        } => {
            let label = format!("the put of the interface `{name}`");
            let source = text_of(
                root,
                &label,
                ("source", source),
                ("source_file", source_file),
            )
            .await?;
            let items = items_input(&context, &name, items, renames)?;
            PutInput::Interface(InterfacePut {
                context,
                name,
                source,
                items,
            })
        }
        PutDto::Harness {
            context,
            name,
            kind,
            builds,
            source,
            source_file,
            cases,
        } => {
            let label = format!("the put of the harness `{name}`");
            let source = text_of(
                root,
                &label,
                ("source", source),
                ("source_file", source_file),
            )
            .await?;
            PutInput::Harness(HarnessPut {
                context,
                name,
                kind: harness_kind(kind.as_deref())?,
                builds,
                source,
                cases: case_inputs(cases),
            })
        }
    })
}

/// The text an operation takes either inline (the first field, `(name, value)`) or from a file (the second): exactly
/// one is given. `label` says what the text is for, in the error.
async fn text_of(
    root: &Path,
    label: &str,
    (inline_field, inline): (&str, Option<String>),
    (file_field, file): (&str, Option<String>),
) -> Result<String, AppError> {
    match (inline, file) {
        (Some(text), None) => Ok(text),
        (None, Some(file)) => read_text(root, label, file_field, &file).await,
        (Some(_), Some(_)) => Err(AppError::Input(format!(
            "{label}: give `{inline_field}` or `{file_field}`, not both"
        ))),
        (None, None) => Err(AppError::Input(format!(
            "{label}: give `{inline_field}` or `{file_field}`"
        ))),
    }
}

/// The text of the file `file`, a path relative to `root`. Reading is here, at the boundary, so the use cases and the
/// domain receive text only, and a path is never kept.
async fn read_text(root: &Path, label: &str, field: &str, file: &str) -> Result<String, AppError> {
    let relative = relative_path(file)
        .map_err(|why| AppError::Input(format!("{label}: `{field}` `{file}` {why}")))?;
    let bytes = tokio::fs::read(root.join(relative))
        .await
        .map_err(|e| AppError::Input(format!("{label}: cannot read `{field}` `{file}`: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|_| AppError::Input(format!("{label}: `{field}` `{file}` is not UTF-8 text")))
}

/// `file` as a path that stays under the root: not empty, not absolute, with no `..`. The error says what is wrong.
fn relative_path(file: &str) -> Result<&Path, &'static str> {
    let path = Path::new(file);
    if file.is_empty() {
        return Err("is empty");
    }
    if path.is_absolute() || path.has_root() {
        return Err("is an absolute path; it is relative to the project's root");
    }
    for part in path.components() {
        match part {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err("goes up with `..`; it stays under the project's root");
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("is an absolute path; it is relative to the project's root");
            }
        }
    }
    Ok(path)
}

impl From<RetireDto> for RetireInput {
    fn from(d: RetireDto) -> RetireInput {
        match d {
            RetireDto::Context { context } => RetireInput::Context(context),
            RetireDto::Contract { context, contract } => {
                RetireInput::Contract { context, contract }
            }
            RetireDto::Interface { context, interface } => {
                RetireInput::Interface { context, interface }
            }
            RetireDto::Harness { context, harness } => RetireInput::Harness { context, harness },
        }
    }
}

fn clause_inputs(clauses: Vec<ClauseDto>) -> Vec<ClauseInput> {
    clauses
        .into_iter()
        .map(|c| ClauseInput {
            label: c.label,
            text: c.text,
        })
        .collect()
}

fn item_inputs(items: Vec<ItemDto>) -> Vec<ItemInput> {
    items
        .into_iter()
        .map(|i| ItemInput {
            key: i.key,
            lines: (i.lines[0], i.lines[1]),
            replaces: i.replaces,
        })
        .collect()
}

/// The Items of an Interface put: the `items` written, or (left out) the ones the provider finds with the `renames`.
/// Both given are refused here, since `renames` are for the Items that are found.
fn items_input(
    context: &str,
    interface: &str,
    items: Option<Vec<ItemDto>>,
    renames: Vec<RenameDto>,
) -> Result<ItemsInput, AppError> {
    match items {
        Some(_) if !renames.is_empty() => Err(AppError::RenamesWithItems {
            context: context.to_string(),
            interface: interface.to_string(),
        }),
        Some(items) => Ok(ItemsInput::Written(item_inputs(items))),
        None => Ok(ItemsInput::Found {
            renames: rename_inputs(renames),
        }),
    }
}

fn rename_inputs(renames: Vec<RenameDto>) -> Vec<RenameInput> {
    renames
        .into_iter()
        .map(|r| RenameInput {
            from: r.from,
            to: r.to,
        })
        .collect()
}

fn case_inputs(cases: Vec<CaseDto>) -> Vec<CaseInput> {
    cases
        .into_iter()
        .map(|c| CaseInput {
            name: c.name,
            lines: (c.lines[0], c.lines[1]),
            items: c.items.into_iter().map(ItemRef::from).collect(),
            clauses: c.clauses.into_iter().map(ClauseRef::from).collect(),
        })
        .collect()
}

fn harness_kind(kind: Option<&str>) -> Result<Option<HarnessKind>, AppError> {
    kind.map(|w| {
        HarnessKind::from_word(w).ok_or_else(|| {
            AppError::Input(format!(
                "the harness kind `{w}` is not one; it is tests or sim"
            ))
        })
    })
    .transpose()
}

/// A part of the model a `view` reads.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SectionDto {
    Contracts,
    Interfaces,
    Harnesses,
    ChangeRequests,
}

impl From<SectionDto> for ModelSection {
    fn from(s: SectionDto) -> ModelSection {
        match s {
            SectionDto::Contracts => ModelSection::Contracts,
            SectionDto::Interfaces => ModelSection::Interfaces,
            SectionDto::Harnesses => ModelSection::Harnesses,
            SectionDto::ChangeRequests => ModelSection::ChangeRequests,
        }
    }
}

/// What a `measure` measures.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MeasureKindDto {
    Mutate,
    Simulate,
}

impl From<MeasureKindDto> for MeasureKind {
    fn from(k: MeasureKindDto) -> MeasureKind {
        match k {
            MeasureKindDto::Mutate => MeasureKind::Mutate,
            MeasureKindDto::Simulate => MeasureKind::Simulate,
        }
    }
}

/// A promise a case broke, in the result of `measure`.
#[derive(Debug, Serialize)]
struct CaseFindingOut<'a> {
    step: u32,
    clause: &'a str,
    text: &'a str,
}

/// A clause a case did not reach, in the result of `measure`.
#[derive(Debug, Serialize)]
struct UnreachedOut<'a> {
    clause: &'a str,
    text: &'a str,
}

/// A case of a simulation, in the result of `measure`.
#[derive(Debug, Serialize)]
struct CaseOut<'a> {
    case: &'a str,
    exit: &'a str,
    findings: Vec<CaseFindingOut<'a>>,
    unreached: Vec<UnreachedOut<'a>>,
    undecided: &'a [String],
    schedule: Option<&'a str>,
    trace: Option<&'a str>,
}

/// A case run a second time, in the result of `measure`.
#[derive(Debug, Serialize)]
struct RerunOut<'a> {
    case: &'a str,
    result: &'static str,
}

/// What a simulation found, in numbers, in the result of `status`.
#[derive(Debug, Serialize)]
struct SimSummaryOut {
    cases: u32,
    findings: u32,
    unreached: u32,
    abnormal: u32,
    reruns: u32,
    differing: u32,
}

/// A mutant in the result of `measure`.
#[derive(Debug, Serialize)]
struct MutantOut<'a> {
    key: &'a str,
    site: &'a str,
    operator: &'a str,
    change: &'a str,
    outcome: &'static str,
}

/// A command of the run, in the result of `measure`.
#[derive(Debug, Serialize)]
struct StepOut<'a> {
    step: &'a str,
    command: &'a str,
    exit_code: Option<i32>,
    duration_ms: u64,
}

/// The result of `measure`.
#[derive(Debug, Serialize)]
struct MeasureOut<'a> {
    op: &'static str,
    kind: &'static str,
    id: &'a str,
    stamp: &'a str,
    outcome: &'static str,
    failure: Option<&'a str>,
    /// The places the provider found to change.
    sites: u32,
    /// The mutants evaluated; fewer than `sites` with `capped` set means `max_mutants` left sites out.
    evaluated: u32,
    capped: bool,
    killed: u32,
    survived: u32,
    stillborn: u32,
    timeout: u32,
    mutants: Vec<MutantOut<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cases: Vec<CaseOut<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reruns: Vec<RerunOut<'a>>,
    evidence: Vec<StepOut<'a>>,
}

/// The latest measurement of an Interface or a sim Harness, in the result of `status`.
#[derive(Debug, Serialize)]
struct MeasurementOut<'a> {
    context: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    interface: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness: Option<&'a str>,
    kind: &'static str,
    id: &'a str,
    stamp: &'a str,
    state: &'static str,
    outcome: &'static str,
    failure: Option<&'a str>,
    /// A mutation run: the places the provider found, the mutants evaluated, and whether `max_mutants` left sites
    /// out of a run that ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    sites: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evaluated: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capped: Option<bool>,
    killed: u32,
    survived: u32,
    stillborn: u32,
    timeout: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    sim: Option<SimSummaryOut>,
    at: u64,
}

/// The fields of `finding_report`.
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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

/// How a change request ended: a word, or a word that points at another change request by its title.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ResolutionDto {
    Done,
    WontDo,
    Invalid,
    Duplicate(String),
    Superseded(String),
}

impl From<ResolutionDto> for ResolutionInput {
    fn from(r: ResolutionDto) -> ResolutionInput {
        match r {
            ResolutionDto::Done => ResolutionInput::Done,
            ResolutionDto::WontDo => ResolutionInput::WontDo,
            ResolutionDto::Invalid => ResolutionInput::Invalid,
            ResolutionDto::Duplicate(t) => ResolutionInput::Duplicate(t),
            ResolutionDto::Superseded(t) => ResolutionInput::Superseded(t),
        }
    }
}

/// The fields of `cr_open`, boxed: they are far larger than any other operation's. A field it does not know is
/// refused, so a word it ignored would not be lost silently.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenDto {
    title: String,
    /// Why it is needed, in the words of whoever raised it. Kept with the request and shown by `status`.
    #[serde(default)]
    situation: Option<String>,
    intent: IntentDto,
    #[serde(default)]
    parent: Option<String>,
    /// Who raised it. Kept with the request and shown by `status`.
    #[serde(default)]
    issuer: Option<String>,
    /// The targets it asks to be revised or retired. A parent need not name any.
    #[serde(default)]
    targets: Vec<RefDto>,
    /// The targets it asks to stay as they are.
    #[serde(default)]
    keep: Vec<RefDto>,
    /// The choice for a decision maker, besides the targets; none: nothing to decide.
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
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssumptionDto {
    statement: String,
    reason: String,
    #[serde(default)]
    evidence: Vec<EvidenceDto>,
}

/// What an Assumption stands on: a place in the repository, or general knowledge.
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PolicyLineDto {
    id: String,
    text: String,
}

/// A finding a change request addresses. New and legacy input forms normalize at the CLI boundary.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(try_from = "AddressWire")]
struct AddressDto {
    rule: String,
    subject: Option<SubjectDto>,
}

/// Preserve presence separately from null so mixed selectors and duplicate null fields are refused.
#[derive(Debug, Deserialize, JsonSchema)]
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

/// A finding in a result: the diagnostic it is (`code`, `level`, `message`, `subjects`, `values`, `help`, ...), and
/// the fields a finding has besides (`rule` is its `code`, kept; `state` says whether a change request deals with it).
#[derive(Debug, Serialize)]
struct FindingOut<'a> {
    #[serde(flatten)]
    diagnostic: Diagnostic,
    rule: &'a str,
    /// `open`, `addressed` or `decided`.
    state: &'static str,
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
            diagnostic: Diagnostic::of_found(f),
            rule: f.rule.as_str(),
            state: f.state.word(),
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
        key: String,
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
                key: r.key,
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

/// A target (or keep) a change request recorded, in the result of `cr_open`.
#[derive(Debug, Serialize)]
struct OpenedExpectOut {
    /// `target` or `keep`.
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

/// How a change request ended, in the result of `status`.
#[derive(Debug, Serialize)]
struct ResolutionOut<'a> {
    /// `done`, `wont_do`, `duplicate`, `superseded` or `invalid`.
    resolution: &'static str,
    /// The title of the change request a `duplicate` or a `superseded` points at.
    of: Option<&'a str>,
    reason: &'a str,
    by: MakerOut<'a>,
    at: u64,
}

impl<'a> From<&'a ResolutionStatus> for ResolutionOut<'a> {
    fn from(r: &'a ResolutionStatus) -> ResolutionOut<'a> {
        ResolutionOut {
            resolution: r.kind,
            of: r.reference.as_deref(),
            reason: &r.reason,
            by: MakerOut {
                kind: r.by_kind,
                name: &r.by,
            },
            at: r.at,
        }
    }
}

/// One step of the history of a change request, in the result of `status`: `step` is `resolved` or `reopened`,
/// and a resolved step has the `resolution` (and the `of` it points at).
#[derive(Debug, Serialize)]
struct HistoryOut<'a> {
    step: &'static str,
    resolution: Option<&'static str>,
    of: Option<&'a str>,
    reason: &'a str,
    by: MakerOut<'a>,
    at: u64,
}

impl<'a> From<&'a TransitionStatus> for HistoryOut<'a> {
    fn from(t: &'a TransitionStatus) -> HistoryOut<'a> {
        match t {
            TransitionStatus::Resolved(r) => HistoryOut {
                step: "resolved",
                resolution: Some(r.kind),
                of: r.reference.as_deref(),
                reason: &r.reason,
                by: MakerOut {
                    kind: r.by_kind,
                    name: &r.by,
                },
                at: r.at,
            },
            TransitionStatus::Reopened(r) => HistoryOut {
                step: "reopened",
                resolution: None,
                of: None,
                reason: &r.reason,
                by: MakerOut {
                    kind: r.by_kind,
                    name: &r.by,
                },
                at: r.at,
            },
        }
    }
}

/// A target a change request asks to be revised or retired, in the result of `status`.
#[derive(Debug, Serialize)]
struct TargetProgressOut<'a> {
    target: &'a str,
    /// `revised`, `retired` or `not touched yet`.
    progress: &'static str,
}

/// A target a change request asks to keep, in the result of `status`.
#[derive(Debug, Serialize)]
struct KeepProgressOut<'a> {
    target: &'a str,
    moved: bool,
}

/// A change request in the result of `status`.
#[derive(Debug, Serialize)]
struct CrOut<'a> {
    title: &'a str,
    situation: Option<&'a str>,
    intent: &'static str,
    parent: Option<&'a str>,
    issuer: Option<&'a str>,
    crux: Option<&'static str>,
    supersedes: Option<&'a str>,
    state: &'static str,
    answer: Option<AnswerOut<'a>>,
    resolution: Option<ResolutionOut<'a>>,
    history: Vec<HistoryOut<'a>>,
    assumptions: Vec<AssumptionOut<'a>>,
    targets: Vec<TargetProgressOut<'a>>,
    keep: Vec<KeepProgressOut<'a>>,
    work: Vec<WorkOut<'a>>,
    out_of_frame: &'a [String],
}

/// What one change set applied for a change request moved, in the result of `status`.
#[derive(Debug, Serialize)]
struct WorkOut<'a> {
    store_revision: u64,
    moved: &'a [String],
}

/// A change request that keeps targets as they are, and they moved, in the result of `status`.
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

/// A thing the project retired, in the result of `status` and of a retire operation.
#[derive(Debug, Serialize)]
struct RetiredOut<'a> {
    kind: &'static str,
    context: &'a str,
    name: &'a str,
    last_revision: u64,
    at: u64,
}

impl<'a> From<&'a RetiredStatus> for RetiredOut<'a> {
    fn from(r: &'a RetiredStatus) -> RetiredOut<'a> {
        RetiredOut {
            kind: r.kind,
            context: &r.context,
            name: &r.name,
            last_revision: r.last_revision,
            at: r.at,
        }
    }
}

/// The result of a retire operation: everything that left the model, the thing first.
#[derive(Debug, Serialize)]
struct RetireOut<'a> {
    op: &'static str,
    retired: Vec<RetiredOut<'a>>,
    findings: Vec<FindingOut<'a>>,
}

/// The result of `items_rekey`: the Items that were given another key, those whose revision went up, and the keys of
/// those no declaration was paired with.
#[derive(Debug, Serialize)]
struct RekeyOut<'a> {
    op: &'static str,
    rekeyed: Vec<KeyChangedOut<'a>>,
    revised: Vec<ItemRevisedOut<'a>>,
    unmatched: &'a [String],
    /// The revision of the Interface once it is done.
    revision: u64,
}

#[derive(Debug, Serialize)]
struct KeyChangedOut<'a> {
    from: &'a str,
    to: &'a str,
}

#[derive(Debug, Serialize)]
struct ItemRevisedOut<'a> {
    key: &'a str,
    from: u64,
    to: u64,
}

/// What one put of a `change_set` came to: the asset's name, id and revision (a setting of a Context has none).
#[derive(Debug, Serialize)]
struct PutOut<'a> {
    put: &'static str,
    context: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
}

impl<'a> From<&'a PutDone> for PutOut<'a> {
    fn from(d: &'a PutDone) -> PutOut<'a> {
        PutOut {
            put: d.kind,
            context: &d.context,
            name: d.name.as_deref(),
            id: d.id.map(|id| id.to_string()),
            revision: d.revision.map(|r| r.number()),
        }
    }
}

/// The result of a `change_set`: each put in the order listed, and everything that left the model (what the puts
/// dropped, then what was retired).
#[derive(Debug, Serialize)]
struct ChangeSetOut<'a> {
    op: &'static str,
    puts: Vec<PutOut<'a>>,
    retired: Vec<RetiredOut<'a>>,
    findings: Vec<FindingOut<'a>>,
}

/// The result of `status`.
#[derive(Debug, Serialize)]
struct StatusOut<'a> {
    op: &'static str,
    retired: Vec<RetiredOut<'a>>,
    pins: Vec<PinOut<'a>>,
    stale_cites: Vec<StaleCiteOut<'a>>,
    change_requests: Vec<CrOut<'a>>,
    keep_moved: Vec<KeepMovedOut<'a>>,
    awaiting: Vec<AwaitingOut<'a>>,
    policy: PolicyOut<'a>,
    findings: Vec<FindingOut<'a>>,
    judged: Vec<StandingOut<'a>>,
    external: Vec<ExternalOut<'a>>,
    measurements: Vec<MeasurementOut<'a>>,
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

/// The result of `view`: the part of the model asked for (read at `store_revision`).
#[derive(Debug, Serialize)]
struct ViewOut<'a> {
    op: &'static str,
    store_revision: u64,
    view: &'a ModelSlice,
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

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OptionDto {
    choice: String,
    /// The exact new wording the option makes of the definitions; none: it words none.
    #[serde(default)]
    wording: Option<String>,
    /// The targets it asks to be revised or retired when it is chosen, besides those of the change request.
    #[serde(default)]
    targets: Vec<RefDto>,
    /// The targets it asks to stay as they are when it is chosen.
    #[serde(default)]
    keep: Vec<RefDto>,
    #[serde(default)]
    consequence: Option<String>,
}

/// A reference to a target of any kind. The Project is `{"project": true}`.
#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContractRefDto {
    context: String,
    contract: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InterfaceRefDto {
    context: String,
    interface: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HarnessRefDto {
    context: String,
    harness: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
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

/// The targets a list of references names.
fn targets_of(refs: Vec<RefDto>) -> Result<Vec<TargetInput>, AppError> {
    refs.into_iter().map(TargetInput::try_from).collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClauseDto {
    label: String,
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameDto {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ItemDto {
    key: String,
    lines: [usize; 2],
    #[serde(default)]
    replaces: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ItemRefDto {
    #[serde(default)]
    context: Option<String>,
    interface: String,
    key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClauseRefDto {
    #[serde(default)]
    context: Option<String>,
    contract: String,
    label: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
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
            key: d.key,
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

/// The JSON Schema of one line `chidev exec` reads: an operation, derived from the types that read it, so the schema
/// and the reading cannot differ.
pub fn input_schema() -> schemars::Schema {
    SchemaSettings::draft2020_12()
        .for_deserialize()
        .into_generator()
        .into_root_schema_for::<Op>()
}

/// Runs every line of `input` against `app` and writes one result line per operation to `out`. A blank line is
/// skipped. A line that fails (bad JSON or a refused operation) gets a result with `"ok": false` and the run goes
/// on. `root` is the project's root, the directory a `source_file` or a `code_file` is relative to. Returns whether
/// every operation succeeded; an error is only a failure to read or write.
pub async fn run<S, G, E>(
    app: &mut App<S, G, E>,
    root: &Path,
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
        // The line is read as an operation directly (a duplicated field is refused, which reading it as a JSON value
        // first would hide); the value is read besides only to say where in the input a refusal is about.
        let raw = serde_json::from_str::<Value>(&line).ok();
        let result = match serde_json::from_str::<Op>(&line) {
            Ok(op) => apply(app, root, op).await.map_err(Refusal::Refused),
            Err(e) => Err(Refusal::Unreadable(Diagnostic::of_unreadable(
                &e.to_string(),
                raw.as_ref(),
            ))),
        };
        let mut result = match result {
            Ok(Value::Object(mut fields)) => {
                fields.insert("ok".to_string(), json!(true));
                Value::Object(fields)
            }
            Ok(other) => json!({ "ok": true, "result": other }),
            Err(refusal) => {
                all_ok = false;
                refused_out(app, refusal, raw.as_ref()).await
            }
        };
        result["line"] = json!(k + 1);
        writeln!(out, "{result}")?;
    }
    Ok(all_ok)
}

/// Why a line gave no result: it could not be read as an operation, or the use case refused it.
enum Refusal {
    /// The line is not an operation chidev reads (not JSON, an unknown `op`, a field it does not take, a missing
    /// field, a value of another type).
    Unreadable(Diagnostic),
    /// The operation was read and refused.
    Refused(AppError),
}

/// The result of a refused line: `ok: false` and the `diagnostics`, which are the app's structure as it is. A
/// change refused by checks also keeps the `findings` (and the `external` attempts of a gate), as before.
async fn refused_out<S, G, E>(app: &App<S, G, E>, refusal: Refusal, input: Option<&Value>) -> Value
where
    S: Store,
    G: IdSource,
    E: ExternalExecutor,
{
    let (error, mut diagnostics) = match refusal {
        Refusal::Unreadable(d) => (None, vec![d]),
        Refusal::Refused(error) => {
            let diagnostics = app.diagnose(&error).await;
            (Some(error), diagnostics)
        }
    };
    if let Some(input) = input {
        diagnostics = diagnostics
            .into_iter()
            .map(|d| d.located_in(input))
            .collect();
    }
    let mut out = json!({ "ok": false, "diagnostics": diagnostics });
    match error {
        Some(AppError::Refused(found)) => out["findings"] = json!(findings_out(&found)),
        Some(AppError::ExternalRefused { findings, external }) => {
            out["findings"] = json!(findings_out(&findings));
            out["external"] = json!(external);
        }
        _ => {}
    }
    out
}

/// A typed result as the JSON object the run prints.
fn to_value(out: &impl Serialize) -> Result<Value, AppError> {
    serde_json::to_value(out).map_err(|e| AppError::Input(format!("cannot write the result: {e}")))
}

/// The result of the retire operation `op`.
fn retire_out(op: &'static str, done: &Checked<Retirement>) -> Result<Value, AppError> {
    to_value(&RetireOut {
        op,
        retired: done.value.retired.iter().map(RetiredOut::from).collect(),
        findings: findings_out(&done.findings),
    })
}

async fn apply<S, G, E>(app: &mut App<S, G, E>, root: &Path, op: Op) -> Result<Value, AppError>
where
    S: Store,
    G: IdSource,
    E: ExternalExecutor,
{
    Ok(match op {
        Op::Rules => json!({ "op": "rules", "rules": app.rules() }),
        Op::Explain { rule } => {
            let mut explained = to_value(&app.explain(&rule)?)?;
            explained["op"] = json!("explain");
            explained
        }
        Op::CheckRun => {
            let result = app.check_run().await?;
            json!({ "op": "check_run", "findings": findings_out(&result.findings), "external": result.external })
        }
        Op::Measure {
            kind,
            context,
            interface,
            harness,
        } => {
            let done = match (MeasureKind::from(kind), interface, harness) {
                (MeasureKind::Mutate, Some(interface), None) => {
                    app.measure(MeasureKind::Mutate, &context, &interface)
                        .await?
                }
                (MeasureKind::Simulate, None, Some(harness)) => {
                    app.simulate(&context, &harness).await?
                }
                (MeasureKind::Mutate, _, _) => {
                    return Err(AppError::Input(
                        "a `mutate` measure takes an `interface` and no `harness`".to_string(),
                    ));
                }
                (MeasureKind::Simulate, _, _) => {
                    return Err(AppError::Input(
                        "a `simulate` measure takes a `harness` (a sim harness) and no `interface`"
                            .to_string(),
                    ));
                }
            };
            to_value(&MeasureOut {
                op: "measure",
                kind: done.kind,
                id: &done.id,
                stamp: &done.stamp,
                outcome: if done.failure.is_some() {
                    "failed"
                } else {
                    "ok"
                },
                failure: done.failure.as_deref(),
                sites: done.sites,
                evaluated: done.evaluated,
                capped: done.capped,
                killed: done.counts.killed,
                survived: done.counts.survived,
                stillborn: done.counts.stillborn,
                timeout: done.counts.timeout,
                mutants: done
                    .mutants
                    .iter()
                    .map(|m| MutantOut {
                        key: &m.key,
                        site: &m.site,
                        operator: &m.operator,
                        change: &m.change,
                        outcome: m.outcome,
                    })
                    .collect(),
                cases: done
                    .cases
                    .iter()
                    .map(|c| CaseOut {
                        case: &c.case,
                        exit: &c.exit,
                        findings: c
                            .findings
                            .iter()
                            .map(|f| CaseFindingOut {
                                step: f.step,
                                clause: &f.clause,
                                text: &f.text,
                            })
                            .collect(),
                        unreached: c
                            .unreached
                            .iter()
                            .map(|u| UnreachedOut {
                                clause: &u.clause,
                                text: &u.text,
                            })
                            .collect(),
                        undecided: &c.undecided,
                        schedule: c.schedule.as_deref(),
                        trace: c.trace.as_deref(),
                    })
                    .collect(),
                reruns: done
                    .reruns
                    .iter()
                    .map(|r| RerunOut {
                        case: &r.case,
                        result: r.result,
                    })
                    .collect(),
                evidence: done
                    .evidence
                    .iter()
                    .map(|e| StepOut {
                        step: &e.step,
                        command: &e.command,
                        exit_code: e.exit_code,
                        duration_ms: e.duration_ms,
                    })
                    .collect(),
            })?
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
        Op::ChangeSet {
            for_change_request,
            puts,
            retires,
        } => {
            let set = ChangeSetInput {
                change_request: for_change_request,
                puts: put_inputs(root, puts).await?,
                retires: retires.into_iter().map(RetireInput::from).collect(),
            };
            let done = app.apply_change_set(&set).await?;
            to_value(&ChangeSetOut {
                op: "change_set",
                puts: done.value.puts.iter().map(PutOut::from).collect(),
                retired: done.value.retired.iter().map(RetiredOut::from).collect(),
                findings: findings_out(&done.findings),
            })?
        }
        Op::ContractPut {
            context,
            name,
            title,
            clauses,
        } => {
            let clauses = clause_inputs(clauses);
            let done = app.put_contract(&context, &name, &title, &clauses).await?;
            json!({ "op": "contract_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
        }
        Op::InterfacePut {
            context,
            name,
            source,
            source_file,
            items,
            renames,
        } => {
            let label = format!("interface `{name}`");
            let source = text_of(
                root,
                &label,
                ("source", source),
                ("source_file", source_file),
            )
            .await?;
            let items = items_input(&context, &name, items, renames)?;
            let done = app
                .put_interface_of(&context, &name, &source, items)
                .await?;
            json!({ "op": "interface_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
        }
        Op::HarnessPut {
            context,
            name,
            kind,
            builds,
            source,
            source_file,
            cases,
        } => {
            let label = format!("harness `{name}`");
            let source = text_of(
                root,
                &label,
                ("source", source),
                ("source_file", source_file),
            )
            .await?;
            let kind = harness_kind(kind.as_deref())?;
            let cases = case_inputs(cases);
            let done = app
                .put_harness_of(&context, &name, kind, &builds, &source, &cases)
                .await?;
            json!({ "op": "harness_put", "id": done.value.id.to_string(), "revision": done.value.revision.number(), "findings": findings_out(&done.findings) })
        }
        Op::ContextRetire { context } => {
            let done = app.retire_context(&context).await?;
            retire_out("context_retire", &done)?
        }
        Op::ContractRetire { context, contract } => {
            let done = app.retire_contract(&context, &contract).await?;
            retire_out("contract_retire", &done)?
        }
        Op::ItemsRekey { context, interface } => {
            let done = app.items_rekey(&context, &interface).await?;
            to_value(&RekeyOut {
                op: "items_rekey",
                rekeyed: done
                    .rekeyed
                    .iter()
                    .map(|k| KeyChangedOut {
                        from: &k.from,
                        to: &k.to,
                    })
                    .collect(),
                revised: done
                    .revised
                    .iter()
                    .map(|r| ItemRevisedOut {
                        key: &r.key,
                        from: r.from,
                        to: r.to,
                    })
                    .collect(),
                unmatched: &done.unmatched,
                revision: done.revision,
            })?
        }
        Op::InterfaceRetire { context, interface } => {
            let done = app.retire_interface(&context, &interface).await?;
            retire_out("interface_retire", &done)?
        }
        Op::HarnessRetire { context, harness } => {
            let done = app.retire_harness(&context, &harness).await?;
            retire_out("harness_retire", &done)?
        }
        Op::Pin {
            context,
            interface,
            code,
            code_file,
        } => {
            let label = format!("pin of the interface `{interface}`");
            let code = text_of(root, &label, ("code", code), ("code_file", code_file)).await?;
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
                targets,
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
                                    targets: targets_of(o.targets)?,
                                    keep: targets_of(o.keep)?,
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
                targets: targets_of(targets)?,
                keep: targets_of(keep)?,
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
        Op::CrResolve {
            title,
            resolution,
            reason,
            by,
        } => {
            let done = app
                .resolve_change_request(&title, &resolution.into(), &reason, &by.into())
                .await?;
            json!({ "op": "cr_resolve", "findings": findings_out(&done.findings) })
        }
        Op::CrReopen { title, reason, by } => {
            app.reopen_change_request(&title, &reason, &by.into())
                .await?;
            to_value(&DoneOut { op: "cr_reopen" })?
        }
        Op::Status => {
            let s = app.status().await?;
            to_value(&StatusOut {
                op: "status",
                retired: s.retired.iter().map(RetiredOut::from).collect(),
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
                        situation: c.situation.as_deref(),
                        intent: c.intent,
                        parent: c.parent.as_deref(),
                        issuer: c.issuer.as_deref(),
                        crux: c.crux,
                        supersedes: c.supersedes.as_deref(),
                        state: c.state,
                        resolution: c.resolution.as_ref().map(ResolutionOut::from),
                        history: c.history.iter().map(HistoryOut::from).collect(),
                        targets: c
                            .targets
                            .iter()
                            .map(|t| TargetProgressOut {
                                target: &t.target,
                                progress: t.progress,
                            })
                            .collect(),
                        keep: c
                            .keep
                            .iter()
                            .map(|k| KeepProgressOut {
                                target: &k.target,
                                moved: k.moved,
                            })
                            .collect(),
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
                        work: c
                            .work
                            .iter()
                            .map(|w| WorkOut {
                                store_revision: w.store_revision,
                                moved: &w.moved,
                            })
                            .collect(),
                        out_of_frame: &c.out_of_frame,
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
                measurements: s
                    .measurements
                    .iter()
                    .map(|m| MeasurementOut {
                        context: &m.context,
                        interface: m.interface.as_deref(),
                        harness: m.harness.as_deref(),
                        kind: m.kind,
                        id: &m.id,
                        stamp: &m.stamp,
                        state: m.state,
                        outcome: m.outcome,
                        failure: m.failure.as_deref(),
                        sites: m.coverage.map(|c| c.sites),
                        evaluated: m.coverage.map(|c| c.evaluated),
                        capped: m.coverage.map(|c| c.capped),
                        killed: m.counts.killed,
                        survived: m.counts.survived,
                        stillborn: m.counts.stillborn,
                        timeout: m.counts.timeout,
                        sim: m.sim.map(|x| SimSummaryOut {
                            cases: x.cases,
                            findings: x.findings,
                            unreached: x.unreached,
                            abnormal: x.abnormal,
                            reruns: x.reruns,
                            differing: x.differing,
                        }),
                        at: m.at,
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
        Op::View { sections, contexts } => {
            let read = app
                .view(&ViewInput {
                    contexts,
                    sections: sections.into_iter().map(ModelSection::from).collect(),
                })
                .await?;
            to_value(&ViewOut {
                op: "view",
                store_revision: read.store_revision.0,
                view: &read.view,
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
