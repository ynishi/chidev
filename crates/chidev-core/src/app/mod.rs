//! The use cases: read what they need, decide with [`crate::domain`], record
//! the outcome.
//!
//! What a use case needs from the outside world is a port: a trait declared
//! here, by its consumer, and implemented in [`crate::infra`]. Code here calls
//! ports, never the file system, a process or the environment directly.
//!
//! The use cases speak in the names a person writes (a Context's name, an
//! asset's name, a clause label, a signature, a case name) and turn them into
//! the ids the domain works with: a definition given again under the same
//! label, signature or case name keeps its id, and a new one gets a fresh id.

mod check;
mod external;
pub use self::external::{ExternalChecked, ExternalStanding, ExternalState};
use self::ports::external::{ExternalExecutor, ExternalReport, NoExternal};
mod judged;
mod measure;
pub use self::measure::{
    CaseLine, EvidenceLine, FindingLine, MeasureDone, MeasurementStanding, MutantLine, RerunLine,
    SimSummary, UnreachedLine,
};
use self::ports::measure::{NoRunner, Runner, RunnerError};
mod model_view;
pub mod subject;
pub use self::subject::SubjectInput;
mod target;
pub use self::target::{ExpectInput, OpenedExpect, TargetInput};
pub mod ports;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use thiserror::Error;

pub use self::check::{CheckInfo, Checked, Found, Judgment, Registry, RegistryError, Run};
pub use self::judged::{
    JudgedState, ReportInput, Reported, ReportedFinding, RuleStanding, RuleToJudge, RulesView,
};
pub use self::model_view::{ModelView, VIEW_VERSION};
use self::ports::{
    AssetKind, AssetName, Changes, Clock, IdSource, Invocation, PinCode, Store, StoreRevision,
};
pub use crate::domain::check::ParamValue;
use crate::domain::check::{CheckView, Finding, Hook, Mention};
use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestDraft, ChangeRequestError, Crux,
};
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::{Project, ProjectError};
use crate::domain::repository::StoreError;
use crate::domain::service::change_request::{
    awaiting_decision, check_supersedes, decide, expectations_now, keep_moved, resolve, unapplied,
};
use crate::domain::service::cite::stale_cites;
use crate::domain::service::pin::{PinError, StaleReason, Staleness, record, staleness};
use crate::domain::vo::answer::{Answer, Basis, DecisionMaker};
use crate::domain::vo::assumption::{Assumption, Evidence, KnowledgeRef, RepoPath};
use crate::domain::vo::case::{CaseCites, CaseName};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::confidence::Confidence;
pub use crate::domain::vo::crux::{CruxKind, Importance};
use crate::domain::vo::crux::{CruxOption, Decision, Reasoned, Recommended};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
pub use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{
    CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
    PinId, ProjectId,
};
pub use crate::domain::vo::intent::Intent;
use crate::domain::vo::intent::Issuer;
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::measure::MeasureError;
use crate::domain::vo::name::ContextName;
use crate::domain::vo::policy::PolicyLineId;
use crate::domain::vo::prose::Prose;
use crate::domain::vo::retired::Retired;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::signature::Signature;
use crate::domain::vo::source::Source;
pub use crate::domain::vo::target::ExpectKind;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// Why a use case did nothing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppError {
    /// There is no Project yet.
    #[error("there is no project yet; init one first")]
    NoProject,
    /// There is a Project already.
    #[error("there is a project already")]
    AlreadyInitialized,
    /// No Context has this name.
    #[error("no context is named `{0}`")]
    UnknownContext(String),
    /// No asset of this kind has this name in the Context.
    #[error("the context `{context}` has no {kind} named `{name}`")]
    UnknownAsset {
        /// The Context.
        context: String,
        /// The kind of asset.
        kind: &'static str,
        /// The name.
        name: String,
    },
    /// The asset was retired: it is out of the model, and its name and id stay reserved.
    #[error(
        "the {kind} `{name}` of the context `{context}` was retired; its name and id are reserved, use another name"
    )]
    AssetRetired {
        /// The Context.
        context: String,
        /// The kind of asset.
        kind: &'static str,
        /// The name.
        name: String,
    },
    /// Cases that stay cite what the retirement would take away.
    #[error(
        "cannot retire: {} cite(s) {}; retire the cases' harness first, or put it again without the citation",
        .cases.join(", "),
        .cited.join(", ")
    )]
    RetireCited {
        /// What would go and is cited, described.
        cited: Vec<String>,
        /// The Cases that stay and cite it, described.
        cases: Vec<String>,
    },
    /// Contexts that stay use the Context to retire.
    #[error(
        "cannot retire the context `{context}`: the context(s) {} use it; change their uses or retire them first",
        .by.join(", ")
    )]
    RetireUsed {
        /// The Context to retire.
        context: String,
        /// The Contexts that use it.
        by: Vec<String>,
    },
    /// The Interface declares no Item with this signature.
    #[error("the interface `{interface}` declares no `{signature}`")]
    UnknownItem {
        /// The Interface.
        interface: String,
        /// The signature looked for.
        signature: String,
    },
    /// The Contract has no Clause with this label.
    #[error("the contract `{contract}` has no clause {label}")]
    UnknownClause {
        /// The Contract.
        contract: String,
        /// The label looked for.
        label: String,
    },
    /// No change request has this title.
    #[error("no change request is titled `{0}`")]
    UnknownChangeRequest(String),
    /// A change request has this title already; a change request is addressed by its title.
    #[error("a change request is titled `{0}` already")]
    TitleTaken(String),
    /// A citation outside a Harness does not say which Context it is in.
    #[error("a citation outside a harness names its context")]
    ContextNotGiven,
    /// A value given is not well formed (a name, a label, a line range, ...).
    #[error("{0}")]
    Input(String),
    /// The change breaks a rule of the Project.
    #[error(transparent)]
    Project(#[from] ProjectError),
    /// The code cannot be pinned.
    #[error(transparent)]
    Pin(#[from] PinError),
    /// The change request cannot be opened, decided or resolved.
    #[error(transparent)]
    ChangeRequest(#[from] ChangeRequestError),
    /// The targets have not followed the change request yet; they are named as `status` names them.
    #[error("the change is not applied yet: {}", not_applied_words(.unchanged, .gone, .not_removed))]
    NotApplied {
        /// The targets to change that are still as they were, described.
        unchanged: Vec<String>,
        /// The targets to change that are gone, described.
        gone: Vec<String>,
        /// The targets to remove that are still there, described.
        not_removed: Vec<String>,
    },
    /// Targets the change request expected to stay as they are have moved, so it cannot be resolved.
    #[error("the change request expected these to stay as they are, but they moved: {}", .moved.join(", "))]
    KeepMoved {
        /// The targets that moved, described.
        moved: Vec<String>,
    },
    /// Change requests that are a part of this one are not resolved yet.
    #[error("these parts of the change request are not resolved: {}", .children.join(", "))]
    ChildrenNotResolved {
        /// The titles of the parts that are not resolved.
        children: Vec<String>,
    },
    /// The Interface has no Pin, so there is no code to measure.
    #[error(
        "the interface `{interface}` of `{context}` is not pinned, so there is no code to measure"
    )]
    NotPinned {
        /// The Context.
        context: String,
        /// The Interface.
        interface: String,
    },
    /// The Pin was made before the code it accepted was kept.
    #[error("the pin of `{interface}` kept no code text; pin the code again")]
    PinCodeNotKept {
        /// The Interface.
        interface: String,
    },
    /// The measure runner cannot measure this.
    #[error(transparent)]
    Runner(#[from] RunnerError),
    /// What a run observed is not a measurement.
    #[error("a measurement is refused: {0}")]
    Measurement(#[from] MeasureError),
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// No check has this id.
    #[error("no check is named `{0}`")]
    UnknownRule(String),
    /// The caller judged a different definition; no result was replaced.
    #[error(
        "the rule `{rule}` changed: expected digest {expected}, reported {reported}; read check_rules again"
    )]
    RuleVersionMismatch {
        /// Rule whose definition differs.
        rule: String,
        /// Current Registry definition digest.
        expected: Digest,
        /// Digest supplied by the caller.
        reported: Digest,
    },
    /// The check is not a judged rule, so an agent has nothing to report for it.
    #[error("the rule `{rule}` is {executor}, not judged: only a judged rule takes a report")]
    NotJudged {
        /// The rule.
        rule: String,
        /// Who runs it.
        executor: &'static str,
    },
    /// The policy turned the rule off.
    #[error("the rule `{0}` is off by the policy")]
    RuleOff(String),
    /// A mandatory check found something at `error`; nothing was kept.
    #[error("refused by the mandatory check(s) {}", rules_of(.0))]
    Refused(Vec<Found>),
    /// An external gate refused the change; attempts are returned but not persisted.
    #[error("refused by external gate(s)")]
    ExternalRefused {
        /// Findings that refused the change.
        findings: Vec<Found>,
        /// All external attempts made while checking the candidate.
        external: Vec<ExternalReport>,
    },
}

/// The rules of `found`, once each, for a message.
fn rules_of(found: &[Found]) -> String {
    let mut rules: Vec<&str> = found.iter().map(|f| f.rule.as_str()).collect();
    rules.sort_unstable();
    rules.dedup();
    rules.join(", ")
}

fn input(e: impl Display) -> AppError {
    AppError::Input(e.to_string())
}

/// No settings: what the hooks of a change give the checks, which read no measurement.
static NO_SETTINGS: BTreeMap<InterfaceId, String> = BTreeMap::new();
static NO_SIM_SETTINGS: BTreeMap<HarnessId, String> = BTreeMap::new();

/// What the checks of a hook read.
fn view<'a>(
    before: Option<&'a Project>,
    after: &'a Project,
    pins: &'a [Pin],
    change_requests: &'a [ChangeRequest],
) -> CheckView<'a> {
    CheckView {
        before,
        after,
        pins,
        change_requests,
        measurements: &[],
        measure_settings: &NO_SETTINGS,
        sim_settings: &NO_SIM_SETTINGS,
    }
}

/// What the checks of a hook gave: the findings for the caller, the records to keep with the change.
struct Gate {
    found: Vec<Found>,
    invocations: Vec<Invocation>,
    external: Vec<ExternalReport>,
}

/// A Clause of a Contract, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseInput {
    /// Its label (`C1`).
    pub label: String,
    /// Its sentence.
    pub text: String,
}

/// An Item of an Interface, as found in the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemInput {
    /// Its declaration.
    pub signature: String,
    /// The first and last line it is on, from 1.
    pub lines: (usize, usize),
    /// The signature it had before, when it changed: the Item keeps its id. Without it, an Item whose signature
    /// matches one the Interface has keeps that id, and any other is new.
    pub replaces: Option<String>,
}

/// A citation of an Item: the Interface (by name) and the signature, in a Context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    /// The Context; within a Harness, its own when not given.
    pub context: Option<String>,
    /// The Interface.
    pub interface: String,
    /// The signature of the Item.
    pub signature: String,
}

/// A citation of a Clause: the Contract (by name) and the label, in a Context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseRef {
    /// The Context; within a Harness, its own when not given.
    pub context: Option<String>,
    /// The Contract.
    pub contract: String,
    /// The label of the Clause.
    pub label: String,
}

/// A Case of a Harness, as found in the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseInput {
    /// Its name.
    pub name: String,
    /// The first and last line it is on, from 1.
    pub lines: (usize, usize),
    /// The Items it checks (one at least).
    pub items: Vec<ItemRef>,
    /// The Clauses it checks.
    pub clauses: Vec<ClauseRef>,
}

/// An asset made or revised: its id and the revision it is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Put<I> {
    /// The asset's id.
    pub id: I,
    /// Its revision after the change.
    pub revision: Revision,
}

/// Where the Pin of one Interface stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinState {
    /// The code of the Interface was never pinned.
    Unpinned,
    /// Nothing the Pin stood on changed.
    Fresh,
    /// The pinned Interface is gone.
    Gone,
    /// These moved, each described.
    Stale(Vec<String>),
}

/// The Pin state of one Interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinStatus {
    /// The Context's name.
    pub context: String,
    /// The Interface's name.
    pub interface: String,
    /// Where its Pin stands.
    pub state: PinState,
}

/// A citation a Case has not been brought up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleCiteStatus {
    /// The Case's name.
    pub case: String,
    /// What it cites, described.
    pub target: String,
    /// The revision it recorded.
    pub cited: u64,
    /// The revision the target has now.
    pub now: u64,
}

/// An option of a Crux, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionInput {
    /// The choice.
    pub choice: String,
    /// The change it makes to the definitions the Crux is about (the exact new wording); none: it changes none.
    pub change: Option<String>,
    /// What it expects of targets when it is chosen, apart from what the change request expects.
    pub expects: Vec<ExpectInput>,
    /// What else it affects.
    pub consequence: Option<String>,
}

/// A choice and why (a decision).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasonedInput {
    /// The choice.
    pub choice: String,
    /// Why.
    pub reason: Option<String>,
}

/// The option recommended, why, and how sure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecommendedInput {
    /// The choice.
    pub choice: String,
    /// Why.
    pub reason: Option<String>,
    /// How sure, from 0 to 100.
    pub confidence: Option<u64>,
}

/// The choice of a change request a decision maker makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CruxInput {
    /// What kind of question it is.
    pub kind: CruxKind,
    /// The options.
    pub options: Vec<OptionInput>,
    /// The option recommended.
    pub recommended: Option<RecommendedInput>,
    /// How much the answer matters.
    pub importance: Option<Importance>,
    /// The title of the change request whose answered Crux this one replaces.
    pub supersedes: Option<String>,
}

/// Where an Assumption stands, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceInput {
    /// A place in the repository: a path relative to it, and the first and last line when they matter.
    Repo {
        /// The file.
        path: String,
        /// The first and last line, from 1.
        lines: Option<(usize, usize)>,
    },
    /// General knowledge, by where it is written.
    Knowledge {
        /// The reference.
        reference: String,
    },
}

/// An interpretation settled from the repository and general knowledge, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssumptionInput {
    /// What is assumed.
    pub statement: String,
    /// Why it holds.
    pub reason: String,
    /// What it stands on.
    pub evidence: Vec<EvidenceInput>,
}

/// What a decision rests on, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasisInput {
    /// A line of the Project's decision Policy (`P1`); the Policy must have it.
    Policy(String),
    /// Nothing derives it; the reason it went this way.
    Arbitrary(String),
}

/// Who decides, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionMakerInput {
    /// A role that holds the authority to decide (`orchestrator`).
    Role(String),
    /// A person.
    Person(String),
}

/// One line of the decision Policy, as a person writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyLineInput {
    /// Its id (`P1`).
    pub id: String,
    /// What it says.
    pub text: String,
}

/// The decision Policy as `policy_put` left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyPut {
    /// The revision of the Policy after the change.
    pub revision: Revision,
}

/// A change request as it is opened. Citations name their Context. A change request is addressed by its title,
/// which is unique among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequestInput {
    /// What is asked, in a line.
    pub title: String,
    /// Why it is needed.
    pub situation: Option<String>,
    /// Whether it means to change the behaviour.
    pub intent: Intent,
    /// The title of the broader change request it is a part of.
    pub parent: Option<String>,
    /// Who raises it.
    pub issuer: Option<String>,
    /// What it expects of targets: change, remove or keep, each target once.
    pub expects: Vec<ExpectInput>,
    /// The choice for a decision maker, besides the expectations; none: nothing to decide.
    pub crux: Option<CruxInput>,
    /// The interpretations settled from the repository and general knowledge while raising it.
    pub assumptions: Vec<AssumptionInput>,
    /// The findings it deals with.
    pub addresses: Vec<FindingInput>,
}

/// A finding a change request addresses: its rule and the shared subject input.
///
/// `subject: None` means the Project as a whole. Names must resolve to an existing subject when the request
/// opens; the domain keeps the resolved id thereafter, even if the subject later disappears. Rule ids are
/// syntax-checked, but need not be registered or have a currently reported finding.
///
/// Rust callers migrating from `item` / `clause` fields use `Some(SubjectInput::Item(r))` /
/// `Some(SubjectInput::Clause(r))`. The CLI separately accepts its legacy wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingInput {
    /// The rule (`cite.uncited-clause`).
    pub rule: String,
    /// What the finding is about; none for the Project.
    pub subject: Option<SubjectInput>,
}

/// What the decision maker decided about a Crux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionInput {
    /// One of the options.
    Chosen(ReasonedInput),
    /// The behaviour in the person's own words.
    Worded {
        /// The behaviour.
        wording: String,
        /// Why.
        reason: Option<String>,
    },
    /// The behaviour left free, as long as these hold.
    Unspecified {
        /// What must still hold.
        keep: Vec<String>,
    },
}

/// What a decision rests on, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasisStatus {
    /// A line of the decision Policy, and the revision of the Policy when it was applied.
    Policy {
        /// The line (`P1`).
        line: String,
        /// The revision of the Policy when the line was applied.
        policy_revision: u64,
    },
    /// Nothing derived it.
    Arbitrary {
        /// Why it went this way.
        reason: String,
    },
}

/// The answer to a Crux, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerStatus {
    /// What was decided, in words.
    pub decision: String,
    /// What it rests on.
    pub basis: BasisStatus,
    /// `role` or `person`.
    pub by_kind: &'static str,
    /// The name of the role or the person.
    pub by: String,
    /// When, in milliseconds since the epoch.
    pub at: u64,
}

/// Where an Assumption stands, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceStatus {
    /// A place in the repository.
    Repo {
        /// The file.
        path: String,
        /// The first and last line, when they were given.
        lines: Option<(usize, usize)>,
    },
    /// General knowledge.
    Knowledge {
        /// Where it is written.
        reference: String,
    },
}

/// An Assumption of a change request, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssumptionStatus {
    /// What is assumed.
    pub statement: String,
    /// Why it holds.
    pub reason: String,
    /// What it stands on.
    pub evidence: Vec<EvidenceStatus>,
}

/// Where one change request stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequestStatus {
    /// Its title.
    pub title: String,
    /// `change_behavior` or `keep_behavior`.
    pub intent: &'static str,
    /// The title of the change request it is a part of.
    pub parent: Option<String>,
    /// Who raised it.
    pub issuer: Option<String>,
    /// The kind of its Crux; none: it has no choice for a decision maker.
    pub crux: Option<&'static str>,
    /// The title of the change request whose answered Crux its Crux supersedes.
    pub supersedes: Option<String>,
    /// `open`, `decided` or `resolved`.
    pub state: &'static str,
    /// The answer to its Crux: the decision, what it rests on, who gave it and when.
    pub answer: Option<AnswerStatus>,
    /// The interpretations it records, in order.
    pub assumptions: Vec<AssumptionStatus>,
}

/// A change request as it was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// Its id.
    pub id: ChangeRequestId,
    /// The expectations it recorded, in the order of the targets, by name.
    pub expects: Vec<OpenedExpect>,
}

/// What an option of an undecided Crux expects of one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingExpect {
    /// `change`, `remove` or `keep`.
    pub kind: &'static str,
    /// The target, described.
    pub target: String,
}

/// An option of an undecided Crux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingOption {
    /// The choice.
    pub choice: String,
    /// The new wording it makes of the definitions.
    pub wording: Option<String>,
    /// What it expects of targets when chosen.
    pub expects: Vec<AwaitingExpect>,
    /// What else it affects.
    pub consequence: Option<String>,
}

/// The recommendation of an undecided Crux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecommendedStatus {
    /// The choice recommended.
    pub choice: String,
    /// Why.
    pub reason: Option<String>,
    /// How sure, from 0 to 100.
    pub confidence: Option<u8>,
}

/// A change request whose Crux has no decision yet: what a decision maker is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingStatus {
    /// The change request's title.
    pub cr: String,
    /// The kind of its Crux.
    pub kind: &'static str,
    /// The options.
    pub options: Vec<AwaitingOption>,
    /// The option recommended, with how sure.
    pub recommended: Option<RecommendedStatus>,
    /// How much the answer matters.
    pub importance: Option<&'static str>,
}

/// One line of the decision Policy, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyLineStatus {
    /// Its id (`P1`).
    pub id: String,
    /// What it says.
    pub text: String,
}

/// The decision Policy of the Project, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyStatus {
    /// Its revision.
    pub revision: u64,
    /// Its lines, in order.
    pub lines: Vec<PolicyLineStatus>,
}

/// An unresolved change request the targets have not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnappliedStatus {
    /// The change request's title.
    pub cr: String,
    /// The decision on its Crux, in words.
    pub decision: Option<String>,
    /// The targets to change that have not changed since, described.
    pub unchanged: Vec<String>,
    /// The targets to change that are gone, described.
    pub gone: Vec<String>,
    /// The targets to remove that are still there, described.
    pub not_removed: Vec<String>,
}

/// An unresolved change request that expected targets to stay as they are, and they moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepMovedStatus {
    /// The change request's title.
    pub cr: String,
    /// The targets that moved (or are gone), described.
    pub moved: Vec<String>,
}

/// A thing the Project retired, in the names a person knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredStatus {
    /// `context`, `contract`, `interface`, `harness`, `clause`, `item` or `case`.
    pub kind: &'static str,
    /// The Context's name: its own for a retired Context, else the one the thing was in.
    pub context: String,
    /// Its name: the Context's, the asset's, or for a Clause, an Item or a Case the asset's name and its label,
    /// signature or name.
    pub name: String,
    /// The revision it had when it was retired.
    pub last_revision: u64,
    /// The revision of what held it once it was retired: the Project's for a Context, the Context's for an asset
    /// (and for the parts that went with it), the asset's for a part a put dropped.
    pub at: u64,
}

/// What a retirement took out of the model: the thing and everything that went with it, in the order recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retirement {
    /// The things retired.
    pub retired: Vec<RetiredStatus>,
}

/// Where the Project stands: what is pinned and whether it holds, the citations behind their targets, the change
/// requests, and those not yet applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Everything the Project retired, in the order it was retired.
    pub retired: Vec<RetiredStatus>,
    /// Every Interface, and every Pin whose Interface is gone.
    pub pins: Vec<PinStatus>,
    /// The citations behind their targets.
    pub stale_cites: Vec<StaleCiteStatus>,
    /// Every change request.
    pub change_requests: Vec<ChangeRequestStatus>,
    /// The change requests not yet applied.
    pub unapplied: Vec<UnappliedStatus>,
    /// The change requests that expected a target to stay as it is, and it moved.
    pub keep_moved: Vec<KeepMovedStatus>,
    /// The change requests whose Crux has no decision yet.
    pub awaiting: Vec<AwaitingStatus>,
    /// The decision Policy of the Project.
    pub policy: PolicyStatus,
    /// What every check finds in the state as it is, each with whether a change request deals with it.
    pub findings: Vec<Found>,
    /// The judged rules never reported or gone stale: what an agent is to judge (`check_rules`).
    pub judged: Vec<RuleStanding>,
    /// The freshness of saved external executions; reading status never starts a process.
    pub external: Vec<ExternalStanding>,
    /// The latest measurement of each Interface that has one: its counts, and whether it still describes the
    /// Pin (`current`) or not (`stale`).
    pub measurements: Vec<MeasurementStanding>,
}

/// A decision, in words.
fn decision_text(d: &Decision) -> String {
    match d {
        Decision::Chosen(c) => c.choice().to_string(),
        Decision::Worded { wording, .. } => format!("in other words: {wording}"),
        Decision::Unspecified { .. } => "left unspecified".to_string(),
    }
}

fn prose(text: &Option<String>) -> Result<Option<Prose>, AppError> {
    text.as_deref()
        .map(|t| Prose::try_from(t).map_err(input))
        .transpose()
}

fn reasoned(r: &ReasonedInput) -> Result<Reasoned, AppError> {
    Ok(Reasoned::new(
        Choice::try_from(r.choice.as_str()).map_err(input)?,
        prose(&r.reason)?,
    ))
}

/// An Assumption as the domain holds it, from what a person wrote.
fn assumption_of(a: &AssumptionInput) -> Result<Assumption, AppError> {
    let mut evidence = Vec::with_capacity(a.evidence.len());
    for e in &a.evidence {
        evidence.push(match e {
            EvidenceInput::Repo { path, lines } => Evidence::Repo {
                path: RepoPath::try_from(path.as_str()).map_err(input)?,
                lines: lines
                    .map(|(first, last)| LineRange::new(first, last).map_err(input))
                    .transpose()?,
            },
            EvidenceInput::Knowledge { reference } => Evidence::Knowledge {
                reference: KnowledgeRef::try_from(reference.as_str()).map_err(input)?,
            },
        });
    }
    Ok(Assumption::new(
        Prose::try_from(a.statement.as_str()).map_err(input)?,
        Prose::try_from(a.reason.as_str()).map_err(input)?,
        evidence,
    ))
}

/// An Assumption, in words, for `status`.
fn assumption_status(a: &Assumption) -> AssumptionStatus {
    AssumptionStatus {
        statement: a.statement().to_string(),
        reason: a.reason().to_string(),
        evidence: a
            .evidence()
            .iter()
            .map(|e| match e {
                Evidence::Repo { path, lines } => EvidenceStatus::Repo {
                    path: path.to_string(),
                    lines: lines.map(|l| (l.first(), l.last())),
                },
                Evidence::Knowledge { reference } => EvidenceStatus::Knowledge {
                    reference: reference.to_string(),
                },
            })
            .collect(),
    }
}

/// An answer, in words, for `status`.
fn answer_status(a: &Answer) -> AnswerStatus {
    AnswerStatus {
        decision: decision_text(a.decision()),
        basis: match a.basis() {
            Basis::Policy {
                line,
                policy_revision,
            } => BasisStatus::Policy {
                line: line.to_string(),
                policy_revision: policy_revision.number(),
            },
            Basis::Arbitrary { reason } => BasisStatus::Arbitrary {
                reason: reason.to_string(),
            },
        },
        by_kind: a.by().word(),
        by: a.by().name().to_string(),
        at: a.at().millis(),
    }
}

/// The use cases over one store, one source of ids and one clock (which dates the decisions).
#[derive(Debug)]
pub struct App<S, G, E = NoExternal> {
    store: S,
    ids: G,
    registry: Registry,
    external: E,
    runner: Box<dyn Runner>,
    clock: Box<dyn Clock>,
}

impl<S: Store, G: IdSource> App<S, G, NoExternal> {
    /// The use cases over `store`, with fresh ids from `ids` and the time from `clock`, running the builtin checks
    /// at their default levels.
    pub fn new(store: S, ids: G, clock: impl Clock + 'static) -> App<S, G> {
        App::with_registry(store, ids, Registry::defaults(), clock)
    }

    /// The use cases over `store`, with fresh ids from `ids` and the time from `clock`, running the checks of
    /// `registry`.
    pub fn with_registry(
        store: S,
        ids: G,
        registry: Registry,
        clock: impl Clock + 'static,
    ) -> App<S, G> {
        App {
            store,
            ids,
            registry,
            external: NoExternal,
            runner: Box::new(NoRunner),
            clock: Box::new(clock),
        }
    }
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Supplies the external execution port without changing the project or registry.
    pub fn with_external<N: ExternalExecutor>(self, external: N) -> App<S, G, N> {
        App {
            store: self.store,
            ids: self.ids,
            registry: self.registry,
            external,
            runner: self.runner,
            clock: self.clock,
        }
    }

    /// Supplies the measure runner; without one, `measure` is refused and the checks of measurements only judge
    /// what is already kept.
    pub fn with_runner(mut self, runner: impl Runner + 'static) -> Self {
        self.runner = Box::new(runner);
        self
    }

    /// The checks and the policy they run under.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The store, given back (to close it).
    pub fn into_store(self) -> S {
        self.store
    }

    /// The store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Makes the Project.
    pub async fn init(&mut self) -> Result<ProjectId, AppError> {
        let at = self.store.revision().await?;
        if self.store.project().await?.is_some() {
            return Err(AppError::AlreadyInitialized);
        }
        let id = ProjectId::from_uuid(self.ids.next_uuid());
        self.commit_project(at, Project::new(id)).await?;
        Ok(id)
    }

    /// Adds a Context named `name` that uses the Contexts named in `uses`.
    pub async fn add_context(
        &mut self,
        name: &str,
        uses: &[String],
    ) -> Result<ContextId, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let uses = context_ids(&p, uses)?;
        let id = ContextId::from_uuid(self.ids.next_uuid());
        let context = crate::domain::entity::context::Context::new(
            id,
            ContextName::try_from(name).map_err(input)?,
            uses,
        )
        .map_err(ProjectError::from)?;
        p.add_context(context)?;
        self.commit_project(at, p).await?;
        Ok(id)
    }

    /// Sets the Contexts the Context named `context` uses.
    pub async fn set_uses(&mut self, context: &str, uses: &[String]) -> Result<(), AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let ctx = context_id(&p, context)?;
        let uses = context_ids(&p, uses)?;
        p.set_uses(ctx, uses)?;
        self.commit_project(at, p).await?;
        Ok(())
    }

    /// Bases the Context named `context` on its Contracts.
    pub async fn base_on_contract(&mut self, context: &str) -> Result<(), AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let ctx = context_id(&p, context)?;
        p.base_on_contract(ctx)?;
        self.commit_project(at, p).await?;
        Ok(())
    }

    /// Makes or revises the Contract named `name` of the Context `context`. A Clause keeps its id when its label
    /// is one the Contract has.
    pub async fn put_contract(
        &mut self,
        context: &str,
        name: &str,
        title: &str,
        clauses: &[ClauseInput],
    ) -> Result<Checked<Put<ContractId>>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Contract, name)
            .await?
            .map(ContractId::from_uuid);
        if let Some(id) = id {
            refuse_retired(&p, Target::Contract(id), context, AssetKind::Contract, name)?;
        }
        let old = id.and_then(|id| p.context(ctx).and_then(|c| c.contract(id)));
        let mut given = Vec::with_capacity(clauses.len());
        for c in clauses {
            let label = ClauseLabel::try_from(c.label.as_str()).map_err(input)?;
            let text = ClauseText::try_from(c.text.as_str()).map_err(input)?;
            let clause = match old.and_then(|k| k.clause_by_label(label)) {
                Some(k) => k.id(),
                None => ClauseId::from_uuid(self.ids.next_uuid()),
            };
            given.push((clause, ClauseContent::new(label, text)));
        }
        let id = id.unwrap_or_else(|| ContractId::from_uuid(self.ids.next_uuid()));
        p.put_contract(ctx, id, Title::try_from(title).map_err(input)?, given)?;
        let revision = p
            .context(ctx)
            .and_then(|c| c.contract(id))
            .map(|k| k.revision())
            .unwrap_or_else(Revision::first);
        let named = AssetName {
            context: ctx,
            kind: AssetKind::Contract,
            name: name.to_string(),
            id: id.as_uuid(),
        };
        let gate = self
            .gate(
                Hook::Put,
                at,
                Some(&before),
                &p,
                std::slice::from_ref(&named),
            )
            .await?;
        self.keep(at, p, named, gate.invocations, gate.external)
            .await?;
        Ok(Checked {
            value: Put { id, revision },
            findings: gate.found,
        })
    }

    /// Makes or revises the Interface named `name` of the Context `context` from its code and the Items found in
    /// it. An Item keeps its id when its signature (or the one it `replaces`) is one the Interface has.
    pub async fn put_interface(
        &mut self,
        context: &str,
        name: &str,
        source: &str,
        items: &[ItemInput],
    ) -> Result<Checked<Put<InterfaceId>>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Interface, name)
            .await?
            .map(InterfaceId::from_uuid);
        if let Some(id) = id {
            refuse_retired(
                &p,
                Target::Interface(id),
                context,
                AssetKind::Interface,
                name,
            )?;
        }
        let old = id.and_then(|id| p.context(ctx).and_then(|c| c.interface(id)));
        let find = |sig: &str| {
            old.and_then(|i| i.items().iter().find(|it| it.signature().as_str() == sig))
                .map(|it| it.id())
        };
        let mut given = Vec::with_capacity(items.len());
        for it in items {
            let signature = Signature::try_from(it.signature.as_str()).map_err(input)?;
            let lines = LineRange::new(it.lines.0, it.lines.1).map_err(input)?;
            let item = match &it.replaces {
                Some(before) => find(before).ok_or_else(|| AppError::UnknownItem {
                    interface: name.to_string(),
                    signature: before.clone(),
                })?,
                None => match find(&it.signature) {
                    Some(item) => item,
                    None => ItemId::from_uuid(self.ids.next_uuid()),
                },
            };
            given.push((item, signature, lines));
        }
        let id = id.unwrap_or_else(|| InterfaceId::from_uuid(self.ids.next_uuid()));
        p.put_interface(ctx, id, Source::new(source.to_string()), given)?;
        let revision = p
            .context(ctx)
            .and_then(|c| c.interface(id))
            .map(|i| i.revision())
            .unwrap_or_else(Revision::first);
        let named = AssetName {
            context: ctx,
            kind: AssetKind::Interface,
            name: name.to_string(),
            id: id.as_uuid(),
        };
        let gate = self
            .gate(
                Hook::Put,
                at,
                Some(&before),
                &p,
                std::slice::from_ref(&named),
            )
            .await?;
        self.keep(at, p, named, gate.invocations, gate.external)
            .await?;
        Ok(Checked {
            value: Put { id, revision },
            findings: gate.found,
        })
    }

    /// Makes or revises the Harness named `name` of the Context `context` from its code and the Cases found in
    /// it. A Case keeps its id when its name is one the Harness has. Each citation records the revision its target
    /// has now: putting a Case says it is written against the definitions as they are.
    pub async fn put_harness(
        &mut self,
        context: &str,
        name: &str,
        source: &str,
        cases: &[CaseInput],
    ) -> Result<Checked<Put<HarnessId>>, AppError> {
        self.put_harness_of(context, name, None, source, cases)
            .await
    }

    /// As [`App::put_harness`], and the Harness is of `kind`: `tests`, or `sim` for a simulation script run by
    /// `simulate`. Without a kind a new Harness is of tests and an existing one keeps its kind.
    pub async fn put_harness_of(
        &mut self,
        context: &str,
        name: &str,
        kind: Option<HarnessKind>,
        source: &str,
        cases: &[CaseInput],
    ) -> Result<Checked<Put<HarnessId>>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = self
            .store
            .asset_id(ctx, AssetKind::Harness, name)
            .await?
            .map(HarnessId::from_uuid);
        if let Some(id) = id {
            refuse_retired(&p, Target::Harness(id), context, AssetKind::Harness, name)?;
        }
        let old = id.and_then(|id| p.context(ctx).and_then(|c| c.harness(id)));
        let mut given = Vec::with_capacity(cases.len());
        for c in cases {
            let case_name = CaseName::try_from(c.name.as_str()).map_err(input)?;
            let lines = LineRange::new(c.lines.0, c.lines.1).map_err(input)?;
            let mut items = std::collections::BTreeMap::new();
            for r in &c.items {
                let (item, revision) = self.item(&p, Some(ctx), r).await?;
                items.insert(item, revision);
            }
            let mut clauses = std::collections::BTreeMap::new();
            for r in &c.clauses {
                let (clause, revision) = self.clause(&p, Some(ctx), r).await?;
                clauses.insert(clause, revision);
            }
            let cites = CaseCites::new(items, clauses).map_err(input)?;
            let case = match old.and_then(|h| h.cases().iter().find(|k| *k.name() == case_name)) {
                Some(k) => k.id(),
                None => CaseId::from_uuid(self.ids.next_uuid()),
            };
            given.push((case, case_name, lines, cites));
        }
        let id = id.unwrap_or_else(|| HarnessId::from_uuid(self.ids.next_uuid()));
        p.put_harness_as(ctx, id, kind, Source::new(source.to_string()), given)?;
        let revision = p
            .context(ctx)
            .and_then(|c| c.harness(id))
            .map(|h| h.revision())
            .unwrap_or_else(Revision::first);
        let named = AssetName {
            context: ctx,
            kind: AssetKind::Harness,
            name: name.to_string(),
            id: id.as_uuid(),
        };
        let gate = self
            .gate(
                Hook::Put,
                at,
                Some(&before),
                &p,
                std::slice::from_ref(&named),
            )
            .await?;
        self.keep(at, p, named, gate.invocations, gate.external)
            .await?;
        Ok(Checked {
            value: Put { id, revision },
            findings: gate.found,
        })
    }

    /// Retires the Context named `context` and everything it owns: it leaves the model, the Project goes to its
    /// next revision, and its ids stay reserved. Refused while a Context that stays uses it or a Case that stays
    /// cites what it owns. Pins, change requests and measurements keep referring to it by id. Like a put, it does
    /// not need an open change request; do it as the work of one that expects the Context to be removed.
    pub async fn retire_context(&mut self, context: &str) -> Result<Checked<Retirement>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let done = p.retire_context(ctx);
        self.finish_retire(at, before, p, done).await
    }

    /// Retires the Contract named `contract` of the Context `context` with its Clauses. The Context goes to its next
    /// revision. See [`App::retire_context`].
    pub async fn retire_contract(
        &mut self,
        context: &str,
        contract: &str,
    ) -> Result<Checked<Retirement>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = ContractId::from_uuid(
            self.asset(ctx, context, AssetKind::Contract, contract)
                .await?,
        );
        refuse_retired(
            &p,
            Target::Contract(id),
            context,
            AssetKind::Contract,
            contract,
        )?;
        let done = p.retire_contract(ctx, id);
        self.finish_retire(at, before, p, done).await
    }

    /// Retires the Interface named `interface` of the Context `context` with its Items. The Context goes to its
    /// next revision. A Pin of it stays and reads `gone`. See [`App::retire_context`].
    pub async fn retire_interface(
        &mut self,
        context: &str,
        interface: &str,
    ) -> Result<Checked<Retirement>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = InterfaceId::from_uuid(
            self.asset(ctx, context, AssetKind::Interface, interface)
                .await?,
        );
        refuse_retired(
            &p,
            Target::Interface(id),
            context,
            AssetKind::Interface,
            interface,
        )?;
        let done = p.retire_interface(ctx, id);
        self.finish_retire(at, before, p, done).await
    }

    /// Retires the Harness named `harness` of the Context `context` with its Cases. The Context goes to its next
    /// revision. See [`App::retire_context`].
    pub async fn retire_harness(
        &mut self,
        context: &str,
        harness: &str,
    ) -> Result<Checked<Retirement>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let ctx = context_id(&p, context)?;
        let id = HarnessId::from_uuid(
            self.asset(ctx, context, AssetKind::Harness, harness)
                .await?,
        );
        refuse_retired(
            &p,
            Target::Harness(id),
            context,
            AssetKind::Harness,
            harness,
        )?;
        let done = p.retire_harness(ctx, id);
        self.finish_retire(at, before, p, done).await
    }

    /// Checks and keeps a Project that retired something (`done`: how the retirement went on `after`, which is
    /// unchanged when it failed), and says what went. The names of what went stay in the store: they are how the
    /// retired things are named later.
    async fn finish_retire(
        &self,
        at: StoreRevision,
        before: Project,
        after: Project,
        done: Result<(), ProjectError>,
    ) -> Result<Checked<Retirement>, AppError> {
        if let Err(e) = done {
            return Err(self.retire_error(&before, e).await?);
        }
        let gate = self.gate(Hook::Put, at, Some(&before), &after, &[]).await?;
        let mut retired = Vec::new();
        for r in after.retired().iter().skip(before.retired().len()) {
            retired.push(self.retired_status(r).await?);
        }
        let changes = Changes {
            project: Some(after),
            invocations: gate.invocations,
            external: gate.external,
            ..Changes::default()
        };
        self.store.commit(at, changes).await?;
        Ok(Checked {
            value: Retirement { retired },
            findings: gate.found,
        })
    }

    /// A refusal of a retirement, with the Cases and Contexts that stand in the way named.
    async fn retire_error(&self, p: &Project, e: ProjectError) -> Result<AppError, AppError> {
        Ok(match e {
            ProjectError::RetireCited { cited, cases } => {
                let cases = cases.into_iter().map(Target::Case).collect::<Vec<_>>();
                AppError::RetireCited {
                    cited: self.describe_all(p, &cited).await?,
                    cases: self.describe_all(p, &cases).await?,
                }
            }
            ProjectError::RetireUsed { context, by } => AppError::RetireUsed {
                context: p
                    .context(context)
                    .map_or_else(|| context.to_string(), |c| c.name().to_string()),
                by: by
                    .iter()
                    .map(|id| {
                        p.context(*id)
                            .map_or_else(|| id.to_string(), |c| c.name().to_string())
                    })
                    .collect(),
            },
            other => other.into(),
        })
    }

    /// A retired thing in the names a person knows it by.
    async fn retired_status(&self, r: &Retired) -> Result<RetiredStatus, AppError> {
        Ok(RetiredStatus {
            kind: r.target().word(),
            context: r.context().to_string(),
            name: self.retired_name(r).await?,
            last_revision: r.last_revision().number(),
            at: r.at().number(),
        })
    }

    /// The name of a retired thing: the Context's, the asset's, or the asset's with the label, signature or name.
    async fn retired_name(&self, r: &Retired) -> Result<String, AppError> {
        match (r.holder(), r.label(), r.target().as_uuid()) {
            (Some(holder), Some(label), _) => {
                Ok(format!("{} {label}", self.name_of(holder).await?))
            }
            (_, _, Some(id)) if !matches!(r.target(), Target::Context(_)) => self.name_of(id).await,
            _ => Ok(r.context().to_string()),
        }
    }

    /// Pins `code` as the code of the Interface named `interface` of the Context `context`, replacing its Pin.
    pub async fn pin(
        &mut self,
        context: &str,
        interface: &str,
        code: &str,
    ) -> Result<Checked<PinId>, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let ctx = context_id(&p, context)?;
        let iface = self
            .asset(ctx, context, AssetKind::Interface, interface)
            .await?;
        let id = PinId::from_uuid(self.ids.next_uuid());
        let pin = record(
            &p,
            InterfaceId::from_uuid(iface),
            id,
            Digest::of(code.as_bytes()),
        )?;
        let mut pins = self.store.pins().await?;
        pins.retain(|k| k.interface() != pin.interface());
        pins.push(pin.clone());
        let crs = self.store.change_requests().await?;
        let gate = self
            .gate_with(Hook::Pin, at, &view(Some(&p), &p, &pins, &crs), &[])
            .await?;
        let changes = Changes {
            pins: vec![pin],
            pin_codes: vec![PinCode {
                pin: id,
                interface: InterfaceId::from_uuid(iface),
                code: code.to_string(),
            }],
            invocations: gate.invocations,
            external: gate.external,
            ..Changes::default()
        };
        self.store.commit(at, changes).await?;
        Ok(Checked {
            value: id,
            findings: gate.found,
        })
    }

    /// Opens a change request: what is asked and why, whether it means to change the behaviour, the change request
    /// it is a part of, who raises it, the definitions it is about (each citation names its Context), and the
    /// question for a person, whose options may name the definitions they ask to change. Gives back the id and the
    /// lines as recorded.
    pub async fn open_change_request(
        &mut self,
        c: &ChangeRequestInput,
    ) -> Result<Opened, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let title = Title::try_from(c.title.as_str()).map_err(input)?;
        let existing = self.store.change_requests().await?;
        if existing.iter().any(|cr| *cr.title() == title) {
            return Err(AppError::TitleTaken(title.to_string()));
        }
        let parent = match &c.parent {
            None => None,
            Some(t) => Some(
                existing
                    .iter()
                    .find(|cr| cr.title().as_str() == t.trim())
                    .map(|cr| cr.id())
                    .ok_or_else(|| AppError::UnknownChangeRequest(t.clone()))?,
            ),
        };
        let mut wanted: Vec<(Target, ExpectKind)> = Vec::with_capacity(c.expects.len());
        for e in &c.expects {
            let target = self.target_of(&p, &e.target).await?;
            if wanted.iter().any(|(t, _)| *t == target) {
                return Err(AppError::Input(format!(
                    "{} is given twice; a target has one expectation",
                    self.describe(&p, target).await?
                )));
            }
            wanted.push((target, e.kind));
        }
        let crux = match &c.crux {
            None => None,
            Some(q) => {
                let mut options = Vec::with_capacity(q.options.len());
                for o in &q.options {
                    let mut expects = BTreeMap::new();
                    for e in &o.expects {
                        let target = self.target_of(&p, &e.target).await?;
                        let twice = expects.insert(target, e.kind).is_some();
                        let clashes = wanted.iter().any(|(t, k)| *t == target && *k != e.kind);
                        if twice || clashes {
                            return Err(AppError::Input(format!(
                                "{} is expected twice, in different ways, by the change request and the option `{}`",
                                self.describe(&p, target).await?,
                                o.choice
                            )));
                        }
                    }
                    options.push(CruxOption::new(
                        Choice::try_from(o.choice.as_str()).map_err(input)?,
                        prose(&o.change)?,
                        expects,
                        prose(&o.consequence)?,
                    ));
                }
                let recommended = q
                    .recommended
                    .as_ref()
                    .map(|r| {
                        Ok::<_, AppError>(Recommended::new(
                            Choice::try_from(r.choice.as_str()).map_err(input)?,
                            prose(&r.reason)?,
                            r.confidence
                                .map(|c| Confidence::try_from(c).map_err(input))
                                .transpose()?,
                        ))
                    })
                    .transpose()?;
                let supersedes = match &q.supersedes {
                    None => None,
                    Some(t) => {
                        let replaced = existing
                            .iter()
                            .find(|cr| cr.title().as_str() == t.trim())
                            .map(|cr| cr.id())
                            .ok_or_else(|| AppError::UnknownChangeRequest(t.clone()))?;
                        check_supersedes(replaced, &existing)?;
                        Some(replaced)
                    }
                };
                Some(Crux::new(
                    q.kind,
                    options,
                    recommended,
                    q.importance,
                    supersedes,
                )?)
            }
        };
        let assumptions = c
            .assumptions
            .iter()
            .map(assumption_of)
            .collect::<Result<Vec<_>, _>>()?;
        let issuer = c
            .issuer
            .as_deref()
            .map(|i| Issuer::try_from(i).map_err(input))
            .transpose()?;
        let mut addresses = Vec::with_capacity(c.addresses.len());
        for a in &c.addresses {
            addresses.push(self.finding_ref(&p, a).await?);
        }
        let id = ChangeRequestId::from_uuid(self.ids.next_uuid());
        let cr = ChangeRequest::open(
            id,
            ChangeRequestDraft {
                title,
                situation: prose(&c.situation)?,
                intent: c.intent,
                parent,
                issuer,
                expects: expectations_now(&p, wanted),
                crux,
                assumptions,
                addresses,
            },
        );
        let mut expects = Vec::with_capacity(cr.expects().len());
        for (target, e) in cr.expects() {
            let Some(named) = self.name_target(&p, *target).await? else {
                return Err(AppError::Input(format!(
                    "{} is not in the Project",
                    self.describe(&p, *target).await?
                )));
            };
            expects.push(OpenedExpect {
                kind: e.kind,
                target: named,
                recorded: e.recorded,
            });
        }
        self.commit_change_request(at, cr, Vec::new(), Vec::new())
            .await?;
        Ok(Opened { id, expects })
    }

    /// Records the answer to the Crux of the change request titled `title`: the `decision`, what it rests on
    /// (`basis`: a line of the Project's Policy, which must exist, or an arbitrary reason), who decided it (`by`),
    /// and when (from the clock). The answer is recorded once and never rewritten: a Crux already answered refuses
    /// another. The Policy line is cited at the revision the Policy has now.
    pub async fn decide_change_request(
        &mut self,
        title: &str,
        decision: &DecisionInput,
        basis: &BasisInput,
        by: &DecisionMakerInput,
    ) -> Result<(), AppError> {
        let now = self.clock.now();
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let mut cr = self.change_request_titled(title).await?;
        let basis = match basis {
            BasisInput::Policy(line) => {
                let line = PolicyLineId::try_from(line.as_str()).map_err(input)?;
                p.policy()
                    .basis_on(line)
                    .map_err(|_| ChangeRequestError::UnknownPolicyLine(line))?
            }
            BasisInput::Arbitrary(reason) => Basis::Arbitrary {
                reason: Prose::try_from(reason.as_str()).map_err(input)?,
            },
        };
        let by = match by {
            DecisionMakerInput::Role(name) => DecisionMaker::role(name),
            DecisionMakerInput::Person(name) => DecisionMaker::person(name),
        }
        .map_err(input)?;
        let decision = match decision {
            DecisionInput::Chosen(r) => Decision::Chosen(reasoned(r)?),
            DecisionInput::Worded { wording, reason } => Decision::Worded {
                wording: Prose::try_from(wording.as_str()).map_err(input)?,
                reason: prose(reason)?,
            },
            DecisionInput::Unspecified { keep } => Decision::Unspecified {
                keep: keep
                    .iter()
                    .map(|k| Prose::try_from(k.as_str()).map_err(input))
                    .collect::<Result<_, _>>()?,
            },
        };
        decide(&mut cr, Answer::new(decision, basis, by, now), &p)?;
        self.commit_change_request(at, cr, Vec::new(), Vec::new())
            .await?;
        Ok(())
    }

    /// Appends an Assumption to the change request titled `title`. Assumptions only grow; a resolved change
    /// request takes none.
    pub async fn assume(
        &mut self,
        title: &str,
        assumption: &AssumptionInput,
    ) -> Result<(), AppError> {
        let at = self.store.revision().await?;
        let mut cr = self.change_request_titled(title).await?;
        cr.assume(assumption_of(assumption)?)?;
        self.commit_change_request(at, cr, Vec::new(), Vec::new())
            .await?;
        Ok(())
    }

    /// Replaces the decision Policy of the Project with `lines`, whole. Its revision goes one up when they differ
    /// from the ones it has. The checks of the put hook run (`crux.policy-line-gone` notes a decision that rests
    /// on a line that is now gone or reworded).
    pub async fn put_policy(
        &mut self,
        lines: &[PolicyLineInput],
    ) -> Result<Checked<PolicyPut>, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let before = p.clone();
        let mut given = Vec::with_capacity(lines.len());
        for l in lines {
            given.push((
                PolicyLineId::try_from(l.id.as_str()).map_err(input)?,
                Prose::try_from(l.text.as_str()).map_err(input)?,
            ));
        }
        p.put_policy(given)?;
        let revision = p.policy().revision();
        let gate = self.gate(Hook::Put, at, Some(&before), &p, &[]).await?;
        let changes = Changes {
            project: Some(p),
            invocations: gate.invocations,
            external: gate.external,
            ..Changes::default()
        };
        self.store.commit(at, changes).await?;
        Ok(Checked {
            value: PolicyPut { revision },
            findings: gate.found,
        })
    }

    /// Closes the record of the change request titled `title`, once the change is applied.
    pub async fn resolve_change_request(&mut self, title: &str) -> Result<Checked<()>, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let mut cr = self.change_request_titled(title).await?;
        let mut crs = self.store.change_requests().await?;
        if let Err(e) = resolve(&mut cr, &p, &crs) {
            return Err(match e {
                ChangeRequestError::NotApplied {
                    unchanged,
                    gone,
                    not_removed,
                } => AppError::NotApplied {
                    unchanged: self.describe_all(&p, &unchanged).await?,
                    gone: self.describe_all(&p, &gone).await?,
                    not_removed: self.describe_all(&p, &not_removed).await?,
                },
                ChangeRequestError::KeepMoved { moved } => AppError::KeepMoved {
                    moved: self.describe_all(&p, &moved).await?,
                },
                ChangeRequestError::ChildrenNotResolved { children } => {
                    AppError::ChildrenNotResolved {
                        children: children
                            .iter()
                            .map(|id| {
                                crs.iter()
                                    .find(|c| c.id() == *id)
                                    .map_or_else(|| id.to_string(), |c| c.title().to_string())
                            })
                            .collect(),
                    }
                }
                other => other.into(),
            });
        }
        let pins = self.store.pins().await?;
        crs.retain(|k| k.id() != cr.id());
        crs.push(cr.clone());
        let gate = self
            .gate_with(Hook::Resolve, at, &view(Some(&p), &p, &pins, &crs), &[])
            .await?;
        self.commit_change_request(at, cr, gate.invocations, gate.external)
            .await?;
        Ok(Checked {
            value: (),
            findings: gate.found,
        })
    }

    /// Runs every check on the state as it is: what the person would see if each check ran now, with what agents
    /// reported for the judged rules. Nothing is kept, and the runs are not recorded.
    pub async fn check(&self) -> Result<Vec<Found>, AppError> {
        Ok(self.checked().await?.0)
    }

    /// The findings of [`App::check`], the judged rules not up to date, and the measurements they read.
    async fn checked(
        &self,
    ) -> Result<(Vec<Found>, Vec<RuleStanding>, measure::Measured), AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let pins = self.store.pins().await?;
        let crs = self.store.change_requests().await?;
        let measured = self.measured(&p).await?;
        let run = self.registry.run(
            Hook::Check,
            at,
            &CheckView {
                before: None,
                after: &p,
                pins: &pins,
                change_requests: &crs,
                measurements: &measured.measurements,
                measure_settings: &measured.settings,
                sim_settings: &measured.sim_settings,
            },
        );
        let mut found = self.found(&run.findings, &p, &crs, &[]).await?;
        found.extend(self.external_standing(&p, &crs, at).await?.0);
        let (judged, standing) = self.judged_standing(&p, &crs, at).await?;
        found.extend(judged);
        Ok((found, standing, measured))
    }

    /// The change request titled `title`.
    async fn change_request_titled(&self, title: &str) -> Result<ChangeRequest, AppError> {
        self.store
            .change_requests()
            .await?
            .into_iter()
            .find(|c| c.title().as_str() == title.trim())
            .ok_or_else(|| AppError::UnknownChangeRequest(title.to_string()))
    }

    /// Where the Project stands.
    pub async fn status(&self) -> Result<Status, AppError> {
        let at = self.store.revision().await?;
        let p = self.project().await?;
        let mut pins = Vec::new();
        let mut seen = BTreeSet::new();
        for c in p.contexts() {
            for i in c.interfaces() {
                seen.insert(i.id());
                let state = match self.store.pin_of(i.id()).await? {
                    None => PinState::Unpinned,
                    Some(pin) => match staleness(&pin, &p) {
                        Staleness::Fresh => PinState::Fresh,
                        Staleness::InterfaceGone => PinState::Gone,
                        Staleness::Stale(why) => {
                            let mut said = Vec::with_capacity(why.len());
                            for r in &why {
                                said.push(self.reason(r).await?);
                            }
                            PinState::Stale(said)
                        }
                    },
                };
                pins.push(PinStatus {
                    context: c.name().to_string(),
                    interface: self.name_of(i.id().as_uuid()).await?,
                    state,
                });
            }
        }
        for pin in self.store.pins().await? {
            if !seen.contains(&pin.interface()) {
                let name = self.name_of(pin.interface().as_uuid()).await?;
                let (context, interface) = match p.retired_of(Target::Interface(pin.interface())) {
                    Some(r) => (r.context().to_string(), format!("{name} (retired)")),
                    None => (String::new(), name),
                };
                pins.push(PinStatus {
                    context,
                    interface,
                    state: PinState::Gone,
                });
            }
        }
        let mut retired = Vec::with_capacity(p.retired().len());
        for r in p.retired() {
            retired.push(self.retired_status(r).await?);
        }
        let mut stale = Vec::new();
        for s in stale_cites(&p) {
            stale.push(StaleCiteStatus {
                case: case_name(&p, s.case),
                target: self.describe(&p, s.target).await?,
                cited: s.cited.number(),
                now: s.now.number(),
            });
        }
        let crs = self.store.change_requests().await?;
        let (findings, judged, measured) = self.checked().await?;
        let measurements = self.measurement_standings(&p, &measured).await?;
        let title_of = |id: ChangeRequestId| {
            crs.iter()
                .find(|c| c.id() == id)
                .map(|c| c.title().to_string())
                .unwrap_or_else(|| id.to_string())
        };
        let decision_of =
            |c: &ChangeRequest| c.crux().and_then(|q| q.decision()).map(decision_text);
        let records = crs
            .iter()
            .map(|c| ChangeRequestStatus {
                title: c.title().to_string(),
                intent: c.intent().word(),
                parent: c.parent().map(title_of),
                issuer: c.issuer().map(|i| i.to_string()),
                crux: c.crux().map(|q| q.kind().word()),
                supersedes: c.crux().and_then(Crux::supersedes).map(title_of),
                state: c.state().word(),
                answer: c.crux().and_then(Crux::answer).map(answer_status),
                assumptions: c.assumptions().iter().map(assumption_status).collect(),
            })
            .collect();
        let mut open = Vec::new();
        for u in unapplied(crs.iter(), &p) {
            let Some(cr) = crs.iter().find(|c| c.id() == u.cr) else {
                continue;
            };
            open.push(UnappliedStatus {
                cr: cr.title().to_string(),
                decision: decision_of(cr),
                unchanged: self.describe_all(&p, &u.unchanged).await?,
                gone: self.describe_all(&p, &u.gone).await?,
                not_removed: self.describe_all(&p, &u.not_removed).await?,
            });
        }
        let mut moved = Vec::new();
        for k in keep_moved(crs.iter(), &p) {
            let Some(cr) = crs.iter().find(|c| c.id() == k.cr) else {
                continue;
            };
            moved.push(KeepMovedStatus {
                cr: cr.title().to_string(),
                moved: self.describe_all(&p, &k.moved).await?,
            });
        }
        let mut awaiting = Vec::new();
        for c in awaiting_decision(crs.iter()) {
            let Some(q) = c.crux() else {
                continue;
            };
            let mut options = Vec::with_capacity(q.options().len());
            for o in q.options() {
                let mut expects = Vec::with_capacity(o.expects().len());
                for (target, kind) in o.expects() {
                    expects.push(AwaitingExpect {
                        kind: kind.word(),
                        target: self.describe(&p, *target).await?,
                    });
                }
                options.push(AwaitingOption {
                    choice: o.choice().to_string(),
                    wording: o.change().map(|w| w.to_string()),
                    expects,
                    consequence: o.consequence().map(|w| w.to_string()),
                });
            }
            awaiting.push(AwaitingStatus {
                cr: c.title().to_string(),
                kind: q.kind().word(),
                options,
                recommended: q.recommended().map(|r| RecommendedStatus {
                    choice: r.choice().to_string(),
                    reason: r.reason().map(|w| w.to_string()),
                    confidence: r.confidence().map(Confidence::percent),
                }),
                importance: q.importance().map(Importance::word),
            });
        }
        let policy = PolicyStatus {
            revision: p.policy().revision().number(),
            lines: p
                .policy()
                .lines()
                .iter()
                .map(|l| PolicyLineStatus {
                    id: l.id().to_string(),
                    text: l.text().to_string(),
                })
                .collect(),
        };
        Ok(Status {
            retired,
            pins,
            stale_cites: stale,
            change_requests: records,
            unapplied: open,
            keep_moved: moved,
            awaiting,
            policy,
            findings,
            judged,
            external: self.external_standing(&p, &crs, at).await?.1,
            measurements,
        })
    }

    async fn project(&self) -> Result<Project, AppError> {
        self.store.project().await?.ok_or(AppError::NoProject)
    }

    /// Commits `p` alone, as the store stood at `at` when the use case began to read.
    async fn commit_project(&self, at: StoreRevision, p: Project) -> Result<(), AppError> {
        let changes = Changes {
            project: Some(p),
            ..Changes::default()
        };
        Ok(self.store.commit(at, changes).await?)
    }

    /// Commits `cr` alone, as the store stood at `at` when the use case began to read.
    async fn commit_change_request(
        &self,
        at: StoreRevision,
        cr: ChangeRequest,
        invocations: Vec<Invocation>,
        external: Vec<ExternalReport>,
    ) -> Result<(), AppError> {
        let changes = Changes {
            change_requests: vec![cr],
            invocations,
            external,
            ..Changes::default()
        };
        Ok(self.store.commit(at, changes).await?)
    }

    /// Commits `p` and the name of the asset that was just made or revised, together: a Project holding an asset
    /// its name does not reach (or the reverse) is never kept. The `invocations` of the checks that ran go with
    /// them.
    async fn keep(
        &self,
        at: StoreRevision,
        p: Project,
        name: AssetName,
        invocations: Vec<Invocation>,
        external: Vec<ExternalReport>,
    ) -> Result<(), AppError> {
        let changes = Changes {
            project: Some(p),
            names: vec![name],
            invocations,
            external,
            ..Changes::default()
        };
        Ok(self.store.commit(at, changes).await?)
    }

    /// Runs the checks of `hook` on `after`, the Project as the use case would keep it, with the Pins and the
    /// change requests as the store has them. `pending` are the names the use case is about to give, which the
    /// store does not have yet.
    async fn gate(
        &self,
        hook: Hook,
        at: StoreRevision,
        before: Option<&Project>,
        after: &Project,
        pending: &[AssetName],
    ) -> Result<Gate, AppError> {
        let pins = self.store.pins().await?;
        let crs = self.store.change_requests().await?;
        self.gate_with(hook, at, &view(before, after, &pins, &crs), pending)
            .await
    }

    /// Runs the checks of `hook` on `after`, with the Pins and change requests the use case would leave. A finding
    /// of a mandatory check at `error` refuses the use case before anything is kept.
    async fn gate_with(
        &self,
        hook: Hook,
        at: StoreRevision,
        view: &CheckView<'_>,
        pending: &[AssetName],
    ) -> Result<Gate, AppError> {
        let mut run = self.registry.run(hook, at, view);
        let (after, crs) = (view.after, view.change_requests);
        if !run.refusing.is_empty() {
            return Err(AppError::Refused(
                self.found(&run.refusing, after, crs, pending).await?,
            ));
        }
        let external = self.run_external(hook, at, view, pending).await?;
        if external.iter().any(|r| {
            self.registry
                .external()
                .iter()
                .any(|rule| rule.descriptor().id.as_str() == r.rule && rule.gate())
                && !r.findings.is_empty()
        }) {
            let refusing = self
                .external_findings(&external, after, crs, pending, false)
                .await?;
            return Err(AppError::ExternalRefused {
                findings: refusing,
                external,
            });
        }
        run.invocations
            .extend(self.external_invocations(&external, hook, at));
        let mut found = self.found(&run.findings, after, crs, pending).await?;
        found.extend(
            self.external_findings(&external, after, crs, pending, false)
                .await?,
        );
        Ok(Gate {
            external,
            found,
            invocations: run.invocations,
        })
    }

    /// The findings as a caller sees them: their state among `crs`, and what they are about in words.
    async fn found(
        &self,
        findings: &[Finding],
        p: &Project,
        crs: &[ChangeRequest],
        pending: &[AssetName],
    ) -> Result<Vec<Found>, AppError> {
        let mut out = Vec::with_capacity(findings.len());
        for f in findings {
            let about = match f.subject() {
                None => None,
                Some(s) => Some(self.describe_subject(p, crs, s, pending).await?),
            };
            out.push(Found {
                rule: f.rule().clone(),
                level: f.level(),
                state: f.state_in(crs),
                message: self.worded(f, pending).await?,
                about,
                subject: f.subject(),
                stale: false,
                judgment: None,
            });
        }
        Ok(out)
    }

    /// The message of a finding, with the names of the things it mentions written after it
    /// (`the code was pinned against what has moved: contract `rounding` changed`).
    async fn worded(&self, f: &Finding, pending: &[AssetName]) -> Result<String, AppError> {
        if f.mentions().is_empty() {
            return Ok(f.message().to_string());
        }
        let mut said = Vec::with_capacity(f.mentions().len());
        for m in f.mentions() {
            let (kind, id) = match m {
                Mention::Interface(id) => ("interface", id.as_uuid()),
                Mention::Harness(id) => ("harness", id.as_uuid()),
                Mention::Contract(id) => ("contract", id.as_uuid()),
            };
            said.push(format!(
                "{kind} `{}` changed",
                self.name_in(id, pending).await?
            ));
        }
        Ok(format!("{}: {}", f.message(), said.join(", ")))
    }

    /// What a finding is about, in words.
    async fn describe_subject(
        &self,
        p: &Project,
        crs: &[ChangeRequest],
        subject: Subject,
        pending: &[AssetName],
    ) -> Result<String, AppError> {
        Ok(match subject {
            Subject::Context(id) => match (p.context(id), p.retired_of(Target::Context(id))) {
                (Some(c), _) => format!("context {}", c.name()),
                (None, Some(r)) => format!("context {} (retired)", r.context()),
                (None, None) => format!("context {id}"),
            },
            Subject::Interface(id) => {
                let name = self.name_in(id.as_uuid(), pending).await?;
                if p.retired_of(Target::Interface(id)).is_some() {
                    format!("interface {name} (retired)")
                } else {
                    format!("interface {name}")
                }
            }
            Subject::Item(id) => self.describe_in(p, Target::Item(id), pending).await?,
            Subject::Clause(id) => self.describe_in(p, Target::Clause(id), pending).await?,
            Subject::Case(id) => match p.retired_of(Target::Case(id)) {
                Some(_) => self.describe_in(p, Target::Case(id), pending).await?,
                None => format!("case {}", case_name(p, id)),
            },
            Subject::ChangeRequest(id) => crs
                .iter()
                .find(|c| c.id() == id)
                .map(|c| format!("change request `{}`", c.title()))
                .unwrap_or_else(|| format!("change request {id}")),
        })
    }

    /// The finding a person names (`input`) as the thing a change request addresses.
    async fn finding_ref(&self, p: &Project, named: &FindingInput) -> Result<FindingRef, AppError> {
        let rule = RuleId::try_from(named.rule.as_str()).map_err(input)?;
        let subject = match &named.subject {
            Some(subject) => Some(self.subject_of(p, subject).await?),
            None => None,
        };
        Ok(FindingRef::new(rule, subject))
    }

    /// The id of the asset of `kind` named `name` in the Context `ctx` (named `context`).
    async fn asset(
        &self,
        ctx: ContextId,
        context: &str,
        kind: AssetKind,
        name: &str,
    ) -> Result<uuid::Uuid, AppError> {
        self.store
            .asset_id(ctx, kind, name)
            .await?
            .ok_or_else(|| AppError::UnknownAsset {
                context: context.to_string(),
                kind: kind.word(),
                name: name.to_string(),
            })
    }

    /// The Context a citation is in: the one it names, else `default`.
    fn cite_context(
        &self,
        p: &Project,
        named: Option<&String>,
        default: Option<ContextId>,
    ) -> Result<(ContextId, String), AppError> {
        match (named, default) {
            (Some(n), _) => Ok((context_id(p, n)?, n.clone())),
            (None, Some(id)) => Ok((
                id,
                p.context(id)
                    .map(|c| c.name().to_string())
                    .unwrap_or_default(),
            )),
            (None, None) => Err(AppError::ContextNotGiven),
        }
    }

    /// The id and current revision of the Item `r` cites.
    async fn item(
        &self,
        p: &Project,
        default: Option<ContextId>,
        r: &ItemRef,
    ) -> Result<(ItemId, Revision), AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), default)?;
        let iface = InterfaceId::from_uuid(
            self.asset(ctx, &ctx_name, AssetKind::Interface, &r.interface)
                .await?,
        );
        p.context(ctx)
            .and_then(|c| c.interface(iface))
            .and_then(|i| {
                i.items()
                    .iter()
                    .find(|it| it.signature().as_str() == r.signature)
            })
            .map(|it| (it.id(), it.revision()))
            .ok_or_else(|| AppError::UnknownItem {
                interface: r.interface.clone(),
                signature: r.signature.clone(),
            })
    }

    /// The id and current revision of the Clause `r` cites.
    async fn clause(
        &self,
        p: &Project,
        default: Option<ContextId>,
        r: &ClauseRef,
    ) -> Result<(ClauseId, Revision), AppError> {
        let (ctx, ctx_name) = self.cite_context(p, r.context.as_ref(), default)?;
        let contract = ContractId::from_uuid(
            self.asset(ctx, &ctx_name, AssetKind::Contract, &r.contract)
                .await?,
        );
        let label = ClauseLabel::try_from(r.label.as_str()).map_err(input)?;
        p.context(ctx)
            .and_then(|c| c.contract(contract))
            .and_then(|k| k.clause_by_label(label))
            .map(|k| (k.id(), k.revision()))
            .ok_or_else(|| AppError::UnknownClause {
                contract: r.contract.clone(),
                label: r.label.clone(),
            })
    }

    /// The name of an asset, or its id when it has none.
    async fn name_of(&self, id: uuid::Uuid) -> Result<String, AppError> {
        self.name_in(id, &[]).await
    }

    /// The name of an asset, a `pending` one (given by the use case, not kept yet) first.
    async fn name_in(&self, id: uuid::Uuid, pending: &[AssetName]) -> Result<String, AppError> {
        if let Some(n) = pending.iter().find(|n| n.id == id) {
            return Ok(n.name.clone());
        }
        Ok(self
            .store
            .asset_name(id)
            .await?
            .unwrap_or_else(|| id.to_string()))
    }

    /// A Pin's reason, in words.
    async fn reason(&self, r: &StaleReason) -> Result<String, AppError> {
        Ok(match r {
            StaleReason::Interface => "the interface changed".to_string(),
            StaleReason::Harness(id) => format!("harness `{}`", self.name_of(id.as_uuid()).await?),
            StaleReason::Contract(id) => {
                format!("contract `{}`", self.name_of(id.as_uuid()).await?)
            }
            StaleReason::UsedInterface(id) => {
                format!("interface `{}`", self.name_of(id.as_uuid()).await?)
            }
        })
    }

    /// Several targets, in words, in order.
    async fn describe_all(&self, p: &Project, targets: &[Target]) -> Result<Vec<String>, AppError> {
        let mut said = Vec::with_capacity(targets.len());
        for t in targets {
            said.push(self.describe(p, *t).await?);
        }
        Ok(said)
    }

    /// A citation's target, in words: the asset's name and the signature or label.
    async fn describe(&self, p: &Project, target: Target) -> Result<String, AppError> {
        self.describe_in(p, target, &[]).await
    }
}

/// What a change request still lacks, in words: the targets as they were, then the ones gone, then the ones to
/// remove that are still there.
fn not_applied_words(unchanged: &[String], gone: &[String], not_removed: &[String]) -> String {
    let mut parts = Vec::with_capacity(3);
    if !unchanged.is_empty() {
        parts.push(format!("not changed: {}", unchanged.join(", ")));
    }
    if !gone.is_empty() {
        parts.push(format!("gone: {}", gone.join(", ")));
    }
    if !not_removed.is_empty() {
        parts.push(format!("not removed: {}", not_removed.join(", ")));
    }
    parts.join("; ")
}

/// Refuses when the asset `target` (the `kind` named `name` of the Context `context`) was retired.
fn refuse_retired(
    p: &Project,
    target: Target,
    context: &str,
    kind: AssetKind,
    name: &str,
) -> Result<(), AppError> {
    if p.retired_of(target).is_some() {
        return Err(AppError::AssetRetired {
            context: context.to_string(),
            kind: kind.word(),
            name: name.to_string(),
        });
    }
    Ok(())
}

/// The id of the Context named `name`.
fn context_id(p: &Project, name: &str) -> Result<ContextId, AppError> {
    let n = ContextName::try_from(name).map_err(input)?;
    p.context_by_name(&n)
        .map(|c| c.id())
        .ok_or_else(|| AppError::UnknownContext(name.to_string()))
}

/// The ids of the Contexts named in `names`.
fn context_ids(p: &Project, names: &[String]) -> Result<BTreeSet<ContextId>, AppError> {
    names.iter().map(|n| context_id(p, n)).collect()
}

/// The name of the Case with this id.
fn case_name(p: &Project, id: CaseId) -> String {
    p.contexts()
        .flat_map(|c| c.harnesses())
        .find_map(|h| h.case(id))
        .map(|c| c.name().to_string())
        .unwrap_or_else(|| id.to_string())
}
