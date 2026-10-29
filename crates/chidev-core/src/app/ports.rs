//! What the use cases need from outside the domain: fresh ids, the time, the names a person gives the assets, and a
//! store that keeps what one use case changed whole or not at all.

pub mod external;
pub mod items;
pub mod measure;

use std::fmt;
use std::future::Future;

use uuid::Uuid;

use super::commit::{Gated, Trigger};

use crate::domain::check::{Executor, Level, Moment};
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::crux::Crux;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{
    ChangeRequestRepository, CruxRepository, PinRepository, ProjectRepository, StoreError,
};
use crate::domain::vo::correction::{
    Correctable, Correction, CorrectionError, SoughtText, check_places, masked,
};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::id::{ContextId, InterfaceId, PinId};
use crate::domain::vo::timestamp::Timestamp;

/// Gives a fresh id each time. The domain makes no ids; the use cases take them from here.
pub trait IdSource {
    /// An id not given before.
    fn next_uuid(&mut self) -> Uuid;
}

/// Tells the time. The domain reads no clock; a use case that dates a record (a decision) takes the time from here
/// and passes it in as a value, and a test fixes it.
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// The time now.
    fn now(&self) -> Timestamp;
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

/// The record of one check run by one use case: which check, at which moment, over the store as it stood, and how many
/// findings of each level it gave. Kept apart from the Project (append-only), so a run can be told apart later
/// from the model it read: the store's revision names the model. There is no duration yet; timing needs a clock
/// port, which the use cases do not have.
///
/// The counts are those of the whole run over the state the use case would keep, however much of it the use case
/// went on to report: a put reports only what it introduced, and its record still counts everything the check
/// found in the model after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The check.
    pub check: RuleId,
    /// The moment it ran at.
    pub moment: Moment,
    /// The use case that ran it, whose entry made the record; none for a row made before the use cases were named.
    pub trigger: Option<Trigger>,
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
    /// The corrections made of its texts, in the order they were made; empty when the run is recorded.
    pub corrections: Vec<Correction<InvocationField>>,
}

/// The identity of a row of the record of runs: the order the rows were kept in. A correction names a row by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InvocationSeq(pub u64);

/// A place of text in a record of a check's run, for a correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InvocationField {
    /// The resolved executable path of an external check.
    Command,
    /// The bounded stderr of an external check.
    Stderr,
    /// The agent that judged a judged rule.
    Judge,
}

impl InvocationField {
    /// The words for the field, which a message and an answer name it by.
    pub fn word(&self) -> String {
        match self {
            InvocationField::Command => "external.command".to_string(),
            InvocationField::Stderr => "external.stderr".to_string(),
            InvocationField::Judge => "judge".to_string(),
        }
    }
}

impl fmt::Display for InvocationField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.word())
    }
}

/// The record of runs is defined here, in the ports (ADR 0002), so its correction is written here, with the
/// domain's [`Correction`] and [`SoughtText`].
impl Correctable for Invocation {
    type Field = InvocationField;

    fn texts(&self) -> Vec<(InvocationField, &str)> {
        let mut out = Vec::new();
        if let Some(external) = &self.external {
            if let Some(command) = &external.command {
                out.push((InvocationField::Command, command.as_str()));
            }
            if !external.stderr.is_empty() {
                out.push((InvocationField::Stderr, external.stderr.as_str()));
            }
        }
        if let Some(judge) = &self.judge {
            out.push((InvocationField::Judge, judge.as_str()));
        }
        out
    }

    fn correct(
        &mut self,
        sought: &SoughtText,
        correction: Correction<InvocationField>,
        _taken: &dyn Fn(&InvocationField, &str) -> bool,
    ) -> Result<(), CorrectionError> {
        check_places(self, sought, correction.fields())?;
        let mut next = self.clone();
        for field in correction.fields() {
            let word = field.word();
            let no_such = || CorrectionError::NoSuchField(word.clone());
            match field {
                InvocationField::Command => {
                    let held = next
                        .external
                        .as_mut()
                        .and_then(|e| e.command.as_mut())
                        .ok_or_else(no_such)?;
                    *held = masked(sought, &word, held)?;
                }
                InvocationField::Stderr => {
                    let held = &mut next.external.as_mut().ok_or_else(no_such)?.stderr;
                    *held = masked(sought, &word, held)?;
                }
                InvocationField::Judge => {
                    let held = next.judge.as_mut().ok_or_else(no_such)?;
                    *held = masked(sought, &word, held)?;
                }
            }
        }
        next.corrections.push(correction);
        *self = next;
        Ok(())
    }
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

/// The code a Pin accepted, kept whole beside the Pin (which keeps only its digest): a measure runs this text and
/// not the working tree's, so what it measures is what was pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinCode {
    /// The Pin the code belongs to.
    pub pin: PinId,
    /// The Interface it is the code of; its code replaces the earlier one.
    pub interface: InterfaceId,
    /// The code.
    pub code: String,
}

/// What one use case changed, to be kept at once.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    /// The Project, as it is after the use case.
    pub project: Option<Project>,
    /// Pins made (each replaces the Pin of its Interface).
    pub pins: Vec<Pin>,
    /// The code of the Pins made, each replacing the code of its Interface.
    pub pin_codes: Vec<PinCode>,
    /// Measurements made, appended: the newest of a kind and Interface is the current one.
    pub measurements: Vec<Measurement>,
    /// Change requests opened or changed.
    pub change_requests: Vec<ChangeRequest>,
    /// Cruxes raised or decided.
    pub cruxes: Vec<Crux>,
    /// Asset names given.
    pub names: Vec<AssetName>,
    /// The checks the use case ran, appended to the record of runs.
    pub invocations: Vec<Invocation>,
    /// Reports of judged rules (each replaces the report of its rule).
    pub judged: Vec<JudgedReport>,
    /// Latest results of external checks, kept with the candidate model.
    pub external: Vec<external::ExternalReport>,
    /// Measurements with a text corrected: each replaces the run of its id, which keeps its place among the runs.
    /// Only a correction of records makes one.
    pub corrected_measurements: Vec<Measurement>,
    /// Rows of the record of runs with a text corrected: each replaces the texts and the corrections of its row.
    /// Only a correction makes one.
    pub corrected_invocations: Vec<(InvocationSeq, Invocation)>,
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
    ProjectRepository
    + PinRepository
    + ChangeRequestRepository
    + CruxRepository
    + AssetNames
    + JudgedRepository
{
    /// Latest persisted execution of every external rule.
    fn external_reports(
        &self,
    ) -> impl Future<Output = Result<Vec<external::ExternalReport>, StoreError>> + Send;

    /// The newest measurement of each kind and Interface that has one, in the order they were made.
    fn latest_measurements(
        &self,
    ) -> impl Future<Output = Result<Vec<Measurement>, StoreError>> + Send;

    /// Every measurement, in the order they were made: the newest of a kind and subject and the runs before it. A
    /// correction reads them all, because a text can stay in a run that is not the latest.
    fn measurements(&self) -> impl Future<Output = Result<Vec<Measurement>, StoreError>> + Send;

    /// Every row of the record of check runs, in the order of its `seq`, with the identity a correction names it by.
    fn invocations(
        &self,
    ) -> impl Future<Output = Result<Vec<(InvocationSeq, Invocation)>, StoreError>> + Send;

    /// Writes the file again without what the store freed. A replaced or deleted row leaves its old bytes in the
    /// free pages and the slack of the pages of the file (and in a write-ahead log); this removes them, so a text
    /// that a correction replaced is not left beside the mark. It runs outside any commit.
    ///
    /// What it cannot reach is what the file system holds: the blocks of a rollback journal that is deleted when a
    /// commit ends, and the pages cut from the end of the file, go back to the file system as they were.
    fn clear_freed(&self) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The code the Pin `pin` accepted; none for a Pin made before the code was kept, or for a Pin that has been
    /// replaced.
    fn pin_code(
        &self,
        pin: PinId,
    ) -> impl Future<Output = Result<Option<String>, StoreError>> + Send;

    /// Where the store stands now; read it before the use case reads anything else.
    fn revision(&self) -> impl Future<Output = Result<StoreRevision, StoreError>> + Send;

    /// The revision of the store at which the model last changed: the Project, the change requests, the Cruxes or the names
    /// (what an agent is shown). A commit that only records a run or a judged report leaves it, so a report made
    /// on a view read before such a commit is not taken for one made on an older model.
    fn model_revision(&self) -> impl Future<Output = Result<StoreRevision, StoreError>> + Send;

    /// Keeps what `gated` changed whole, if the store still stands at `read_at`; otherwise keeps nothing and
    /// answers [`StoreError::Conflict`]. A [`Gated`] is made only by an entry of [`App`](super::App), so nothing is
    /// kept that did not go through the checks its use case is held to.
    fn commit(
        &self,
        read_at: StoreRevision,
        gated: Gated,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}
