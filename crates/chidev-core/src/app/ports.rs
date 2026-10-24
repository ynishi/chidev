//! What the use cases need from outside the domain: fresh ids, the names a person gives the assets, and a store
//! that keeps what one use case changed whole or not at all.

pub mod external;

use std::future::Future;

use uuid::Uuid;

use crate::domain::check::{Executor, Hook, Level};
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{
    ChangeRequestRepository, PinRepository, ProjectRepository, StoreError,
};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::id::ContextId;

/// Gives a fresh id each time. The domain makes no ids; the use cases take them from here.
pub trait IdSource {
    /// An id not given before.
    fn next_uuid(&mut self) -> Uuid;
}

/// The kind of an asset, for its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    /// A Contract.
    Contract,
    /// An Interface.
    Interface,
    /// A Harness.
    Harness,
}

impl AssetKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            AssetKind::Contract => "contract",
            AssetKind::Interface => "interface",
            AssetKind::Harness => "harness",
        }
    }
}

/// The name a person gives an asset: one per asset within its Context and kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetName {
    /// The Context the asset is in.
    pub context: ContextId,
    /// What kind of asset it is.
    pub kind: AssetKind,
    /// The name.
    pub name: String,
    /// The asset's id.
    pub id: Uuid,
}

/// The names a person addresses the assets by. A name is not part of the model (an asset is its id); it is how
/// the commands find the id, so it is kept beside the model.
pub trait AssetNames {
    /// The id of the asset of `kind` named `name` in `context`.
    fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
    ) -> impl Future<Output = Result<Option<Uuid>, StoreError>> + Send;

    /// The name of the asset with this id.
    fn asset_name(
        &self,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;
}

/// How a check run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It found nothing.
    Ok,
    /// It found something (at any level).
    Found,
    /// The external program failed to execute or answer the protocol.
    Failed,
}

impl Outcome {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Found => "found",
            Outcome::Failed => "failed",
        }
    }
}

impl Outcome {
    /// The outcome a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<Outcome> {
        [Outcome::Ok, Outcome::Found, Outcome::Failed]
            .into_iter()
            .find(|o| o.word() == word)
    }
}

/// The record of one check run by one use case: which check, at which hook, over the store as it stood, and how many
/// findings of each level it gave. Kept apart from the Project (append-only), so a run can be told apart later
/// from the model it read: the store's revision names the model. There is no duration yet; timing needs a clock
/// port, which the use cases do not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The check.
    pub check: RuleId,
    /// The hook it ran at.
    pub hook: Hook,
    /// The revision of the store the use case read.
    pub read_at: StoreRevision,
    /// Findings at `note`.
    pub notes: u32,
    /// Findings at `warning`.
    pub warnings: u32,
    /// Findings at `error`.
    pub errors: u32,
    /// How it ended.
    pub outcome: Outcome,
    /// Who ran it: the use cases run builtin checks; the report of a judged rule is recorded as a run of it.
    pub executor: Executor,
    /// The agent that judged it, for a judged rule; none otherwise.
    pub judge: Option<String>,
    /// Evidence of an external process; absent for builtin and judged checks.
    pub external: Option<external::ExecutionEvidence>,
    /// Definition digest echoed by the judge; none for other executors and for old database rows.
    pub rule_digest: Option<Digest>,
}

/// One finding an agent reported for a judged rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedFinding {
    /// What it is about; none for the Project as a whole.
    pub subject: Option<Subject>,
    /// What is wrong, in the agent's words.
    pub message: String,
    /// The level the agent gave, never above the rule's level; none: the rule's level.
    pub level: Option<Level>,
    /// The fingerprint of the subject's entry in the view of the model when it was reported (of the whole model
    /// for a finding about the Project): the finding is stale once the subject's entry is no longer this.
    pub stamp: Digest,
}

/// What an agent reported for a judged rule. A rule is judged as a whole, so a report replaces every earlier
/// finding of the rule, and a report with no finding is kept too: it says the rule was judged and found nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgedReport {
    /// The rule.
    pub rule: RuleId,
    /// The definition judged. Always set by a report; none only for an old database row, which reads as stale.
    pub rule_digest: Option<Digest>,
    /// The agent that judged it.
    pub judge: String,
    /// The revision of the store the view the agent judged from was read at.
    pub read_at: StoreRevision,
    /// The revision of the store the report was made on.
    pub reported_at: StoreRevision,
    /// Whether the model changed between `read_at` and the report: the agent judged an older model than the one
    /// the findings are stamped against, so they cannot be known to hold.
    pub on_older_revision: bool,
    /// The fingerprint of the part of the model the rule judges (all the Clauses for a rule about Clauses),
    /// when the report was made: the rule is stale once that part has changed.
    pub scope: Digest,
    /// The findings; none: it found nothing.
    pub findings: Vec<JudgedFinding>,
}

/// What one use case changed, to be kept at once.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    /// The Project, as it is after the use case.
    pub project: Option<Project>,
    /// Pins made (each replaces the Pin of its Interface).
    pub pins: Vec<Pin>,
    /// Change requests opened or changed.
    pub change_requests: Vec<ChangeRequest>,
    /// Asset names given.
    pub names: Vec<AssetName>,
    /// The checks the use case ran, appended to the record of runs.
    pub invocations: Vec<Invocation>,
    /// Reports of judged rules (each replaces the report of its rule).
    pub judged: Vec<JudgedReport>,
    /// Latest results of external checks, kept with the candidate model.
    pub external: Vec<external::ExternalReport>,
}

/// Where the store stood when a use case began to read. A commit given an older one is refused
/// ([`StoreError::Conflict`]): someone else wrote in between, and the use case's reads may be out of date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreRevision(pub u64);

/// Finds what agents reported for the judged rules.
pub trait JudgedRepository {
    /// The latest report of every judged rule that has one, in the order of the rules' ids.
    fn judged_reports(&self) -> impl Future<Output = Result<Vec<JudgedReport>, StoreError>> + Send;
}

/// A store: the repositories and the names to read from, and one commit per use case to write. A use case reads
/// what it needs (each read on its own; no transaction is held across them), decides, and commits every change at
/// once; the store keeps them all or none.
pub trait Store:
    ProjectRepository + PinRepository + ChangeRequestRepository + AssetNames + JudgedRepository
{
    /// Latest persisted execution of every external rule.
    fn external_reports(
        &self,
    ) -> impl Future<Output = Result<Vec<external::ExternalReport>, StoreError>> + Send;

    /// Where the store stands now; read it before the use case reads anything else.
    fn revision(&self) -> impl Future<Output = Result<StoreRevision, StoreError>> + Send;

    /// The revision of the store at which the model last changed: the Project, the change requests or the names
    /// (what an agent is shown). A commit that only records a run or a judged report leaves it, so a report made
    /// on a view read before such a commit is not taken for one made on an older model.
    fn model_revision(&self) -> impl Future<Output = Result<StoreRevision, StoreError>> + Send;

    /// Keeps `changes` whole, if the store still stands at `read_at`; otherwise keeps nothing and answers
    /// [`StoreError::Conflict`].
    fn commit(
        &self,
        read_at: StoreRevision,
        changes: Changes,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
