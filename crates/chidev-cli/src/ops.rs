//! The operations `chidev exec` reads, one JSON object per line, and the results it prints. This is where the JSON
//! form is turned into the inputs of the use cases and back; nothing past it sees JSON.

use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path};

use chidev_core::app::ports::external::ExternalExecutor;
use chidev_core::app::ports::{IdSource, InvocationField, InvocationSeq, Store};
use chidev_core::app::{
    App, AppError, AssumptionInput, AwaitingOption, BasisInput, BasisStatus, BeyondInput,
    BeyondStatus, CaseInput, ChangeRequestInput, ChangeSetInput, Checked, ClauseInput, ClauseRef,
    ClausesPut, Cleared, ContractPut, CorrectInput, CorrectionStatus, CruxKind, CruxStatus,
    DecisionInput, DecisionMakerInput, DecisionStatus, Diagnostic, EvidenceInput, EvidenceStatus,
    ExampleInput, ExampleTexts, FindingInput, Found, HarnessKind, HarnessPut, HarnessPutContent,
    Importance, Intent, InterfacePut, ItemInput, ItemRef, ItemsInput, MeasurementState, ModelSlice,
    ModelView, MovedPart, OpenedExpect, OptionInput, ParamValue, PinState, Place, PolicyLineInput,
    PutDone, PutInput, RaiseCruxInput, Raised, RecommendationInput, RecommendedStatus, Remaining,
    RenameInput, ReportInput, ReportedFinding, ResolutionInput, ResolutionStatus, RetireInput,
    RetiredStatus, Retirement, RuleCard, RuleStanding, RuleToJudge, SubjectInput, TargetInput,
    TargetRef, TransitionStatus, ViewInput,
};
use chidev_core::domain::check::{Level, Moment, renamed_key};
use chidev_core::domain::entity::change_request::ChangeRequestField;
use chidev_core::domain::entity::crux::CruxField;
use chidev_core::domain::entity::measurement::MeasurementField;
use chidev_core::domain::vo::id::{ChangeRequestId, CruxId, CruxOptionId, MeasurementId};
use chidev_core::domain::vo::measure::MeasureKind;
use chidev_core::domain::vo::model_section::ModelSection;
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::infra::accept;
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

mod diagnostic;
use diagnostic::DiagnosticOut;

/// One operation. `op` names it; the other fields are its arguments.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    /// Makes the project.
    Init,
    /// Adds a Context.
    ContextAdd {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        uses: Vec<String>,
    },
    /// Sets the Contexts a Context uses.
    ContextUses { context: String, uses: Vec<String> },
    /// Bases a Context on its Contracts.
    BaseOnContract { context: String },
    /// Makes or revises a Contract: `clauses` are all of the Clauses it has afterwards, in order. A Clause keeps its
    /// id when its label is one the Contract has, or when it `replaces` one (the label it had before, now given a
    /// new one: its citations stay, and a new label alone moves no revision). To change some Clauses and leave the
    /// rest as they are, use `clauses_put`.
    ContractPut {
        context: String,
        name: String,
        title: String,
        clauses: Vec<ClauseDto>,
    },
    /// Changes some Clauses of a Contract that exists, and leaves the others, the order and the title as they are.
    /// A Clause in `clauses` whose label the Contract has gets the sentence given; one that `replaces` a label the
    /// Contract has takes that Clause's place with the new label and the sentence; any other is new and comes after
    /// the Clauses the Contract has. `remove` lists the labels of Clauses to remove.
    ClausesPut {
        context: String,
        contract: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        clauses: Vec<ClauseDto>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remove: Vec<String>,
    },
    /// Makes or revises an Interface. The code is `source`, or the text of the file `source_file` (relative to the
    /// project's root); exactly one is given. `items` may be left out: the provider of the language its
    /// `[[measure.layout]]` gives finds them in the code, and `renames` (`{"from","to"}`) hand the id of the Item
    /// that had the key `from` to the one found under `to`. With `items`, nothing is found and `renames` is refused.
    InterfacePut {
        context: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items: Option<Vec<ItemDto>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        renames: Vec<RenameDto>,
    },
    /// Gives the Items of an Interface the keys and lines the provider of its language finds in the code the
    /// Interface keeps (not the working tree). An Item is paired with a declaration when their lines overlap and each
    /// overlaps no other of the other side; the rest stay as they are and are listed in `unmatched`. The ids stay, so
    /// the citations do; `revised` lists the Items whose new lines declare something else, which go to their next
    /// revision. Not a change set: nothing else is put, and a store that moved meanwhile refuses it.
    ItemsRekey { context: String, interface: String },
    /// Makes or revises a Harness. `kind` is `tests` (the default for a new Harness), `sim` or `examples`; an
    /// existing Harness keeps its kind when none is given, except one of examples, which must be told (a put of code
    /// to it without a kind is refused). `builds` (a sim, or a Harness of examples) names the Interfaces of the
    /// Context whose pinned code the program is built from; a put states all of it.
    ///
    /// Code (`tests` or `sim`): the source is `source`, or the text of the file `source_file`; exactly one is given.
    /// Every Case has `lines`, and no Case has `example`; there is no `examples_dir`. A Case of tests cites an Item
    /// at least, a Case of a sim cites Clauses only.
    ///
    /// Examples: there is no source and no Case has `lines` or `items`. The examples are in `examples_dir`, a
    /// directory relative to the project's root with `<name>.args`, `<name>.stdin`, `<name>.expected` (the output,
    /// then a last line `#exit <code>`) and optionally `<name>.stderr` for each example, and every Case is named
    /// for one of them; or every Case has an `example` `{args, stdin, expected, stderr?}` with the same texts. Exactly
    /// one of the two is used. The texts are read here and kept in the model, never the directory, so a change of one
    /// is a new revision of its Case and of the Harness. A Case cites Clauses only, one at least.
    HarnessPut {
        context: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        builds: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        examples_dir: Option<String>,
        cases: Vec<CaseDto>,
    },
    /// Puts and retires several assets as one change: the rules of the project are checked once, on the state it
    /// ends in, so a Harness can be put without the Cases that cite a Clause the same change removes, and can cite
    /// a Clause the same change makes. All of it is applied, or none (and the error names every problem). Each
    /// entry of `puts` is tagged with `put` (`contract`, `clauses`, `interface`, `harness`, `uses`,
    /// `base_on_contract`) and
    /// has the fields of the operation of that name without `op`; each entry of `retires` is tagged with `retire`
    /// (`context`, `contract`, `interface`, `harness`). `for` is the title of the change request the work is
    /// applied for: the targets the set moves are recorded as that request's work in the same commit, and `status`
    /// reads from such work whether the request's targets were revised or retired. Without `for` the set belongs
    /// to no request.
    ChangeSet {
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        for_change_request: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        puts: Vec<PutDto>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code_file: Option<String>,
    },
    /// Opens a change request.
    CrOpen(Box<OpenDto>),
    /// Replaces the decision Policy (the numbered lines `P1`, `P2`, ... that settle a choice without asking a
    /// decision maker) with `lines`, whole.
    PolicyPut { lines: Vec<PolicyLineDto> },
    /// Raises a Crux in the change request titled `in`: a question only a decision maker can answer (`question`, of
    /// a `kind`), the `options` (each may name the `targets` it asks to be revised or retired, and the `keep` it asks
    /// to stay as they are, once it is chosen; at most one is `recommended`), the `findings` it is raised from
    /// (written like the `addresses` of `cr_open`), how much it matters (`importance`), and the Crux it replaces
    /// (`supersedes`, an id; that one must exist, have a decision to replace, and be replaced by no other Crux that
    /// is not withdrawn). The change request cannot be resolved until
    /// the Crux is decided. A resolved change request takes none. The result has the id of the Crux and of each
    /// option, which `cr_crux_decide` points at.
    CrCruxRaise(Box<RaiseDto>),
    /// Decides a Crux, once: the `crux` (an id), exactly one of `option` (an option id of the Crux; with an optional
    /// `reason`), `wording` (the behaviour in the decision maker's words; with an optional `reason`), `unspecified`
    /// (what must still hold, for a behaviour left free) and `withdrawn: true` (the question is not needed any more),
    /// the `basis` it rests on (a Policy line or an arbitrary reason), and who decides (`by`, a claim that nothing
    /// checks). The targets the option chosen asks for are recorded in the change request the Crux was raised in
    /// when they are inside its frame. The ones outside it are not added to it: `beyond` says where they go,
    /// `{"requests":["<title>"]}` (the change requests opened for them, each framing them) or `{"declined":"<why>"}`.
    CrCruxDecide(Box<DecideDto>),
    /// Appends an Assumption to a change request that is not resolved.
    CrAssume {
        title: String,
        statement: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        evidence: Vec<EvidenceDto>,
    },
    /// Closes the record of a change request with a resolution: `resolution` is `"done"`, `"wont_do"`,
    /// `"invalid"`, `{"duplicate":"<title>"}` or `{"superseded":"<title>"}` (the title of a change request that
    /// exists), with the `reason` and who resolves it (`by`). Refused only by explicit states: resolved already,
    /// a Crux raised in it not decided (whatever the resolution), a part not resolved, a reference that does not
    /// exist. The targets and the work are not consulted; checks report where they disagree.
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interface: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// Runs every check on the project as it is; nothing is kept. With a `context`, the findings are those about
    /// that Context (what it holds, and the change requests that revise, retire or keep something in it), and
    /// `outside` is how many findings were left out (the Project's own, and change requests that name nothing of
    /// the Context). The whole run is made either way.
    Check {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<String>,
    },
    /// Explicitly executes external check-moment programs and records this run.
    CheckRun,
    /// Lists every check with the level it runs at.
    CheckList,
    /// The judged rules to judge (those of `moment`, or the one `rule`; all when neither is given), with a view of
    /// the model to judge them on. The agent that drives chidev judges; chidev keeps the results.
    CheckRules {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        moment: Option<String>,
        /// Only to be refused: the argument was `hook` before it was `moment`. Without the field the operation
        /// would run with no moment given, which is every rule. Kept apart from null, so `"hook": null` is refused
        /// too.
        #[serde(default, deserialize_with = "present", skip_serializing)]
        #[schemars(skip)]
        hook: Option<Option<serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule: Option<String>,
    },
    /// What an agent judged of one rule: replaces every finding the rule had.
    FindingReport(Box<ReportDto>),
    /// Reads the part of the model that is asked for: the `sections` named (at least one; nothing is read by
    /// default), of the Contexts named in `contexts` (every Context when it is left out; none when it is `[]`). The
    /// Contexts always come with their `id`, `name`, `basis` and `uses`; the change requests and the Cruxes belong
    /// to the Project, so they come when `change_requests` and `cruxes` are sections, whatever `contexts` says. The
    /// result is the `view`, and the `store_revision` it was read at.
    View(ViewDto),
    /// Lists where a text is kept in the records only a correction can change: (record, field) for each place,
    /// never the text. A record is named by its id, and every field that holds the text is listed, so each place
    /// is one `record_correct` can replace it in. Nothing is kept.
    TextFind { text: SoughtDto },
    /// Replaces a text with a mark in each place named, keeps the correction as a step of each record's history,
    /// and clears what the store file kept of it. `places` are those `text_find` lists for the text (at least
    /// one, none twice); `reason` and `by` are kept in each record and do not hold the text. It is forced: no
    /// check runs and none refuses it. The result lists the places `corrected`, the places that `remaining` hold
    /// the text (`{"unknown":"<why>"}` when they could not be searched for), and whether the file was `cleared`
    /// (`"done"`, or `{"not_finished":"<why>"}` when the correction is kept and the file could not be cleared).
    /// Once the correction is kept, the result is never a refusal.
    RecordCorrect {
        text: SoughtDto,
        reason: String,
        by: MakerDto,
        places: Vec<PlaceDto>,
    },
}

/// The fields of `view`. A field it does not know is refused: a misspelled `contexts` would otherwise read every
/// Context, more than the reader was to be given.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewDto {
    sections: Vec<SectionDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contexts: Option<Vec<String>>,
}

/// One entry of the `puts` of a `change_set`: the fields of the operation named by `put`, without `op`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "put", rename_all = "snake_case", deny_unknown_fields)]
enum PutDto {
    /// As `context_add`.
    Context {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    /// As `clauses_put`.
    Clauses {
        context: String,
        contract: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        clauses: Vec<ClauseDto>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remove: Vec<String>,
    },
    /// As `interface_put`.
    Interface {
        context: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items: Option<Vec<ItemDto>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        renames: Vec<RenameDto>,
    },
    /// As `harness_put`.
    Harness {
        context: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        builds: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        examples_dir: Option<String>,
        cases: Vec<CaseDto>,
    },
}

/// One entry of the `retires` of a `change_set`: the fields of the retire operation named by `retire`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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

/// An entry of the `puts` of a change set as the input of the use case; the text of a `source_file` is read here, and
/// the name of an asset is read as one (a text that breaks the rule of a name is refused, naming the character).
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
            name: AssetName::try_from(name)?,
            title,
            clauses: clause_inputs(clauses),
        }),
        PutDto::Clauses {
            context,
            contract,
            clauses,
            remove,
        } => PutInput::Clauses(ClausesPut {
            context,
            contract: AssetName::try_from(contract)?,
            clauses: clause_inputs(clauses),
            remove,
        }),
        PutDto::Interface {
            context,
            name,
            source,
            source_file,
            items,
            renames,
        } => {
            let name = AssetName::try_from(name)?;
            let label = format!("the put of the interface `{name}`");
            let source = text_of(
                root,
                &label,
                ("source", source),
                ("source_file", source_file),
            )
            .await?;
            let items = items_input(&context, name.as_str(), items, renames)?;
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
            examples_dir,
            cases,
        } => {
            let name = AssetName::try_from(name)?;
            let builds = builds
                .iter()
                .map(|n| AssetName::try_from(n.as_str()))
                .collect::<Result<Vec<_>, _>>()?;
            let label = format!("the put of the harness `{name}`");
            let content = harness_content(
                root,
                &label,
                HarnessText {
                    kind,
                    source,
                    source_file,
                    examples_dir,
                },
                cases,
            )
            .await?;
            PutInput::Harness(HarnessPut {
                context,
                name,
                builds,
                content,
            })
        }
    })
}

/// What a put of a Harness says of its content outside its Cases: the kind, and where the code or the examples are.
struct HarnessText {
    kind: Option<String>,
    source: Option<String>,
    source_file: Option<String>,
    examples_dir: Option<String>,
}

/// What a put of a Harness holds, from the fields that say it. The kind decides which fields belong: a Harness of
/// examples has no source and no Case on lines or citing an Item, and takes its texts from `examples_dir` or from
/// each Case's `example`, one of the two; any other has code, every Case on lines, and no `examples_dir` or `example`.
/// Every field that does not belong, and every example that has no Case or Case that has no example, is named in
/// the error. A directory is read here, at the boundary, so the use case receives text and never a path.
async fn harness_content(
    root: &Path,
    label: &str,
    text: HarnessText,
    cases: Vec<CaseDto>,
) -> Result<HarnessPutContent, AppError> {
    let HarnessText {
        kind,
        source,
        source_file,
        examples_dir,
    } = text;
    let kind = harness_kind(kind.as_deref())?;
    let mut problems = Vec::new();
    if kind == Some(HarnessKind::Examples) {
        if source.is_some() || source_file.is_some() {
            problems.push(
                "a harness of examples has no source: leave out `source` and `source_file`"
                    .to_string(),
            );
        }
        for c in &cases {
            if c.lines.is_some() {
                problems.push(format!(
                    "the case `{}` has `lines`, but an example is on no lines",
                    c.name
                ));
            }
            if !c.items.is_empty() {
                problems.push(format!(
                    "the case `{}` cites items, but an example cites clauses only",
                    c.name
                ));
            }
        }
        let loaded = match &examples_dir {
            Some(dir) => {
                for c in cases.iter().filter(|c| c.example.is_some()) {
                    problems.push(format!(
                        "the case `{}` has an `example` and `examples_dir` is given: use one of the two",
                        c.name
                    ));
                }
                load_examples(root, dir, &cases, &mut problems).await
            }
            None => {
                for c in cases.iter().filter(|c| c.example.is_none()) {
                    problems.push(format!(
                        "the case `{}` has no `example`: give each case an `example`, or `examples_dir`",
                        c.name
                    ));
                }
                Vec::new()
            }
        };
        if !problems.is_empty() {
            return Err(refusal(label, problems));
        }
        let mut examples = Vec::with_capacity(cases.len());
        for c in cases {
            let example = match c.example {
                Some(e) => ExampleTexts {
                    args: e.args,
                    stdin: e.stdin,
                    expected: e.expected,
                    stderr: e.stderr,
                },
                None => match loaded.iter().find(|(name, _)| name == c.name.trim()) {
                    Some((_, texts)) => texts.clone(),
                    // found above, with the rest of the problems; not reachable once there are none
                    None => {
                        return Err(AppError::Input(format!(
                            "{label}: the case `{}` has no example",
                            c.name
                        )));
                    }
                },
            };
            examples.push(ExampleInput {
                name: c.name,
                clauses: c
                    .clauses
                    .into_iter()
                    .map(ClauseRef::try_from)
                    .collect::<Result<_, _>>()?,
                example,
            });
        }
        return Ok(HarnessPutContent::Examples(examples));
    }
    if examples_dir.is_some() {
        problems.push(format!(
            "`examples_dir` is for a harness of examples, and this one is of {}: give `kind` `examples`",
            kind.unwrap_or_default().word()
        ));
    }
    for c in &cases {
        if c.example.is_some() {
            problems.push(format!(
                "the case `{}` has an `example`, which only a harness of examples has: give `kind` `examples`",
                c.name
            ));
        }
        if c.lines.is_none() {
            problems.push(format!(
                "the case `{}` has no `lines`: a case of code is on the lines of its code",
                c.name
            ));
        }
    }
    if !problems.is_empty() {
        return Err(refusal(label, problems));
    }
    let source = text_of(
        root,
        label,
        ("source", source),
        ("source_file", source_file),
    )
    .await?;
    Ok(HarnessPutContent::Code {
        kind: kind.and_then(HarnessKind::code),
        source,
        cases: case_inputs(cases)?,
    })
}

/// One refusal for the problems of an input: the only one as it is, several as one that names every problem.
fn refusal(label: &str, problems: Vec<String>) -> AppError {
    let mut all: Vec<AppError> = problems
        .into_iter()
        .map(|p| AppError::Input(format!("{label}: {p}")))
        .collect();
    if all.len() == 1 {
        all.remove(0)
    } else {
        AppError::Many(all)
    }
}

/// The texts of the examples in the directory `dir` (relative to `root`), by name, for the Cases `cases`. The
/// problems found are added to `problems`: a directory that cannot be read, an example that is not whole, an example
/// with no Case and a Case with no example in the directory.
async fn load_examples(
    root: &Path,
    dir: &str,
    cases: &[CaseDto],
    problems: &mut Vec<String>,
) -> Vec<(String, ExampleTexts)> {
    let relative = match relative_path(dir) {
        Ok(relative) => relative,
        Err(why) => {
            problems.push(format!("`examples_dir` `{dir}` {why}"));
            return Vec::new();
        }
    };
    let loaded = match accept::load(&root.join(relative)).await {
        Ok(loaded) => loaded,
        Err(e) => {
            problems.push(format!("`examples_dir` `{dir}`: {e}"));
            return Vec::new();
        }
    };
    let named: Vec<&str> = cases.iter().map(|c| c.name.trim()).collect();
    for l in &loaded {
        if !named.contains(&l.name.as_str()) {
            problems.push(format!(
                "the example `{}` in `{dir}` has no case: add a case named `{}`, or take its files out",
                l.name, l.name
            ));
        }
    }
    for name in &named {
        if !loaded.iter().any(|l| l.name.as_str() == *name) {
            problems.push(format!(
                "the case `{name}` has no example in `{dir}`: its files are {name}.args, {name}.stdin and {name}.expected"
            ));
        }
    }
    loaded
        .into_iter()
        .map(|l| {
            (
                l.name.to_string(),
                ExampleTexts {
                    args: l.args,
                    stdin: l.stdin,
                    expected: l.expected,
                    stderr: l.stderr,
                },
            )
        })
        .collect()
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

impl TryFrom<RetireDto> for RetireInput {
    type Error = AppError;

    fn try_from(d: RetireDto) -> Result<RetireInput, AppError> {
        Ok(match d {
            RetireDto::Context { context } => RetireInput::Context(context),
            RetireDto::Contract { context, contract } => RetireInput::Contract {
                context,
                contract: AssetName::try_from(contract)?,
            },
            RetireDto::Interface { context, interface } => RetireInput::Interface {
                context,
                interface: AssetName::try_from(interface)?,
            },
            RetireDto::Harness { context, harness } => RetireInput::Harness {
                context,
                harness: AssetName::try_from(harness)?,
            },
        })
    }
}

fn clause_inputs(clauses: Vec<ClauseDto>) -> Vec<ClauseInput> {
    clauses
        .into_iter()
        .map(|c| ClauseInput {
            label: c.label,
            text: c.text,
            replaces: c.replaces,
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
            context: ContextName::try_from(context)?,
            interface: AssetName::try_from(interface)?,
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

/// The Cases of code. A Case with no `lines` is not one, and is refused (see [`harness_content`], which names every
/// such Case before this is asked).
fn case_inputs(cases: Vec<CaseDto>) -> Result<Vec<CaseInput>, AppError> {
    cases
        .into_iter()
        .map(|c| {
            let [first, last] = c
                .lines
                .ok_or_else(|| AppError::Input(format!("the case `{}` has no `lines`", c.name)))?;
            Ok(CaseInput {
                name: c.name,
                lines: (first, last),
                items: c
                    .items
                    .into_iter()
                    .map(ItemRef::try_from)
                    .collect::<Result<_, _>>()?,
                clauses: c
                    .clauses
                    .into_iter()
                    .map(ClauseRef::try_from)
                    .collect::<Result<_, _>>()?,
            })
        })
        .collect()
}

fn harness_kind(kind: Option<&str>) -> Result<Option<HarnessKind>, AppError> {
    kind.map(|w| {
        HarnessKind::from_word(w).ok_or_else(|| {
            let words: Vec<&str> = HarnessKind::ALL.iter().map(|k| k.word()).collect();
            let listed = match words.split_last() {
                Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
                _ => words.join(""),
            };
            AppError::Input(format!("the harness kind `{w}` is not one; it is {listed}"))
        })
    })
    .transpose()
}

/// A part of the model a `view` reads, written as its word. The words are the domain's (`ModelSection::word`), so a
/// section added there can be asked for here without a second list to keep in step.
#[derive(Debug, Clone, Copy)]
struct SectionDto(ModelSection);

impl<'de> Deserialize<'de> for SectionDto {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<SectionDto, D::Error> {
        let word = String::deserialize(d)?;
        ModelSection::from_word(&word)
            .map(SectionDto)
            .ok_or_else(|| {
                // Serde's own wording, so the diagnostic reads it as an unknown variant and suggests the near word.
                let expected = ModelSection::ALL
                    .iter()
                    .map(|s| format!("`{}`", s.word()))
                    .collect::<Vec<_>>()
                    .join(", ");
                serde::de::Error::custom(format!(
                    "unknown variant `{word}`, expected one of {expected}"
                ))
            })
    }
}

impl JsonSchema for SectionDto {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SectionDto".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let words: Vec<&str> = ModelSection::ALL.iter().map(|s| s.word()).collect();
        schemars::json_schema!({ "type": "string", "enum": words })
    }
}

impl Serialize for SectionDto {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.word())
    }
}

impl From<SectionDto> for ModelSection {
    fn from(s: SectionDto) -> ModelSection {
        s.0
    }
}

/// What a `measure` measures.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
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

/// A part of what a measurement stands on that moved, in the result of `status`.
/// Each shape has exactly the fields it says, and none is left out by a flag.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MovedOut<'a> {
    /// A part about an asset: its word and the asset's name.
    Asset { part: &'static str, name: &'a str },
    /// Settings the runner cannot read: the word and the runner's reason.
    Unreadable { part: &'static str, reason: &'a str },
    /// Any other part: its word.
    Plain { part: &'static str },
}

impl<'a> From<&'a MovedPart> for MovedOut<'a> {
    fn from(moved: &'a MovedPart) -> MovedOut<'a> {
        let part = moved.part().word();
        match (moved.name(), moved.part().reason()) {
            (Some(name), _) => MovedOut::Asset {
                part,
                name: name.as_str(),
            },
            (None, Some(reason)) => MovedOut::Unreadable { part, reason },
            (None, None) => MovedOut::Plain { part },
        }
    }
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
    /// `current`, `stale`, `retired` or `unpinned`.
    state: &'static str,
    /// The parts that moved, when it is stale: each by its word, with the name of the asset when it is about one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    moved: Vec<MovedOut<'a>>,
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
    /// The corrections made of its texts, in the order they were made; left out when there is none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    corrections: Vec<CorrectionOut<'a>>,
}

/// The fields of `finding_report`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ReportDto {
    rule: String,
    /// The rule digest returned by check_rules, echoed back. Required: a missing or null one is refused.
    rule_digest: String,
    /// The agent that judged.
    judge: String,
    /// The store revision of the view it judged from (`store_revision` of `check_rules`).
    read_at: u64,
    /// Every finding of the rule; empty: it found nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    findings: Vec<ReportedFindingDto>,
}

/// A finding in a report.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ReportedFindingDto {
    /// What it is about; none: the Project as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject: Option<SubjectDto>,
    message: String,
    /// `note`, `warning` or `error`, not above the rule's level; none: the rule's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level: Option<String>,
}

/// What a finding is about, tagged by its kind: `{ "clause": { ... } }`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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

impl TryFrom<SubjectDto> for SubjectInput {
    type Error = AppError;

    fn try_from(s: SubjectDto) -> Result<SubjectInput, AppError> {
        Ok(match s {
            SubjectDto::Context(name) => SubjectInput::Context(ContextName::try_from(name)?),
            SubjectDto::Interface { context, interface } => SubjectInput::Interface {
                context: ContextName::try_from(context)?,
                interface: AssetName::try_from(interface)?,
            },
            SubjectDto::Item(r) => SubjectInput::Item(ItemRef::try_from(r)?),
            SubjectDto::Clause(r) => SubjectInput::Clause(ClauseRef::try_from(r)?),
            SubjectDto::Case {
                context,
                harness,
                name,
            } => SubjectInput::Case {
                context: ContextName::try_from(context)?,
                harness: AssetName::try_from(harness)?,
                name,
            },
            SubjectDto::ChangeRequest(title) => SubjectInput::ChangeRequest(title),
        })
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
                        subject: f.subject.map(SubjectInput::try_from).transpose()?,
                        message: f.message,
                        level: f.level.as_deref().map(level_of).transpose()?,
                    })
                })
                .collect::<Result<_, AppError>>()?,
        })
    }
}

/// How a change request ended: a word, or a word that points at another change request by its title.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenDto {
    title: String,
    /// Why it is needed, in the words of whoever raised it. Kept with the request and shown by `status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    situation: Option<String>,
    intent: IntentDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    /// Who raised it. Kept with the request and shown by `status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    /// The targets it asks to be revised or retired. A parent need not name any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    targets: Vec<RefDto>,
    /// The targets it asks to stay as they are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keep: Vec<RefDto>,
    /// The interpretations settled from the repository and general knowledge while raising it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    assumptions: Vec<AssumptionDto>,
    /// The findings it deals with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    addresses: Vec<AddressDto>,
}

/// An Assumption: what is assumed, why it holds, and what it stands on.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssumptionDto {
    statement: String,
    reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence: Vec<EvidenceDto>,
}

/// What an Assumption stands on: a place in the repository, or general knowledge.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum EvidenceDto {
    Repo {
        path: String,
        #[serde(
            default,
            deserialize_with = "maybe_two_lines",
            skip_serializing_if = "Option::is_none"
        )]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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

/// The text a correction seeks. It is a secret: `Debug` hides it, so that a print of an operation does not carry
/// it, and nothing here writes it into a result.
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(transparent)]
struct SoughtDto(String);

impl fmt::Debug for SoughtDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SoughtDto(..)")
    }
}

/// A place where a text is kept, as `text_find` answers it and `record_correct` takes it: a record by its id, and a
/// field of it. `{"change_request":{"id":"<uuid>","field":"situation"}}`, `{"crux":{"id":"<uuid>","field":{"option_choice":"<option uuid>"}}}`,
/// `{"measurement":{"id":"<uuid>","field":{"case_finding":{"case":0,"finding":1}}}}`,
/// `{"invocation":{"seq":3,"field":"stderr"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum PlaceDto {
    /// A field of a change request (`id` is its id, not its title).
    ChangeRequest { id: String, field: CrFieldDto },
    /// A field of a Crux.
    Crux { id: String, field: CruxFieldDto },
    /// A field of a measurement (`id` is the id `measure` and `simulate` answer).
    Measurement {
        id: String,
        field: MeasurementFieldDto,
    },
    /// A field of a row of the record of check runs (`seq` is the order the rows were kept in).
    Invocation { seq: u64, field: InvocationFieldDto },
}

/// A field of a change request. An index counts from 0 in the list it is of; a step is an index into the history
/// (`status` lists it, corrections included).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum CrFieldDto {
    Title,
    Situation,
    Issuer,
    AssumptionStatement(usize),
    AssumptionReason(usize),
    AssumptionEvidence { assumption: usize, evidence: usize },
    StepReason(usize),
    StepBy(usize),
}

/// A field of a Crux. An option is named by its id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum CruxFieldDto {
    Question,
    OptionChoice(String),
    OptionChange(String),
    OptionConsequence(String),
    RecommendationReason,
    DecisionReason,
    DecisionWording,
    DecisionKeep(usize),
    BasisReason,
    BeyondReason,
    AnswerBy,
}

/// A field of a measurement. An index counts from 0 in the list it is of; a case is an index into the cases of a
/// simulation, in the order they were run. `case_exit` is the reason a case could not start the program.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum MeasurementFieldDto {
    StepCommand(usize),
    StepStderr(usize),
    Failure,
    MutantFrom(usize),
    MutantTo(usize),
    CaseFinding { case: usize, finding: usize },
    CaseUnreached { case: usize, unreached: usize },
    CaseUndecided { case: usize, undecided: usize },
    CaseExit(usize),
}

/// A field of a row of the record of check runs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum InvocationFieldDto {
    Command,
    Stderr,
    Judge,
}

/// The uuid an id is written as. What is not one names nothing, which is a mistake of the input.
fn uuid_of(what: &str, text: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(text.trim())
        .map_err(|_| AppError::Input(format!("`{text}` is not the id of {what}")))
}

impl TryFrom<PlaceDto> for Place {
    type Error = AppError;

    fn try_from(place: PlaceDto) -> Result<Place, AppError> {
        Ok(match place {
            PlaceDto::ChangeRequest { id, field } => Place::ChangeRequest {
                id: ChangeRequestId::from_uuid(uuid_of("a change request", &id)?),
                field: field.into(),
            },
            PlaceDto::Crux { id, field } => Place::Crux {
                id: CruxId::from_uuid(uuid_of("a crux", &id)?),
                field: field.try_into()?,
            },
            PlaceDto::Measurement { id, field } => Place::Measurement {
                id: MeasurementId::from_uuid(uuid_of("a measurement", &id)?),
                field: field.into(),
            },
            PlaceDto::Invocation { seq, field } => Place::Invocation {
                seq: InvocationSeq(seq),
                field: field.into(),
            },
        })
    }
}

impl From<CrFieldDto> for ChangeRequestField {
    fn from(f: CrFieldDto) -> ChangeRequestField {
        match f {
            CrFieldDto::Title => ChangeRequestField::Title,
            CrFieldDto::Situation => ChangeRequestField::Situation,
            CrFieldDto::Issuer => ChangeRequestField::Issuer,
            CrFieldDto::AssumptionStatement(assumption) => {
                ChangeRequestField::AssumptionStatement { assumption }
            }
            CrFieldDto::AssumptionReason(assumption) => {
                ChangeRequestField::AssumptionReason { assumption }
            }
            CrFieldDto::AssumptionEvidence {
                assumption,
                evidence,
            } => ChangeRequestField::AssumptionEvidence {
                assumption,
                evidence,
            },
            CrFieldDto::StepReason(step) => ChangeRequestField::StepReason { step },
            CrFieldDto::StepBy(step) => ChangeRequestField::StepBy { step },
        }
    }
}

impl TryFrom<CruxFieldDto> for CruxField {
    type Error = AppError;

    fn try_from(f: CruxFieldDto) -> Result<CruxField, AppError> {
        let option = |id: &str| uuid_of("a crux option", id).map(CruxOptionId::from_uuid);
        Ok(match f {
            CruxFieldDto::Question => CruxField::Question,
            CruxFieldDto::OptionChoice(id) => CruxField::OptionChoice {
                option: option(&id)?,
            },
            CruxFieldDto::OptionChange(id) => CruxField::OptionChange {
                option: option(&id)?,
            },
            CruxFieldDto::OptionConsequence(id) => CruxField::OptionConsequence {
                option: option(&id)?,
            },
            CruxFieldDto::RecommendationReason => CruxField::RecommendationReason,
            CruxFieldDto::DecisionReason => CruxField::DecisionReason,
            CruxFieldDto::DecisionWording => CruxField::DecisionWording,
            CruxFieldDto::DecisionKeep(keep) => CruxField::DecisionKeep { keep },
            CruxFieldDto::BasisReason => CruxField::BasisReason,
            CruxFieldDto::BeyondReason => CruxField::BeyondReason,
            CruxFieldDto::AnswerBy => CruxField::AnswerBy,
        })
    }
}

impl From<MeasurementFieldDto> for MeasurementField {
    fn from(f: MeasurementFieldDto) -> MeasurementField {
        match f {
            MeasurementFieldDto::StepCommand(step) => MeasurementField::StepCommand { step },
            MeasurementFieldDto::StepStderr(step) => MeasurementField::StepStderr { step },
            MeasurementFieldDto::Failure => MeasurementField::Failure,
            MeasurementFieldDto::MutantFrom(mutant) => MeasurementField::MutantFrom { mutant },
            MeasurementFieldDto::MutantTo(mutant) => MeasurementField::MutantTo { mutant },
            MeasurementFieldDto::CaseFinding { case, finding } => {
                MeasurementField::CaseFinding { case, finding }
            }
            MeasurementFieldDto::CaseUnreached { case, unreached } => {
                MeasurementField::CaseUnreached { case, unreached }
            }
            MeasurementFieldDto::CaseUndecided { case, undecided } => {
                MeasurementField::CaseUndecided { case, undecided }
            }
            MeasurementFieldDto::CaseExit(case) => MeasurementField::CaseExit { case },
        }
    }
}

impl From<InvocationFieldDto> for InvocationField {
    fn from(f: InvocationFieldDto) -> InvocationField {
        match f {
            InvocationFieldDto::Command => InvocationField::Command,
            InvocationFieldDto::Stderr => InvocationField::Stderr,
            InvocationFieldDto::Judge => InvocationField::Judge,
        }
    }
}

impl From<&Place> for PlaceDto {
    fn from(place: &Place) -> PlaceDto {
        match place {
            Place::ChangeRequest { id, field } => PlaceDto::ChangeRequest {
                id: id.to_string(),
                field: field.into(),
            },
            Place::Crux { id, field } => PlaceDto::Crux {
                id: id.to_string(),
                field: field.into(),
            },
            Place::Measurement { id, field } => PlaceDto::Measurement {
                id: id.to_string(),
                field: field.into(),
            },
            Place::Invocation { seq, field } => PlaceDto::Invocation {
                seq: seq.0,
                field: field.into(),
            },
        }
    }
}

impl From<&ChangeRequestField> for CrFieldDto {
    fn from(f: &ChangeRequestField) -> CrFieldDto {
        match *f {
            ChangeRequestField::Title => CrFieldDto::Title,
            ChangeRequestField::Situation => CrFieldDto::Situation,
            ChangeRequestField::Issuer => CrFieldDto::Issuer,
            ChangeRequestField::AssumptionStatement { assumption } => {
                CrFieldDto::AssumptionStatement(assumption)
            }
            ChangeRequestField::AssumptionReason { assumption } => {
                CrFieldDto::AssumptionReason(assumption)
            }
            ChangeRequestField::AssumptionEvidence {
                assumption,
                evidence,
            } => CrFieldDto::AssumptionEvidence {
                assumption,
                evidence,
            },
            ChangeRequestField::StepReason { step } => CrFieldDto::StepReason(step),
            ChangeRequestField::StepBy { step } => CrFieldDto::StepBy(step),
        }
    }
}

impl From<&CruxField> for CruxFieldDto {
    fn from(f: &CruxField) -> CruxFieldDto {
        match *f {
            CruxField::Question => CruxFieldDto::Question,
            CruxField::OptionChoice { option } => CruxFieldDto::OptionChoice(option.to_string()),
            CruxField::OptionChange { option } => CruxFieldDto::OptionChange(option.to_string()),
            CruxField::OptionConsequence { option } => {
                CruxFieldDto::OptionConsequence(option.to_string())
            }
            CruxField::RecommendationReason => CruxFieldDto::RecommendationReason,
            CruxField::DecisionReason => CruxFieldDto::DecisionReason,
            CruxField::DecisionWording => CruxFieldDto::DecisionWording,
            CruxField::DecisionKeep { keep } => CruxFieldDto::DecisionKeep(keep),
            CruxField::BasisReason => CruxFieldDto::BasisReason,
            CruxField::BeyondReason => CruxFieldDto::BeyondReason,
            CruxField::AnswerBy => CruxFieldDto::AnswerBy,
        }
    }
}

impl From<&MeasurementField> for MeasurementFieldDto {
    fn from(f: &MeasurementField) -> MeasurementFieldDto {
        match *f {
            MeasurementField::StepCommand { step } => MeasurementFieldDto::StepCommand(step),
            MeasurementField::StepStderr { step } => MeasurementFieldDto::StepStderr(step),
            MeasurementField::Failure => MeasurementFieldDto::Failure,
            MeasurementField::MutantFrom { mutant } => MeasurementFieldDto::MutantFrom(mutant),
            MeasurementField::MutantTo { mutant } => MeasurementFieldDto::MutantTo(mutant),
            MeasurementField::CaseFinding { case, finding } => {
                MeasurementFieldDto::CaseFinding { case, finding }
            }
            MeasurementField::CaseUnreached { case, unreached } => {
                MeasurementFieldDto::CaseUnreached { case, unreached }
            }
            MeasurementField::CaseUndecided { case, undecided } => {
                MeasurementFieldDto::CaseUndecided { case, undecided }
            }
            MeasurementField::CaseExit { case } => MeasurementFieldDto::CaseExit(case),
        }
    }
}

impl From<&InvocationField> for InvocationFieldDto {
    fn from(f: &InvocationField) -> InvocationFieldDto {
        match f {
            InvocationField::Command => InvocationFieldDto::Command,
            InvocationField::Stderr => InvocationFieldDto::Stderr,
            InvocationField::Judge => InvocationFieldDto::Judge,
        }
    }
}

/// The result of `text_find`: the places that hold the text, never the text.
#[derive(Debug, Serialize)]
struct PlacesOut {
    places: Vec<PlaceDto>,
}

/// Whether the store's file was cleared: `"done"`, or `{"not_finished":"<why>"}`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ClearedOut<'a> {
    Done,
    NotFinished(&'a str),
}

/// The places that still hold the text: the list when the search was made, `{"unknown":"<why>"}` when it could not
/// be (the correction is kept either way).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum RemainingOut<'a> {
    Found(Vec<PlaceDto>),
    Unknown { unknown: &'a str },
}

/// The result of `record_correct`: the places corrected, the places that still hold the text, and the clearing.
#[derive(Debug, Serialize)]
struct CorrectOut<'a> {
    corrected: Vec<PlaceDto>,
    remaining: RemainingOut<'a>,
    cleared: ClearedOut<'a>,
}

/// One line of the decision Policy.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PolicyLineDto {
    id: String,
    text: String,
}

/// A finding a change request addresses. New and legacy input forms normalize at the CLI boundary.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(try_from = "AddressWire")]
struct AddressDto {
    rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// `lines`: the first and last line, counted from 1. Any other value (a list of another length, one number) is
/// refused with words that say the form, since serde's own ("invalid length 1, expected a tuple of size 2",
/// "expected a sequence") name neither the field nor the form.
fn two_lines<'de, D>(deserializer: D) -> Result<[usize; 2], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let two = value.as_array().and_then(|all| match all.as_slice() {
        [first, last] => Some([
            usize::try_from(first.as_u64()?).ok()?,
            usize::try_from(last.as_u64()?).ok()?,
        ]),
        _ => None,
    });
    two.ok_or_else(|| {
        let given = match value.as_array() {
            Some(all) if all.len() != 2 => format!("a list of {}", all.len()),
            _ => format!("`{value}`"),
        };
        serde::de::Error::custom(format!(
            "`lines` is [first, last], two line numbers counted from 1 ([n, n] for one line), not {given}"
        ))
    })
}

/// `lines` that may be left out, read as [`two_lines`] reads them.
fn optional_lines<'de, D>(deserializer: D) -> Result<Option<[usize; 2]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Lines(#[serde(deserialize_with = "two_lines")] [usize; 2]);
    Ok(Option::<Lines>::deserialize(deserializer)?.map(|Lines(lines)| lines))
}

/// `lines` that may be left out, as the first and last line.
fn maybe_two_lines<'de, D>(deserializer: D) -> Result<Option<(usize, usize)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(optional_lines(deserializer)?.map(|[first, last]| (first, last)))
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

impl TryFrom<AddressDto> for FindingInput {
    type Error = AppError;

    fn try_from(a: AddressDto) -> Result<FindingInput, AppError> {
        Ok(FindingInput {
            rule: a.rule,
            subject: a.subject.map(SubjectInput::try_from).transpose()?,
        })
    }
}

/// A finding in a result: the diagnostic it is (`code`, `level`, `message`, `subjects`, `values`, `help`, ...), and
/// the fields a finding has besides (`rule` is its `code`, kept; `state` says whether a change request deals with it).
#[derive(Debug, Serialize)]
struct FindingOut<'a> {
    #[serde(flatten)]
    diagnostic: DiagnosticOut<'a>,
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
            diagnostic: DiagnosticOut::from(f),
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

/// A target (or keep) a change request recorded, in the result of `cr_open`.
#[derive(Debug, Serialize)]
struct OpenedExpectOut {
    /// `target` or `keep`.
    kind: &'static str,
    target: TargetRef,
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
struct CrOpenOut<'a> {
    cr: String,
    expects: Vec<OpenedExpectOut>,
    findings: Vec<FindingOut<'a>>,
}

/// The result of `cr_crux_raise`: the id of the Crux, and of each option with its choice.
#[derive(Debug, Serialize)]
struct CruxRaisedOut<'a> {
    crux: String,
    options: Vec<RaisedOptionOut<'a>>,
    findings: Vec<FindingOut<'a>>,
}

/// An option of a Crux just raised.
#[derive(Debug, Serialize)]
struct RaisedOptionOut<'a> {
    id: String,
    choice: &'a str,
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

/// What was decided about a Crux, in the result of `status`: the words `view` uses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum DecisionOut<'a> {
    Chosen {
        option: &'a str,
        reason: Option<&'a str>,
    },
    Worded {
        wording: &'a str,
        reason: Option<&'a str>,
    },
    Unspecified(&'a [String]),
    Withdrawn,
}

impl<'a> From<&'a DecisionStatus> for DecisionOut<'a> {
    fn from(d: &'a DecisionStatus) -> DecisionOut<'a> {
        match d {
            DecisionStatus::Chosen { option, reason } => DecisionOut::Chosen {
                option,
                reason: reason.as_deref(),
            },
            DecisionStatus::Worded { wording, reason } => DecisionOut::Worded {
                wording,
                reason: reason.as_deref(),
            },
            DecisionStatus::Unspecified { keep } => DecisionOut::Unspecified(keep),
            DecisionStatus::Withdrawn => DecisionOut::Withdrawn,
        }
    }
}

/// Where what an answer added outside its change request went, in the result of `status`: the titles of the change
/// requests opened for it, or why it is not done.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum BeyondOut<'a> {
    Requests(&'a [String]),
    Declined(&'a str),
}

impl<'a> From<&'a BeyondStatus> for BeyondOut<'a> {
    fn from(b: &'a BeyondStatus) -> BeyondOut<'a> {
        match b {
            BeyondStatus::Requests(titles) => BeyondOut::Requests(titles),
            BeyondStatus::Declined(reason) => BeyondOut::Declined(reason),
        }
    }
}

/// The answer to a Crux, in the result of `status`: the decision, where what it added outside its change request
/// went (left out when it says nothing), what it rests on, who gave it, and when (milliseconds since the epoch).
#[derive(Debug, Serialize)]
struct AnswerOut<'a> {
    id: &'a str,
    decision: DecisionOut<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    beyond: Option<BeyondOut<'a>>,
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

/// One step of the history of a change request, in the result of `status`: `step` is `resolved`, `reopened` or
/// `corrected`. A resolved step has the `resolution` (and the `of` it points at, when it points at one), a reopened
/// step has neither, and a corrected step has the `fields` it replaced a text in (by their words) and no text.
#[derive(Debug, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
enum HistoryOut<'a> {
    Resolved {
        resolution: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        of: Option<&'a str>,
        reason: &'a str,
        by: MakerOut<'a>,
        at: u64,
    },
    Reopened {
        reason: &'a str,
        by: MakerOut<'a>,
        at: u64,
    },
    Corrected {
        fields: &'a [String],
        reason: &'a str,
        by: MakerOut<'a>,
        at: u64,
    },
}

impl<'a> From<&'a TransitionStatus> for HistoryOut<'a> {
    fn from(t: &'a TransitionStatus) -> HistoryOut<'a> {
        match t {
            TransitionStatus::Resolved(r) => HistoryOut::Resolved {
                resolution: r.kind,
                of: r.reference.as_deref(),
                reason: &r.reason,
                by: MakerOut {
                    kind: r.by_kind,
                    name: &r.by,
                },
                at: r.at,
            },
            TransitionStatus::Reopened(r) => HistoryOut::Reopened {
                reason: &r.reason,
                by: MakerOut {
                    kind: r.by_kind,
                    name: &r.by,
                },
                at: r.at,
            },
            TransitionStatus::Corrected(c) => HistoryOut::Corrected {
                fields: &c.fields,
                reason: &c.reason,
                by: MakerOut {
                    kind: c.by_kind,
                    name: &c.by,
                },
                at: c.at,
            },
        }
    }
}

/// A correction of the texts of a record, in the result of `status`: the fields by their words, why, who and when.
/// The text replaced is in none of it.
#[derive(Debug, Serialize)]
struct CorrectionOut<'a> {
    fields: &'a [String],
    reason: &'a str,
    by: MakerOut<'a>,
    at: u64,
}

impl<'a> From<&'a CorrectionStatus> for CorrectionOut<'a> {
    fn from(c: &'a CorrectionStatus) -> CorrectionOut<'a> {
        CorrectionOut {
            fields: &c.fields,
            reason: &c.reason,
            by: MakerOut {
                kind: c.by_kind,
                name: &c.by,
            },
            at: c.at,
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
    /// The ids of the Cruxes raised in it, in `cruxes`.
    cruxes: &'a [String],
    state: &'static str,
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
    renamed: Vec<RenameOut<'a>>,
}

/// A Clause or an Item a change set gave a new name, in the result of `status`.
#[derive(Debug, Serialize)]
struct RenameOut<'a> {
    target: &'a str,
    from: &'a str,
    to: &'a str,
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

/// An option of a Crux, in the result of `status`.
#[derive(Debug, Serialize)]
struct AwaitingOptionOut<'a> {
    id: &'a str,
    choice: &'a str,
    wording: Option<&'a str>,
    expects: Vec<AwaitingExpectOut<'a>>,
    consequence: Option<&'a str>,
}

impl<'a> From<&'a AwaitingOption> for AwaitingOptionOut<'a> {
    fn from(o: &'a AwaitingOption) -> AwaitingOptionOut<'a> {
        AwaitingOptionOut {
            id: &o.id,
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
        }
    }
}

/// The recommendation of a Crux: the id of the option, why, and how sure (0 to 100).
#[derive(Debug, Serialize)]
struct RecommendedOut<'a> {
    option: &'a str,
    reason: Option<&'a str>,
    confidence: Option<u8>,
}

impl<'a> From<&'a RecommendedStatus> for RecommendedOut<'a> {
    fn from(r: &'a RecommendedStatus) -> RecommendedOut<'a> {
        RecommendedOut {
            option: &r.option,
            reason: r.reason.as_deref(),
            confidence: r.confidence,
        }
    }
}

/// A finding a Crux was raised from, in the result of `status`.
#[derive(Debug, Serialize)]
struct CruxFindingOut<'a> {
    rule: &'a str,
    about: Option<&'a str>,
}

/// A Crux in the result of `status`: the record `view` shows, with the change request it stops by its title.
#[derive(Debug, Serialize)]
struct CruxOut<'a> {
    id: &'a str,
    raised_in: &'a str,
    findings: Vec<CruxFindingOut<'a>>,
    kind: &'static str,
    question: &'a str,
    importance: Option<&'static str>,
    options: Vec<AwaitingOptionOut<'a>>,
    recommended: Option<RecommendedOut<'a>>,
    supersedes: Option<&'a str>,
    state: &'static str,
    answer: Option<AnswerOut<'a>>,
    /// The corrections made of its texts, in the order they were made; left out when there is none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    corrections: Vec<CorrectionOut<'a>>,
}

impl<'a> From<&'a CruxStatus> for CruxOut<'a> {
    fn from(q: &'a CruxStatus) -> CruxOut<'a> {
        CruxOut {
            id: &q.id,
            raised_in: &q.raised_in,
            findings: q
                .findings
                .iter()
                .map(|f| CruxFindingOut {
                    rule: &f.rule,
                    about: f.about.as_deref(),
                })
                .collect(),
            kind: q.kind,
            question: &q.question,
            importance: q.importance,
            options: q.options.iter().map(AwaitingOptionOut::from).collect(),
            recommended: q.recommended.as_ref().map(RecommendedOut::from),
            supersedes: q.supersedes.as_deref(),
            state: q.state,
            answer: q.answer.as_ref().map(|a| AnswerOut {
                id: &a.id,
                decision: DecisionOut::from(&a.decision),
                beyond: a.beyond.as_ref().map(BeyondOut::from),
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
            corrections: q.corrections.iter().map(CorrectionOut::from).collect(),
        }
    }
}

/// A Crux that has no decision yet, in the result of `status`: what a decision maker is asked.
#[derive(Debug, Serialize)]
struct AwaitingOut<'a> {
    crux: &'a str,
    raised_in: &'a str,
    question: &'a str,
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
    revision: u64,
    findings: Vec<FindingOut<'a>>,
}

/// The result of an operation that gives nothing back but its name and what the checks of its edit said.
#[derive(Debug, Serialize)]
struct DoneOut<'a> {
    findings: Vec<FindingOut<'a>>,
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
    retired: Vec<RetiredOut<'a>>,
    findings: Vec<FindingOut<'a>>,
}

/// The result of `items_rekey`: the Items that were given another key, those whose revision went up, and the keys of
/// those no declaration was paired with.
#[derive(Debug, Serialize)]
struct RekeyOut<'a> {
    rekeyed: Vec<KeyChangedOut<'a>>,
    revised: Vec<ItemRevisedOut<'a>>,
    unmatched: &'a [String],
    /// The revision of the Interface once it is done.
    revision: u64,
    findings: Vec<FindingOut<'a>>,
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

/// What one put of a `change_set` came to: the setting, the Context, or the asset.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum PutOut<'a> {
    Setting {
        put: &'static str,
        context: &'a str,
    },
    Context {
        put: &'static str,
        context: &'a str,
        id: String,
        revision: u64,
    },
    Asset {
        put: &'static str,
        context: &'a str,
        name: &'a str,
        id: String,
        revision: u64,
    },
}

impl<'a> From<&'a PutDone> for PutOut<'a> {
    fn from(d: &'a PutDone) -> PutOut<'a> {
        match d {
            PutDone::Setting { kind, context } => PutOut::Setting {
                put: kind.word(),
                context: context.as_str(),
            },
            PutDone::Context { name, id, revision } => PutOut::Context {
                put: "context",
                context: name.as_str(),
                id: id.to_string(),
                revision: revision.number(),
            },
            PutDone::Asset {
                kind,
                context,
                name,
                id,
                revision,
            } => PutOut::Asset {
                put: kind.word(),
                context: context.as_str(),
                name: name.as_str(),
                id: id.to_string(),
                revision: revision.number(),
            },
        }
    }
}

/// The result of a `change_set`: each put in the order listed, and everything that left the model (what the puts
/// dropped, then what was retired).
#[derive(Debug, Serialize)]
struct ChangeSetOut<'a> {
    puts: Vec<PutOut<'a>>,
    retired: Vec<RetiredOut<'a>>,
    findings: Vec<FindingOut<'a>>,
}

/// The result of a put of one asset (`contract_put`, `clauses_put`, `interface_put`, `harness_put`): the asset's id
/// and the revision it has after the put.
#[derive(Debug, Serialize)]
struct AssetPutOut<'a> {
    id: String,
    revision: u64,
    findings: Vec<FindingOut<'a>>,
}

/// The result of `status`.
#[derive(Debug, Serialize)]
struct StatusOut<'a> {
    retired: Vec<RetiredOut<'a>>,
    pins: Vec<PinOut<'a>>,
    stale_cites: Vec<StaleCiteOut<'a>>,
    change_requests: Vec<CrOut<'a>>,
    keep_moved: Vec<KeepMovedOut<'a>>,
    cruxes: Vec<CruxOut<'a>>,
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
    findings: Vec<FindingOut<'a>>,
    /// How many findings the `context` asked for left out; only when one was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    outside: Option<usize>,
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
    /// `change` for a check that runs at an edit and speaks about what an edit made stale or out of place, `state`
    /// for one that does not and speaks about the state of the model. It is read from the `moments`.
    kind: &'static str,
    moments: Vec<&'static str>,
    params: std::collections::BTreeMap<String, ParamOut>,
}

/// The result of `check_list`: the checks, and a fingerprint of the policy they run under.
#[derive(Debug, Serialize)]
struct CheckListOut {
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
    moments: Vec<&'static str>,
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
            moments: d.moments.iter().map(|m| m.word()).collect(),
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
    store_revision: u64,
    rules: Vec<RuleOut<'a>>,
    model: &'a ModelView,
}

/// The result of `view`: the part of the model asked for (read at `store_revision`).
#[derive(Debug, Serialize)]
struct ViewOut<'a> {
    store_revision: u64,
    view: &'a ModelSlice,
}

/// The result of `finding_report`.
#[derive(Debug, Serialize)]
struct ReportOut<'a> {
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
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

/// The fields of `cr_crux_raise`, boxed: they are far larger than most operations'. There is no `default` for the
/// decision maker: what depends on the answer waits for it.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RaiseDto {
    /// The title of the change request the Crux is raised in, which it stops.
    #[serde(rename = "in")]
    cr: String,
    /// The question, in a sentence.
    question: String,
    kind: KindDto,
    /// The findings it is raised from, written like the `addresses` of `cr_open`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    findings: Vec<AddressDto>,
    options: Vec<OptionDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    importance: Option<ImportanceDto>,
    /// The id of the Crux whose decision this one replaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supersedes: Option<String>,
}

impl TryFrom<RaiseDto> for RaiseCruxInput {
    type Error = AppError;

    fn try_from(d: RaiseDto) -> Result<RaiseCruxInput, AppError> {
        Ok(RaiseCruxInput {
            cr: d.cr,
            kind: d.kind.into(),
            question: d.question,
            findings: d
                .findings
                .into_iter()
                .map(FindingInput::try_from)
                .collect::<Result<_, _>>()?,
            options: d
                .options
                .into_iter()
                .map(|o| {
                    Ok(OptionInput {
                        choice: o.choice,
                        change: o.wording,
                        targets: targets_of(o.targets)?,
                        keep: targets_of(o.keep)?,
                        consequence: o.consequence,
                        recommended: o.recommended.map(RecommendationInput::from),
                    })
                })
                .collect::<Result<_, AppError>>()?,
            importance: d.importance.map(Importance::from),
            supersedes: d.supersedes,
        })
    }
}

/// The fields of `cr_crux_decide`: the Crux, exactly one decision, what it rests on and who decides.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DecideDto {
    /// The id of the Crux.
    crux: String,
    /// The id of the option chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    option: Option<String>,
    /// The behaviour in the decision maker's own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wording: Option<String>,
    /// What must still hold, for a behaviour left free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unspecified: Option<Vec<String>>,
    /// `true`: the question is not needed any more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    withdrawn: Option<bool>,
    /// Why the option or the wording was decided; a decision to leave it unspecified or to withdraw the question
    /// has its reason in the `basis`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// Where what the option chosen adds outside the change request goes: `{"requests":["<title>"]}`, the change
    /// requests opened for it, or `{"declined":"<why>"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    beyond: Option<BeyondDto>,
    basis: BasisDto,
    by: MakerDto,
}

/// Where what an answer adds outside its change request goes: the titles of the change requests opened for it, or
/// why it is not done.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BeyondDto {
    Requests(Vec<String>),
    Declined(String),
}

impl From<BeyondDto> for BeyondInput {
    fn from(b: BeyondDto) -> BeyondInput {
        match b {
            BeyondDto::Requests(titles) => BeyondInput::Requests(titles),
            BeyondDto::Declined(reason) => BeyondInput::Declined(reason),
        }
    }
}

impl DecideDto {
    /// The decision, parsed once from the fields it is written in: exactly one of them is given.
    fn decision(&self) -> Result<DecisionInput, AppError> {
        let given = usize::from(self.option.is_some())
            + usize::from(self.wording.is_some())
            + usize::from(self.unspecified.is_some())
            + usize::from(self.withdrawn.is_some());
        if given != 1 {
            return Err(AppError::Input(
                "give exactly one of `option`, `wording`, `unspecified` and `withdrawn`"
                    .to_string(),
            ));
        }
        if let Some(option) = &self.option {
            return Ok(DecisionInput::Chosen {
                option: option.clone(),
                reason: self.reason.clone(),
            });
        }
        if let Some(wording) = &self.wording {
            return Ok(DecisionInput::Worded {
                wording: wording.clone(),
                reason: self.reason.clone(),
            });
        }
        if self.reason.is_some() {
            return Err(AppError::Input(
                "`reason` goes with `option` or `wording`; a decision to leave it unspecified or to withdraw the question gives its reason in the `basis`".to_string(),
            ));
        }
        match (&self.unspecified, self.withdrawn) {
            (Some(keep), _) => Ok(DecisionInput::Unspecified { keep: keep.clone() }),
            (None, Some(true)) => Ok(DecisionInput::Withdrawn),
            (None, _) => Err(AppError::Input(
                "`withdrawn` is `true`, or left out".to_string(),
            )),
        }
    }
}

/// Why an option is recommended, and how sure (0 to 100).
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecommendationDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confidence: Option<u64>,
}

impl From<RecommendationDto> for RecommendationInput {
    fn from(r: RecommendationDto) -> RecommendationInput {
        RecommendationInput {
            reason: r.reason,
            confidence: r.confidence,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OptionDto {
    choice: String,
    /// The exact new wording the option makes of the definitions; none: it words none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wording: Option<String>,
    /// The targets it asks to be revised or retired when it is chosen, besides those of the change request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    targets: Vec<RefDto>,
    /// The targets it asks to stay as they are when it is chosen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keep: Vec<RefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consequence: Option<String>,
    /// Present: the Crux recommends this option (at most one option does), and why, and how sure (0 to 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recommended: Option<RecommendationDto>,
}

/// A reference to a target of any kind. The Project is `{"project": true}`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContractRefDto {
    context: String,
    contract: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InterfaceRefDto {
    context: String,
    interface: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HarnessRefDto {
    context: String,
    harness: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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
            RefDto::Context(name) => TargetInput::Context(ContextName::try_from(name)?),
            RefDto::Contract(c) => TargetInput::Contract {
                context: ContextName::try_from(c.context)?,
                contract: AssetName::try_from(c.contract)?,
            },
            RefDto::Interface(i) => TargetInput::Interface {
                context: ContextName::try_from(i.context)?,
                interface: AssetName::try_from(i.interface)?,
            },
            RefDto::Harness(h) => TargetInput::Harness {
                context: ContextName::try_from(h.context)?,
                harness: AssetName::try_from(h.harness)?,
            },
            RefDto::Item(i) => TargetInput::Item(ItemRef::try_from(i)?),
            RefDto::Clause(c) => TargetInput::Clause(ClauseRef::try_from(c)?),
            RefDto::Case(c) => TargetInput::Case {
                context: ContextName::try_from(c.context)?,
                harness: AssetName::try_from(c.harness)?,
                name: c.name,
            },
        })
    }
}

/// The targets a list of references names.
fn targets_of(refs: Vec<RefDto>) -> Result<Vec<TargetInput>, AppError> {
    refs.into_iter().map(TargetInput::try_from).collect()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ClauseDto {
    label: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replaces: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameDto {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ItemDto {
    key: String,
    #[serde(deserialize_with = "two_lines")]
    lines: [usize; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replaces: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ItemRefDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    interface: String,
    key: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClauseRefDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    contract: String,
    label: String,
}

/// A Case of a Harness. A Case of code is on `lines` (required for it) and, for a test, cites `items`; a Case of a
/// Harness of examples has no `lines` or `items`, and holds an `example` unless the Harness takes them from
/// `examples_dir`, in which case its name is the name of the example. Which fields belong to which kind is named when
/// a put breaks it.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct CaseDto {
    name: String,
    #[serde(
        default,
        deserialize_with = "optional_lines",
        skip_serializing_if = "Option::is_none"
    )]
    lines: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    items: Vec<ItemRefDto>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    clauses: Vec<ClauseRefDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    example: Option<ExampleDto>,
}

/// An example, in the words of its files: `args` (the arguments, split on white space), `stdin`, `expected` (the
/// standard output, then a last line `#exit <code>`) and, to have the standard error compared, `stderr`.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExampleDto {
    args: String,
    stdin: String,
    expected: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
}

impl TryFrom<ItemRefDto> for ItemRef {
    type Error = AppError;

    fn try_from(d: ItemRefDto) -> Result<ItemRef, AppError> {
        Ok(ItemRef {
            context: d.context.map(ContextName::try_from).transpose()?,
            interface: AssetName::try_from(d.interface)?,
            key: d.key,
        })
    }
}

impl TryFrom<ClauseRefDto> for ClauseRef {
    type Error = AppError;

    fn try_from(d: ClauseRefDto) -> Result<ClauseRef, AppError> {
        Ok(ClauseRef {
            context: d.context.map(ContextName::try_from).transpose()?,
            contract: AssetName::try_from(d.contract)?,
            label: d.label,
        })
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
                // serde matched raw["op"] against the variant's tag when reading Op,
                // so raw["op"] is that variant's tag, derived once and never hand-written in replies.
                if let Some(op) = raw.as_ref().and_then(|r| r.get("op")) {
                    fields.insert("op".to_string(), op.clone());
                }
                Value::Object(fields)
            }
            Ok(other) => {
                let mut obj = serde_json::Map::new();
                obj.insert("ok".to_string(), json!(true));
                if let Some(op) = raw.as_ref().and_then(|r| r.get("op")) {
                    obj.insert("op".to_string(), op.clone());
                }
                obj.insert("result".to_string(), other);
                Value::Object(obj)
            }
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
    let diag_outs: Vec<DiagnosticOut<'_>> = diagnostics.iter().map(DiagnosticOut::from).collect();
    let mut out = json!({ "ok": false, "diagnostics": diag_outs });
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

/// The result of `rules`: every rule chidev knows.
#[derive(Debug, Serialize)]
struct RulesOut {
    rules: Vec<RuleCard>,
}

/// The result of `check_run`.
#[derive(Debug, Serialize)]
struct CheckRunOut<'a> {
    findings: Vec<FindingOut<'a>>,
    external: &'a [chidev_core::app::ports::external::ExternalReport],
}

/// The result of `init`.
#[derive(Debug, Serialize)]
struct InitOut {
    project: String,
}

/// The result of `context_add`.
#[derive(Debug, Serialize)]
struct ContextAddOut<'a> {
    context: String,
    findings: Vec<FindingOut<'a>>,
}

/// The result of `pin`.
#[derive(Debug, Serialize)]
struct PinResultOut<'a> {
    pin: String,
    findings: Vec<FindingOut<'a>>,
}

/// The result of a retire operation.
fn retire_out(done: &Checked<Retirement>) -> Result<Value, AppError> {
    to_value(&RetireOut {
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
        Op::Rules => to_value(&RulesOut { rules: app.rules() })?,
        Op::Explain { rule } => to_value(&app.explain(&rule)?)?,
        Op::CheckRun => {
            let result = app.check_run().await?;
            to_value(&CheckRunOut {
                findings: findings_out(&result.findings),
                external: &result.external,
            })?
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
        Op::Init => to_value(&InitOut {
            project: app.init().await?.to_string(),
        })?,
        Op::ContextAdd { name, uses } => {
            let done = app.add_context(&name, &uses).await?;
            to_value(&ContextAddOut {
                context: done.value.to_string(),
                findings: findings_out(&done.findings),
            })?
        }
        Op::ContextUses { context, uses } => {
            let done = app.set_uses(&context, &uses).await?;
            to_value(&DoneOut {
                findings: findings_out(&done.findings),
            })?
        }
        Op::BaseOnContract { context } => {
            let done = app.base_on_contract(&context).await?;
            to_value(&DoneOut {
                findings: findings_out(&done.findings),
            })?
        }
        Op::ChangeSet {
            for_change_request,
            puts,
            retires,
        } => {
            let set = ChangeSetInput {
                change_request: for_change_request,
                puts: put_inputs(root, puts).await?,
                retires: retires
                    .into_iter()
                    .map(RetireInput::try_from)
                    .collect::<Result<_, _>>()?,
            };
            let done = app.apply_change_set(&set).await?;
            to_value(&ChangeSetOut {
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
            to_value(&AssetPutOut {
                id: done.value.id.to_string(),
                revision: done.value.revision.number(),
                findings: findings_out(&done.findings),
            })?
        }
        Op::ClausesPut {
            context,
            contract,
            clauses,
            remove,
        } => {
            let clauses = clause_inputs(clauses);
            let done = app
                .put_clauses(&context, &contract, &clauses, &remove)
                .await?;
            to_value(&AssetPutOut {
                id: done.value.id.to_string(),
                revision: done.value.revision.number(),
                findings: findings_out(&done.findings),
            })?
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
            to_value(&AssetPutOut {
                id: done.value.id.to_string(),
                revision: done.value.revision.number(),
                findings: findings_out(&done.findings),
            })?
        }
        Op::HarnessPut {
            context,
            name,
            kind,
            builds,
            source,
            source_file,
            examples_dir,
            cases,
        } => {
            let label = format!("harness `{name}`");
            let content = harness_content(
                root,
                &label,
                HarnessText {
                    kind,
                    source,
                    source_file,
                    examples_dir,
                },
                cases,
            )
            .await?;
            let done = match content {
                HarnessPutContent::Code {
                    kind,
                    source,
                    cases,
                } => {
                    app.put_harness_of(
                        &context,
                        &name,
                        kind.map(HarnessKind::from),
                        &builds,
                        &source,
                        &cases,
                    )
                    .await?
                }
                HarnessPutContent::Examples(examples) => {
                    app.put_examples_harness(&context, &name, &builds, &examples)
                        .await?
                }
            };
            to_value(&AssetPutOut {
                id: done.value.id.to_string(),
                revision: done.value.revision.number(),
                findings: findings_out(&done.findings),
            })?
        }
        Op::ContextRetire { context } => {
            let done = app.retire_context(&context).await?;
            retire_out(&done)?
        }
        Op::ContractRetire { context, contract } => {
            let done = app.retire_contract(&context, &contract).await?;
            retire_out(&done)?
        }
        Op::ItemsRekey { context, interface } => {
            let done = app.items_rekey(&context, &interface).await?;
            to_value(&RekeyOut {
                rekeyed: done
                    .value
                    .rekeyed
                    .iter()
                    .map(|k| KeyChangedOut {
                        from: &k.from,
                        to: &k.to,
                    })
                    .collect(),
                revised: done
                    .value
                    .revised
                    .iter()
                    .map(|r| ItemRevisedOut {
                        key: &r.key,
                        from: r.from,
                        to: r.to,
                    })
                    .collect(),
                unmatched: &done.value.unmatched,
                revision: done.value.revision,
                findings: findings_out(&done.findings),
            })?
        }
        Op::InterfaceRetire { context, interface } => {
            let done = app.retire_interface(&context, &interface).await?;
            retire_out(&done)?
        }
        Op::HarnessRetire { context, harness } => {
            let done = app.retire_harness(&context, &harness).await?;
            retire_out(&done)?
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
            to_value(&PinResultOut {
                pin: done.value.to_string(),
                findings: findings_out(&done.findings),
            })?
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
                assumptions,
                addresses,
            } = *open;
            let input = ChangeRequestInput {
                title,
                situation,
                intent: intent.into(),
                parent,
                issuer,
                targets: targets_of(targets)?,
                keep: targets_of(keep)?,
                assumptions: assumptions.into_iter().map(AssumptionInput::from).collect(),
                addresses: addresses
                    .into_iter()
                    .map(FindingInput::try_from)
                    .collect::<Result<_, _>>()?,
            };
            let done = app.open_change_request(&input).await?;
            to_value(&CrOpenOut {
                cr: done.value.id.to_string(),
                expects: done
                    .value
                    .expects
                    .into_iter()
                    .map(OpenedExpectOut::from)
                    .collect(),
                findings: findings_out(&done.findings),
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
            let done = app.assume(&title, &assumption).await?;
            to_value(&DoneOut {
                findings: findings_out(&done.findings),
            })?
        }
        Op::CrCruxRaise(raise) => {
            let input = RaiseCruxInput::try_from(*raise)?;
            let done = app.raise_crux(&input).await?;
            let Raised { id, options } = &done.value;
            to_value(&CruxRaisedOut {
                crux: id.to_string(),
                options: options
                    .iter()
                    .map(|o| RaisedOptionOut {
                        id: o.id.to_string(),
                        choice: &o.choice,
                    })
                    .collect(),
                findings: findings_out(&done.findings),
            })?
        }
        Op::CrCruxDecide(decide) => {
            let decision = decide.decision()?;
            let DecideDto {
                crux,
                beyond,
                basis,
                by,
                ..
            } = *decide;
            let beyond = beyond.map(BeyondInput::from);
            let done = app
                .decide_crux(&crux, &decision, beyond.as_ref(), &basis.into(), &by.into())
                .await?;
            to_value(&DoneOut {
                findings: findings_out(&done.findings),
            })?
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
            to_value(&DoneOut {
                findings: findings_out(&done.findings),
            })?
        }
        Op::CrReopen { title, reason, by } => {
            let done = app
                .reopen_change_request(&title, &reason, &by.into())
                .await?;
            to_value(&DoneOut {
                findings: findings_out(&done.findings),
            })?
        }
        Op::Status => {
            let s = app.status().await?;
            to_value(&StatusOut {
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
                        cruxes: &c.cruxes,
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
                                renamed: w
                                    .renamed
                                    .iter()
                                    .map(|r| RenameOut {
                                        target: &r.target,
                                        from: &r.from,
                                        to: &r.to,
                                    })
                                    .collect(),
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
                cruxes: s.cruxes.iter().map(CruxOut::from).collect(),
                awaiting: s
                    .awaiting
                    .iter()
                    .map(|a| AwaitingOut {
                        crux: &a.crux,
                        raised_in: &a.raised_in,
                        question: &a.question,
                        kind: a.kind,
                        options: a.options.iter().map(AwaitingOptionOut::from).collect(),
                        recommended: a.recommended.as_ref().map(RecommendedOut::from),
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
                        state: m.state.word(),
                        moved: match &m.state {
                            MeasurementState::Stale(parts) => {
                                parts.iter().map(MovedOut::from).collect()
                            }
                            MeasurementState::Current
                            | MeasurementState::Retired
                            | MeasurementState::Unpinned => Vec::new(),
                        },
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
                        corrections: m.corrections.iter().map(CorrectionOut::from).collect(),
                    })
                    .collect(),
            })?
        }
        Op::Check { context } => {
            let checked = app.check(context.as_deref()).await?;
            to_value(&CheckOut {
                findings: findings_out(&checked.findings),
                outside: context.as_ref().map(|_| checked.outside),
            })?
        }
        Op::CheckRules { moment, hook, rule } => {
            if hook.is_some() {
                return Err(AppError::Input(renamed_key("hook", "moment")));
            }
            let moment = moment
                .as_deref()
                .map(|w| {
                    Moment::parse(w).map_err(|e| AppError::Input(format!("the moment `{w}`: {e}")))
                })
                .transpose()?;
            let view = app.check_rules(moment, rule.as_deref()).await?;
            to_value(&CheckRulesOut {
                store_revision: view.store_revision.0,
                rules: view.rules.iter().map(RuleOut::of).collect(),
                model: &view.model,
            })?
        }
        Op::View(ViewDto { sections, contexts }) => {
            let read = app
                .view(&ViewInput {
                    contexts,
                    sections: sections.into_iter().map(ModelSection::from).collect(),
                })
                .await?;
            to_value(&ViewOut {
                store_revision: read.store_revision.0,
                view: &read.view,
            })?
        }
        Op::TextFind { text } => {
            let places = app.find_text(&text.0).await?;
            to_value(&PlacesOut {
                places: places.iter().map(PlaceDto::from).collect(),
            })?
        }
        Op::RecordCorrect {
            text,
            reason,
            by,
            places,
        } => {
            let places = places
                .into_iter()
                .map(Place::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            let answer = app
                .correct_records(&CorrectInput {
                    text: text.0,
                    reason,
                    by: by.into(),
                    places,
                })
                .await?;
            to_value(&CorrectOut {
                corrected: answer.corrected.iter().map(PlaceDto::from).collect(),
                remaining: match &answer.remaining {
                    Remaining::Found(places) => {
                        RemainingOut::Found(places.iter().map(PlaceDto::from).collect())
                    }
                    Remaining::Unknown { reason } => RemainingOut::Unknown { unknown: reason },
                },
                cleared: match &answer.cleared {
                    Cleared::Done => ClearedOut::Done,
                    Cleared::NotFinished { reason } => ClearedOut::NotFinished(reason),
                },
            })?
        }
        Op::FindingReport(report) => {
            let done = app.finding_report(&ReportInput::try_from(*report)?).await?;
            to_value(&ReportOut {
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
                    kind: c.descriptor.kind().word(),
                    moments: c.descriptor.moments.iter().map(|m| m.word()).collect(),
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
                policy: registry.policy().hash().to_string(),
                checks,
            })?
        }
    })
}

#[cfg(test)]
mod tests {
    use chidev_core::app::RecordKind;

    use super::*;

    /// The kind of record a place of the wire form is in. It matches without a wildcard, so a variant added to
    /// [`PlaceDto`] does not compile until it is given its kind here.
    fn kind_of(place: &PlaceDto) -> RecordKind {
        match place {
            PlaceDto::ChangeRequest { .. } => RecordKind::ChangeRequest,
            PlaceDto::Crux { .. } => RecordKind::Crux,
            PlaceDto::Measurement { .. } => RecordKind::Measurement,
            PlaceDto::Invocation { .. } => RecordKind::Invocation,
        }
    }

    #[test]
    fn the_tag_of_each_place_on_the_wire_is_the_word_of_its_kind_of_record() {
        let id = Uuid::nil().to_string();
        let places = [
            PlaceDto::ChangeRequest {
                id: id.clone(),
                field: CrFieldDto::Title,
            },
            PlaceDto::Crux {
                id: id.clone(),
                field: CruxFieldDto::Question,
            },
            PlaceDto::Measurement {
                id,
                field: MeasurementFieldDto::Failure,
            },
            PlaceDto::Invocation {
                seq: 1,
                field: InvocationFieldDto::Stderr,
            },
        ];
        for place in places {
            let kind = kind_of(&place);
            let written = serde_json::to_value(&place).unwrap();
            let tags: Vec<&String> = written.as_object().unwrap().keys().collect();
            assert_eq!(tags, [kind.word()], "{written}");
            // and the place it is read as is in the same kind of record
            let read = Place::try_from(place).unwrap();
            assert_eq!(read.kind(), kind);
        }
    }
}
