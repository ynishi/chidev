//! The change request: a request that the definitions change (or a confirmation that they need not). It names the
//! targets it asks to be revised or retired, and the targets to keep. A question that only a decision maker can
//! answer is a [`Crux`](super::crux::Crux), a record of its own that is raised in a change request and stops it; the
//! change request does not hold it. The option a decision maker chooses may name more targets, and the change
//! request records those it is about when it is decided; the ones outside it are the answer's `beyond`.
//!
//! A request ends by an explicit resolution (how it ended, why, who, when), and can be opened again; the history of
//! both is only appended to. Neither the targets nor the revisions of the definitions decide whether it may end.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use thiserror::Error;

use crate::domain::vo::assumption::{Assumption, Evidence, KnowledgeRef, RepoPath};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::correction::{
    Correctable, Correction, CorrectionError, SoughtText, check_places, masked, masked_distinct,
};
use crate::domain::vo::crux::{CruxKind, CruxOption};
use crate::domain::vo::finding::FindingRef;
use crate::domain::vo::footprint::Footprint;
use crate::domain::vo::id::{AnswerId, ChangeRequestId, CruxId, CruxOptionId};
use crate::domain::vo::intent::{Intent, Issuer};
use crate::domain::vo::policy::PolicyLineId;
use crate::domain::vo::prose::Prose;
use crate::domain::vo::resolution::{Reopening, Resolution, Transition};
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::{ExpectKind, Expectation, Target};
use crate::domain::vo::title::Title;

/// Why a change request or a Crux cannot be opened, raised, decided, resolved or reopened. Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChangeRequestError {
    /// A Crux has no option to choose.
    #[error("a crux needs at least one option")]
    NoOption,
    /// Two options are the same choice.
    #[error("the option `{0}` is given twice")]
    DuplicateOption(Choice),
    /// Two options have the same id.
    #[error("the option id {0} is given twice")]
    DuplicateOptionId(CruxOptionId),
    /// A recommendation or a decision names an option that the Crux does not have.
    #[error("the option {0} is not one of the options of the crux")]
    NotAnOption(CruxOptionId),
    /// A question of a kind that is about words (`model`) cannot be left unspecified.
    #[error("the crux {crux}, a {} question, cannot be left unspecified", kind.word())]
    UnspecifiedNotAllowed {
        /// The Crux.
        crux: CruxId,
        /// Its kind.
        kind: CruxKind,
    },
    /// The change request is resolved; it takes no more decisions and no more assumptions.
    #[error("the change request is resolved")]
    AlreadyResolved,
    /// A Crux raised in the change request has no decision yet (the first such, the one named), so the change request
    /// cannot be resolved, whatever the resolution.
    #[error("the crux {0}, raised in the change request, has no decision yet")]
    Undecided(CruxId),
    /// The Crux is answered already. An answer is an external input that cannot be reproduced, so it is never
    /// rewritten: a different decision is a new Crux that supersedes this one.
    #[error(
        "the crux {0} is answered already; a different decision is a new crux that supersedes it"
    )]
    AlreadyAnswered(CruxId),
    /// A Crux is to supersede a Crux that does not exist.
    #[error("the crux {0} that the crux supersedes does not exist")]
    SupersedesUnknown(CruxId),
    /// A Crux is to supersede a Crux that has no decision to replace: it is open, or its decision is a withdrawal.
    #[error("the crux {0} that the crux supersedes has no decision to replace")]
    SupersedesUnanswered(CruxId),
    /// A decision rests on a line the Project's Policy does not have.
    #[error("the policy has no line {0}")]
    UnknownPolicyLine(PolicyLineId),
    /// A decision cites the Policy at a revision other than the one it has now.
    #[error("the decision cites the policy at {recorded}, but it is at {now}")]
    PolicyRevisionMismatch {
        /// The revision the decision cites.
        recorded: Revision,
        /// The revision the Policy has.
        now: Revision,
    },
    /// The change request is not resolved; there is nothing to reopen.
    #[error("the change request is not resolved")]
    NotResolved,
    /// A resolution, or the `beyond` of an answer, points at a change request that does not exist.
    #[error("the change request {0} that a resolution or an answer points at does not exist")]
    ReferenceUnknown(ChangeRequestId),
    /// A resolution points at the change request it resolves.
    #[error("a change request cannot be resolved by pointing at itself")]
    ReferencesItself,
    /// A stored history does not alternate between a resolution and a reopening, starting with a resolution.
    #[error("the history of resolutions and reopenings is out of order")]
    HistoryOutOfOrder,
    /// Change requests that are a part of this one are not resolved.
    #[error("{} part(s) of the change request are not resolved", .children.len())]
    ChildrenNotResolved {
        /// The change requests whose parent it is and that are not resolved.
        children: Vec<ChangeRequestId>,
    },
    /// A stored expectation does not name the answer of a Crux raised in the change request, or that answer's
    /// decision does not ask it.
    #[error("{0:?} is not an expectation the decision it names asks")]
    UnaskedExpectation(Target),
    /// An option asks of a target the other of being revised or retired and kept, where the change request (or a
    /// decision that stands) asks it already.
    #[error("the option `{option}` asks {target:?} the other of being revised or retired and kept")]
    ExpectationConflict {
        /// The option that asks it.
        option: Choice,
        /// The target.
        target: Target,
    },
    /// A stored Crux was raised in a change request that does not exist.
    #[error("the change request {0} that a crux was raised in does not exist")]
    RaisedInUnknown(ChangeRequestId),
    /// The option chosen names targets the change request does not frame, and the answer says nothing of where they
    /// go (`beyond`).
    #[error(
        "the option `{option}` names {} target(s) outside the change request, and the answer does not say where they go",
        targets.len()
    )]
    AnswerOutside {
        /// The Crux being decided.
        crux: CruxId,
        /// The option chosen.
        option: Choice,
        /// The targets it names that are outside the frame of the change request, in order.
        targets: Vec<Target>,
    },
    /// A target outside the change request is in the frame of none of the change requests the answer names.
    #[error(
        "{target:?} is outside the change request and in the frame of none of those the answer names"
    )]
    BeyondUnframed {
        /// The target.
        target: Target,
    },
    /// The answer names the change request it was given in as the place what lies outside it goes.
    #[error("the answer names the change request it was given in")]
    BeyondItself,
    /// The answer names a change request that is resolved, which takes no more work.
    #[error("the change request {0} that the answer names is resolved")]
    BeyondResolved(ChangeRequestId),
    /// A withdrawal asks nothing and adds nothing, so it has no outside to speak of.
    #[error("a withdrawal says nothing of what lies outside the change request")]
    BeyondOnWithdrawal,
    /// A Crux is to supersede a decision that another Crux replaces already.
    #[error("the decision of the crux {crux} is replaced by the crux {by} already")]
    SupersededAlready {
        /// The Crux whose decision is to be replaced.
        crux: CruxId,
        /// The Crux that replaces it.
        by: CruxId,
    },
}

/// Where a change request stands. It depends on the Cruxes raised in it, which it does not hold, so it is read
/// together with them (`service::change_request::state_of`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRequestState {
    /// Not resolved, and either it has no Crux or one of its Cruxes is open: a decision maker may be awaited.
    Open,
    /// It has Cruxes, every one has a decision (a withdrawal is one), and it is not resolved.
    Decided,
    /// A resolution closed the record (until it is reopened).
    Resolved,
}

impl ChangeRequestState {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            ChangeRequestState::Open => "open",
            ChangeRequestState::Decided => "decided",
            ChangeRequestState::Resolved => "resolved",
        }
    }
}

/// What a change request is opened with, before its rules are checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequestDraft {
    /// What is asked, in a line.
    pub title: Title,
    /// Why: the situation that makes it needed.
    pub situation: Option<Prose>,
    /// Whether it means to change the behaviour the specification promises. A record: no rule of resolving reads it.
    pub intent: Intent,
    /// The broader change request it is a part of: a change request is often split.
    pub parent: Option<ChangeRequestId>,
    /// Who raised it.
    pub issuer: Option<Issuer>,
    /// What it asks of targets (to revise or retire, or to keep), one entry for each, with the revision each target
    /// has now. A parent need not have any.
    pub expects: BTreeMap<Target, Expectation>,
    /// The interpretations settled from the repository and general knowledge while raising it.
    pub assumptions: Vec<Assumption>,
    /// The findings of the checks it deals with: while it is unresolved they are addressed, once resolved decided.
    pub addresses: Vec<FindingRef>,
}

/// `addresses` without a finding twice, in the order given.
fn distinct(addresses: Vec<FindingRef>) -> Vec<FindingRef> {
    let mut out: Vec<FindingRef> = Vec::with_capacity(addresses.len());
    for a in addresses {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// What a decision asked of a target, once it was recorded in the change request: the target, the kind, the revision
/// the target had at the decision, and the answer of the decision that asked for it. One is kept per decision and
/// target, so that what a decision asked can be taken back when a Crux supersedes that decision without losing what
/// another decision still asks of the same target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecidedExpectation {
    /// The target asked.
    pub target: Target,
    /// What is asked.
    pub kind: ExpectKind,
    /// The revision the target had when the decision was taken.
    pub recorded: Revision,
    /// The answer of the decision that asked for it.
    pub answer: AnswerId,
}

impl DecidedExpectation {
    /// The expectation, without the answer that asked it.
    pub fn expectation(&self) -> Expectation {
        Expectation {
            kind: self.kind,
            recorded: self.recorded,
        }
    }
}

/// A request that the definitions change, or a confirmation that they need not: whatever is there now does not
/// satisfy. It names the targets it asks to be revised or retired and the targets to keep (each with the revision it
/// had, so a keep that moved is seen). It may be part of a broader request, which need not name any target. A Crux
/// raised in it stops it until a decision maker answers; the option chosen may name more targets, recorded here when
/// it is decided ([`ChangeRequest::record_decided_expects`]). It records the Assumptions made while raising it,
/// which only grow.
///
/// It is closed by a [`Resolution`] a decision maker gives (how it ended, why, who, when), and can be opened again
/// by a [`Reopening`]; the history keeps every step. Only explicit states refuse a resolution: a Crux raised in it
/// without a decision, a part not resolved, the request resolved already. The revisions of the targets and the work
/// recorded for it are read by checks, not by the resolution. Its intent (whether it means to change the behaviour)
/// is a record too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    id: ChangeRequestId,
    title: Title,
    situation: Option<Prose>,
    intent: Intent,
    parent: Option<ChangeRequestId>,
    issuer: Option<Issuer>,
    expects: BTreeMap<Target, Expectation>,
    /// What the options chosen expect: one entry per decision and target, with the revision the target had when it
    /// was decided, in the order they were recorded. Two decisions may ask the same target.
    decided_expects: Vec<DecidedExpectation>,
    assumptions: Vec<Assumption>,
    addresses: Vec<FindingRef>,
    /// What the change sets applied for it moved, in the order they were kept.
    work: Vec<Footprint>,
    /// Resolutions, reopenings and corrections, in the order they happened; resolved when the last resolution or
    /// reopening is a resolution.
    history: Vec<Transition>,
}

impl ChangeRequest {
    /// An open change request. It cannot be its own parent.
    pub fn open(id: ChangeRequestId, draft: ChangeRequestDraft) -> ChangeRequest {
        ChangeRequest {
            id,
            title: draft.title,
            situation: draft.situation,
            intent: draft.intent,
            parent: draft.parent.filter(|p| *p != id),
            issuer: draft.issuer,
            expects: draft.expects,
            decided_expects: Vec::new(),
            assumptions: draft.assumptions,
            addresses: distinct(draft.addresses),
            work: Vec::new(),
            history: Vec::new(),
        }
    }

    /// A change request as a store kept it: the expectations the decisions asked, with the revisions recorded, and
    /// its history. The resolutions and reopenings in a history alternate, starting with a resolution; a correction
    /// may stand anywhere among them. Whether
    /// the expectations recorded are ones a decision asks, and whether a request resolved as done has its Cruxes
    /// decided, are read with the Cruxes, which are other records
    /// (`service::change_request::check_requests_and_cruxes`). Whether the parts of a resolved one are resolved is
    /// not asked either.
    pub(crate) fn restore(
        id: ChangeRequestId,
        draft: ChangeRequestDraft,
        decided_expects: Vec<DecidedExpectation>,
        work: Vec<Footprint>,
        history: Vec<Transition>,
    ) -> Result<ChangeRequest, ChangeRequestError> {
        let in_order = history
            .iter()
            .filter(|step| step.is_state())
            .enumerate()
            .all(|(k, step)| matches!(step, Transition::Resolved(_)) == (k % 2 == 0));
        if !in_order {
            return Err(ChangeRequestError::HistoryOutOfOrder);
        }
        Ok(ChangeRequest {
            id,
            title: draft.title,
            situation: draft.situation,
            intent: draft.intent,
            parent: draft.parent.filter(|p| *p != id),
            issuer: draft.issuer,
            expects: draft.expects,
            decided_expects,
            assumptions: draft.assumptions,
            addresses: distinct(draft.addresses),
            work,
            history,
        })
    }

    /// Its id.
    pub fn id(&self) -> ChangeRequestId {
        self.id
    }

    /// What is asked.
    pub fn title(&self) -> &Title {
        &self.title
    }

    /// Why it is needed.
    pub fn situation(&self) -> Option<&Prose> {
        self.situation.as_ref()
    }

    /// Whether it means to change the behaviour. A record: no rule of resolving reads it.
    pub fn intent(&self) -> Intent {
        self.intent
    }

    /// The broader change request it is a part of.
    pub fn parent(&self) -> Option<ChangeRequestId> {
        self.parent
    }

    /// Who raised it.
    pub fn issuer(&self) -> Option<&Issuer> {
        self.issuer.as_ref()
    }

    /// What it asks of targets (to revise or retire, or to keep), with the revision each had when it was opened.
    /// They hold whatever its Cruxes decide.
    pub fn expects(&self) -> &BTreeMap<Target, Expectation> {
        &self.expects
    }

    /// What the options chosen for the Cruxes raised in it expect of targets: one entry for each decision and target,
    /// with the revision the target had when the Crux was decided and the answer that asked for it. Empty until
    /// decided, and when the decisions expect nothing more. A target several decisions ask has an entry for each.
    pub fn decided_expects(&self) -> &[DecidedExpectation] {
        &self.decided_expects
    }

    /// Every expectation that holds now, one for each target: its own, then those the decisions asked. A target is
    /// in one of them, or in both with the same kind: a decision that would add it to the other with another kind is
    /// refused. A target several decisions ask is given once, with the earliest revision recorded for it, so that a
    /// keep is judged against the oldest of them.
    pub fn expectations(&self) -> impl Iterator<Item = (Target, Expectation)> + '_ {
        let own = self.expects.iter().map(|(t, e)| (*t, *e));
        let mut decided: BTreeMap<Target, Expectation> = BTreeMap::new();
        for d in self
            .decided_expects
            .iter()
            .filter(|d| !self.expects.contains_key(&d.target))
        {
            decided
                .entry(d.target)
                .and_modify(|e| e.recorded = e.recorded.min(d.recorded))
                .or_insert_with(|| d.expectation());
        }
        own.chain(decided)
    }

    /// The kind asked of `target` now, by the request or by a decision, leaving out what the decision with the answer
    /// `leaving_out` asked: that decision is about to be superseded, so what it asked no longer stands.
    pub fn kind_asked(&self, target: &Target, leaving_out: Option<AnswerId>) -> Option<ExpectKind> {
        self.expects.get(target).map(|e| e.kind).or_else(|| {
            self.decided_expects
                .iter()
                .find(|d| d.target == *target && Some(d.answer) != leaving_out)
                .map(|d| d.kind)
        })
    }

    /// The interpretations it records, in the order they were made.
    pub fn assumptions(&self) -> &[Assumption] {
        &self.assumptions
    }

    /// The findings it deals with: a finding listed here is addressed while the change request is unresolved and
    /// decided once it is resolved (see `Finding::state_in`).
    pub fn addresses(&self) -> &[FindingRef] {
        &self.addresses
    }

    /// What the change sets applied for it moved, in the order they were kept. Whether its targets were revised or
    /// retired is read from these (and those of its parts), not from any move of the target.
    pub fn work(&self) -> &[Footprint] {
        &self.work
    }

    /// Records what a change set applied for it moved. A resolved change request is a closed record and takes no
    /// more work (until it is reopened). A change set that moved nothing (the same puts applied again) is no work:
    /// its footprint is not kept, so the work holds only footprints that moved something. A footprint of new names
    /// alone is not empty: a Clause or an Item given a new name is a move.
    pub fn record_work(&mut self, footprint: Footprint) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        if !footprint.moves().is_empty() {
            self.work.push(footprint);
        }
        Ok(())
    }

    /// The last resolution or reopening: a correction between them changes no state, so it is passed over.
    fn last_state(&self) -> Option<&Transition> {
        self.history.iter().rev().find(|step| step.is_state())
    }

    /// Whether the last resolution or reopening in its history is a resolution.
    pub fn is_resolved(&self) -> bool {
        matches!(self.last_state(), Some(Transition::Resolved(_)))
    }

    /// The resolution it stands under, when it is resolved.
    pub fn resolution(&self) -> Option<&Resolution> {
        match self.last_state() {
            Some(Transition::Resolved(r)) => Some(r),
            _ => None,
        }
    }

    /// Every resolution, reopening and correction, in the order they happened. It only grows: a reopening does not
    /// take the resolution it follows out of it, and a correction keeps the fields it replaced a text in and no text.
    pub fn history(&self) -> &[Transition] {
        &self.history
    }

    /// Whether `option` may be offered or chosen in this request: it asks of a target what the request and the
    /// decisions that stand ask of it, or nothing of it. The first target it asks with the other kind (revised or
    /// retired, where it is kept, or the reverse) is refused with the option and the target. `leaving_out` is the
    /// answer of a decision that is about to be superseded, so that what it asked no longer stands (see
    /// [`ChangeRequest::kind_asked`]). This is the one statement of the rule: raising a Crux reads it for the options
    /// it offers, and deciding one reads it for the option chosen.
    pub fn check_option(
        &self,
        option: &CruxOption,
        leaving_out: Option<AnswerId>,
    ) -> Result<(), ChangeRequestError> {
        for (target, kind) in option.expects() {
            if self
                .kind_asked(target, leaving_out)
                .is_some_and(|held| held != *kind)
            {
                return Err(ChangeRequestError::ExpectationConflict {
                    option: option.choice().clone(),
                    target: *target,
                });
            }
        }
        Ok(())
    }

    /// Records what the option chosen for a Crux raised in it expects, for the decision with the answer `answer`: each
    /// target the option asks is recorded with its kind and the revision in `now`, the one it has as the decision is
    /// taken (a target missing from `now` is gone: it is not recorded). The entry is the decision's own: a target
    /// another decision asks with the same kind has an entry of each, each with its revision, so that taking one
    /// decision back leaves what the other asks. A target this request asks itself with the same kind is not
    /// recorded, and one this decision recorded already is not recorded twice. A target that is asked with another
    /// kind, by the request or by another decision that stands, is refused ([`ChangeRequest::check_option`]), and
    /// nothing is recorded. A resolved request is a closed record and takes no more (until it is reopened).
    ///
    /// `leave_out` is the targets this decision does not ask of this request: they are not recorded, so that a
    /// decision never widens what the request is about. The conflict check still reads the whole option.
    pub fn record_decided_expects(
        &mut self,
        option: &CruxOption,
        now: &BTreeMap<Target, Revision>,
        answer: AnswerId,
        leave_out: &BTreeSet<Target>,
    ) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        self.check_option(option, None)?;
        let mut add = Vec::new();
        for (target, kind) in option.expects() {
            if leave_out.contains(target)
                || self.expects.contains_key(target)
                || self
                    .decided_expects
                    .iter()
                    .any(|d| d.target == *target && d.answer == answer)
            {
                continue;
            }
            if let Some(recorded) = now.get(target) {
                add.push(DecidedExpectation {
                    target: *target,
                    kind: *kind,
                    recorded: *recorded,
                    answer,
                });
            }
        }
        self.decided_expects.extend(add);
        Ok(())
    }

    /// Takes back what the decision with the answer `answer` asked: its decided expectations are removed, and the
    /// others stay, even those of the same targets. Used when a Crux supersedes that decision; what the request asks
    /// itself is never touched. A resolved request is a closed record and takes none back (until it is reopened).
    pub fn take_back_decided_expects(
        &mut self,
        answer: AnswerId,
    ) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        self.decided_expects.retain(|d| d.answer != answer);
        Ok(())
    }

    /// Appends an Assumption. Assumptions only grow: there is no way to change or take one back, so what was
    /// assumed stays readable. A resolved change request is a closed record and takes none (until it is reopened).
    pub fn assume(&mut self, assumption: Assumption) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        self.assumptions.push(assumption);
        Ok(())
    }

    /// Closes the record with `resolution`; nothing else of the request is read. Whether the Cruxes raised in it have
    /// their decisions, whether its parts are resolved, and whether the request a resolution points at exists, are
    /// read against the other records, so the caller checks them first (`service::change_request::resolve`).
    pub fn resolve(&mut self, resolution: Resolution) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        self.history.push(Transition::Resolved(resolution));
        Ok(())
    }

    /// Opens a resolved request again. The resolution stays in the history; the request takes work, Cruxes, decisions
    /// and assumptions again, and can be resolved anew.
    pub fn reopen(&mut self, reopening: Reopening) -> Result<(), ChangeRequestError> {
        if !self.is_resolved() {
            return Err(ChangeRequestError::NotResolved);
        }
        self.history.push(Transition::Reopened(reopening));
        Ok(())
    }
}

/// A place of text in a change request, for a correction. An index counts from 0 in the list it is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeRequestField {
    /// The title.
    Title,
    /// The situation.
    Situation,
    /// The issuer.
    Issuer,
    /// What an Assumption states.
    AssumptionStatement {
        /// The Assumption, in the order they were made.
        assumption: usize,
    },
    /// Why an Assumption holds.
    AssumptionReason {
        /// The Assumption.
        assumption: usize,
    },
    /// What an Assumption stands on: the path of a repository evidence, or the reference of a knowledge one.
    AssumptionEvidence {
        /// The Assumption.
        assumption: usize,
        /// The evidence, in the order given.
        evidence: usize,
    },
    /// The reason of a resolution or a reopening.
    StepReason {
        /// The step, as an index into the history (corrections included).
        step: usize,
    },
    /// The name of who resolved or reopened it. The kind of the maker (role or person) is kept.
    StepBy {
        /// The step, as an index into the history (corrections included).
        step: usize,
    },
}

impl ChangeRequestField {
    /// The words for the field, which a message and an answer name it by: `title`, `assumption[0].reason`,
    /// `step[2].by`.
    pub fn word(&self) -> String {
        match self {
            ChangeRequestField::Title => "title".to_string(),
            ChangeRequestField::Situation => "situation".to_string(),
            ChangeRequestField::Issuer => "issuer".to_string(),
            ChangeRequestField::AssumptionStatement { assumption } => {
                format!("assumption[{assumption}].statement")
            }
            ChangeRequestField::AssumptionReason { assumption } => {
                format!("assumption[{assumption}].reason")
            }
            ChangeRequestField::AssumptionEvidence {
                assumption,
                evidence,
            } => format!("assumption[{assumption}].evidence[{evidence}]"),
            ChangeRequestField::StepReason { step } => format!("step[{step}].reason"),
            ChangeRequestField::StepBy { step } => format!("step[{step}].by"),
        }
    }
}

impl fmt::Display for ChangeRequestField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.word())
    }
}

impl Correctable for ChangeRequest {
    type Field = ChangeRequestField;

    fn texts(&self) -> Vec<(ChangeRequestField, &str)> {
        let mut out = vec![(ChangeRequestField::Title, self.title.as_str())];
        if let Some(situation) = &self.situation {
            out.push((ChangeRequestField::Situation, situation.as_str()));
        }
        if let Some(issuer) = &self.issuer {
            out.push((ChangeRequestField::Issuer, issuer.as_str()));
        }
        for (a, assumption) in self.assumptions.iter().enumerate() {
            out.push((
                ChangeRequestField::AssumptionStatement { assumption: a },
                assumption.statement().as_str(),
            ));
            out.push((
                ChangeRequestField::AssumptionReason { assumption: a },
                assumption.reason().as_str(),
            ));
            for (e, evidence) in assumption.evidence().iter().enumerate() {
                let text = match evidence {
                    Evidence::Repo { path, .. } => path.as_str(),
                    Evidence::Knowledge { reference } => reference.as_str(),
                };
                out.push((
                    ChangeRequestField::AssumptionEvidence {
                        assumption: a,
                        evidence: e,
                    },
                    text,
                ));
            }
        }
        for (k, step) in self.history.iter().enumerate() {
            match step {
                Transition::Resolved(r) => {
                    out.push((
                        ChangeRequestField::StepReason { step: k },
                        r.reason().as_str(),
                    ));
                    out.push((
                        ChangeRequestField::StepBy { step: k },
                        r.by().name().as_str(),
                    ));
                }
                Transition::Reopened(r) => {
                    out.push((
                        ChangeRequestField::StepReason { step: k },
                        r.reason().as_str(),
                    ));
                    out.push((
                        ChangeRequestField::StepBy { step: k },
                        r.by().name().as_str(),
                    ));
                }
                Transition::Corrected(_) => {}
            }
        }
        out
    }

    fn correct(
        &mut self,
        sought: &SoughtText,
        correction: Correction<ChangeRequestField>,
        taken: &dyn Fn(&ChangeRequestField, &str) -> bool,
    ) -> Result<(), CorrectionError> {
        check_places(self, sought, correction.fields())?;
        let mut next = self.clone();
        for field in correction.fields() {
            next.replace_text(*field, sought, taken)?;
        }
        next.history.push(Transition::Corrected(correction));
        *self = next;
        Ok(())
    }
}

impl ChangeRequest {
    /// Replaces the text sought in one field, and builds the value again with its own constructor. Called on a
    /// copy: an error leaves it half done, and the copy is dropped.
    fn replace_text(
        &mut self,
        field: ChangeRequestField,
        sought: &SoughtText,
        taken: &dyn Fn(&ChangeRequestField, &str) -> bool,
    ) -> Result<(), CorrectionError> {
        let word = field.word();
        let text = self
            .texts()
            .into_iter()
            .find(|(f, _)| *f == field)
            .map(|(_, t)| t.to_string())
            .ok_or_else(|| CorrectionError::NoSuchField(word.clone()))?;
        let new = if field == ChangeRequestField::Title {
            masked_distinct(sought, &word, &text, |candidate| taken(&field, candidate))?
        } else {
            masked(sought, &word, &text)?
        };
        let invalid = || CorrectionError::Invalid(word.clone());
        let prose = |s: &str| Prose::try_from(s).map_err(|_| invalid());
        let no_such = || CorrectionError::NoSuchField(word.clone());
        match field {
            ChangeRequestField::Title => {
                self.title = Title::try_from(new.as_str()).map_err(|_| invalid())?;
            }
            ChangeRequestField::Situation => self.situation = Some(prose(&new)?),
            ChangeRequestField::Issuer => {
                self.issuer = Some(Issuer::try_from(new.as_str()).map_err(|_| invalid())?);
            }
            ChangeRequestField::AssumptionStatement { assumption } => {
                let a = self.assumptions.get_mut(assumption).ok_or_else(no_such)?;
                *a = Assumption::new(prose(&new)?, a.reason().clone(), a.evidence().to_vec());
            }
            ChangeRequestField::AssumptionReason { assumption } => {
                let a = self.assumptions.get_mut(assumption).ok_or_else(no_such)?;
                *a = Assumption::new(a.statement().clone(), prose(&new)?, a.evidence().to_vec());
            }
            ChangeRequestField::AssumptionEvidence {
                assumption,
                evidence,
            } => {
                let a = self.assumptions.get_mut(assumption).ok_or_else(no_such)?;
                let mut all = a.evidence().to_vec();
                let slot = all.get_mut(evidence).ok_or_else(no_such)?;
                *slot = match slot {
                    Evidence::Repo { lines, .. } => Evidence::Repo {
                        path: RepoPath::try_from(new.as_str()).map_err(|_| invalid())?,
                        lines: *lines,
                    },
                    Evidence::Knowledge { .. } => Evidence::Knowledge {
                        reference: KnowledgeRef::try_from(new.as_str()).map_err(|_| invalid())?,
                    },
                };
                *a = Assumption::new(a.statement().clone(), a.reason().clone(), all);
            }
            ChangeRequestField::StepReason { step } => {
                let reason = prose(&new)?;
                match self.history.get_mut(step).ok_or_else(no_such)? {
                    Transition::Resolved(r) => {
                        *r = Resolution::new(r.kind(), reason, r.by().clone(), r.at());
                    }
                    Transition::Reopened(r) => {
                        *r = Reopening::new(reason, r.by().clone(), r.at());
                    }
                    Transition::Corrected(_) => return Err(no_such()),
                }
            }
            ChangeRequestField::StepBy { step } => {
                match self.history.get_mut(step).ok_or_else(no_such)? {
                    Transition::Resolved(r) => {
                        let by = r.by().renamed(&new).map_err(|_| invalid())?;
                        *r = Resolution::new(r.kind(), r.reason().clone(), by, r.at());
                    }
                    Transition::Reopened(r) => {
                        let by = r.by().renamed(&new).map_err(|_| invalid())?;
                        *r = Reopening::new(r.reason().clone(), by, r.at());
                    }
                    Transition::Corrected(_) => return Err(no_such()),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::answer::DecisionMaker;
    use crate::domain::vo::assumption::{Evidence, KnowledgeRef, RepoPath};
    use crate::domain::vo::id::{ClauseId, test_uuid};
    use crate::domain::vo::intent::{Intent, Issuer};
    use crate::domain::vo::resolution::ResolutionKind;
    use crate::domain::vo::timestamp::Timestamp;
    use crate::domain::vo::title::Title;

    fn prose(s: &str) -> Prose {
        Prose::try_from(s).unwrap()
    }

    fn answer(n: u128) -> AnswerId {
        AnswerId::from_uuid(test_uuid(900 + n))
    }

    fn line() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(30)))
    }

    fn detail() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(31)))
    }

    fn change_at_first() -> Expectation {
        Expectation {
            kind: ExpectKind::Target,
            recorded: Revision::first(),
        }
    }

    fn resolution(kind: ResolutionKind) -> Resolution {
        Resolution::new(
            kind,
            prose("settled"),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(9),
        )
    }

    fn done() -> Resolution {
        resolution(ResolutionKind::Done)
    }

    fn reopening() -> Reopening {
        Reopening::new(
            prose("it came back"),
            DecisionMaker::person("bob").unwrap(),
            Timestamp::from_millis(10),
        )
    }

    fn id(n: u128) -> ChangeRequestId {
        ChangeRequestId::from_uuid(test_uuid(n))
    }

    fn draft() -> ChangeRequestDraft {
        ChangeRequestDraft {
            title: Title::try_from("How is a half rounded?").unwrap(),
            situation: Some(prose("The requirement says rounded, not how.")),
            intent: Intent::ChangeBehavior,
            parent: Some(id(9)),
            issuer: Some(Issuer::try_from("designer").unwrap()),
            expects: BTreeMap::from([(line(), change_at_first())]),
            assumptions: Vec::new(),
            addresses: Vec::new(),
        }
    }

    fn cr() -> ChangeRequest {
        ChangeRequest::open(id(1), draft())
    }

    /// An option, named `choice`, that asks every one of `asked`.
    fn option_asking(choice: &str, asked: &[(Target, ExpectKind)]) -> CruxOption {
        CruxOption::new(
            CruxOptionId::from_uuid(test_uuid(1000)),
            Choice::try_from(choice).unwrap(),
            None,
            asked.iter().copied().collect(),
            None,
        )
    }

    /// An option that asks the one target.
    fn asks(target: Target, kind: ExpectKind) -> CruxOption {
        option_asking("this", &[(target, kind)])
    }

    fn now(revision: Revision) -> BTreeMap<Target, Revision> {
        BTreeMap::from([(line(), revision), (detail(), revision)])
    }

    /// No target left out of a decision.
    fn none() -> BTreeSet<Target> {
        BTreeSet::new()
    }

    #[test]
    fn a_new_change_request_is_open_and_keeps_what_it_was_opened_with() {
        let c = cr();
        assert!(!c.is_resolved());
        assert!(c.decided_expects().is_empty());
        assert!(c.assumptions().is_empty());
        assert_eq!(c.intent(), Intent::ChangeBehavior);
        assert_eq!(c.parent(), Some(id(9)));
        assert_eq!(c.issuer().unwrap().as_str(), "designer");
        assert_eq!(c.expects()[&line()], change_at_first());
        assert_eq!(c.expectations().count(), 1);
    }

    #[test]
    fn a_change_request_keeps_the_findings_it_addresses_once_each() {
        use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
        let a = FindingRef::new(
            RuleId::from_static("cite.uncited-clause"),
            Some(Subject::Clause(ClauseId::from_uuid(test_uuid(30)))),
        );
        let b = FindingRef::new(RuleId::from_static("context.no-harness"), None);
        let mut d = draft();
        d.addresses = vec![a.clone(), b.clone(), a.clone()];
        assert_eq!(ChangeRequest::open(id(1), d).addresses(), [a, b]);
    }

    #[test]
    fn a_change_request_is_not_its_own_parent() {
        let mut d = draft();
        d.parent = Some(id(1));
        assert_eq!(ChangeRequest::open(id(1), d.clone()).parent(), None);
        d.parent = Some(id(2));
        assert_eq!(ChangeRequest::open(id(1), d).parent(), Some(id(2)));
    }

    #[test]
    fn a_decision_records_the_revisions_of_what_the_option_expects_and_leaves_its_own() {
        let mut c = cr();
        let later = Revision::first().next();
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Target),
            &now(later),
            answer(1),
            &none(),
        )
        .unwrap();
        assert_eq!(
            c.decided_expects(),
            [DecidedExpectation {
                target: detail(),
                kind: ExpectKind::Target,
                recorded: later,
                answer: answer(1)
            }]
        );
        // What it was opened with keeps its revision: it holds whatever is decided.
        assert_eq!(c.expects()[&line()], change_at_first());
        assert_eq!(c.expectations().count(), 2);
    }

    #[test]
    fn an_option_may_name_a_target_or_a_keep_and_records_the_kind() {
        for kind in [ExpectKind::Target, ExpectKind::Keep] {
            let mut c = cr();
            c.record_decided_expects(
                &asks(detail(), kind),
                &now(Revision::first()),
                answer(1),
                &none(),
            )
            .unwrap();
            assert_eq!(c.decided_expects()[0].kind, kind);
        }
    }

    #[test]
    fn a_target_gone_when_decided_is_not_recorded() {
        let mut c = cr();
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Target),
            &BTreeMap::new(),
            answer(1),
            &none(),
        )
        .unwrap();
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn a_target_left_out_is_not_recorded_but_is_still_read_for_a_clash() {
        let mut c = cr();
        let other = Target::Clause(ClauseId::from_uuid(test_uuid(32)));
        let both = option_asking(
            "both",
            &[(detail(), ExpectKind::Target), (other, ExpectKind::Keep)],
        );
        let all = BTreeMap::from([
            (detail(), Revision::first()),
            (other, Revision::first()),
            (line(), Revision::first()),
        ]);
        c.record_decided_expects(&both, &all, answer(1), &BTreeSet::from([detail()]))
            .unwrap();
        // Only the one that is not left out is recorded.
        assert_eq!(c.decided_expects().len(), 1);
        assert_eq!(c.decided_expects()[0].target, other);
        // The clash is read on the whole option: the request asks `line` to be revised, and this keeps it.
        let clash = option_asking(
            "clash",
            &[(detail(), ExpectKind::Target), (line(), ExpectKind::Keep)],
        );
        assert_eq!(
            c.record_decided_expects(&clash, &all, answer(2), &BTreeSet::from([line()])),
            Err(ChangeRequestError::ExpectationConflict {
                option: clash.choice().clone(),
                target: line()
            })
        );
    }

    #[test]
    fn a_target_two_decisions_ask_with_the_same_kind_has_an_entry_of_each_at_its_own_revision() {
        let mut c = cr();
        let first = Revision::first();
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Keep),
            &now(first),
            answer(1),
            &none(),
        )
        .unwrap();
        // A later decision asks the same, when the target has moved on.
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Keep),
            &now(first.next()),
            answer(2),
            &none(),
        )
        .unwrap();
        let held: Vec<(AnswerId, Revision)> = c
            .decided_expects()
            .iter()
            .map(|d| (d.answer, d.recorded))
            .collect();
        assert_eq!(held, [(answer(1), first), (answer(2), first.next())]);
        // It is expected once, from the earliest revision, so a keep is judged against it.
        let expected: Vec<_> = c.expectations().filter(|(t, _)| *t == detail()).collect();
        assert_eq!(
            expected,
            [(
                detail(),
                Expectation {
                    kind: ExpectKind::Keep,
                    recorded: first
                }
            )]
        );
        // The same decision never records a target twice.
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Keep),
            &now(first.next().next()),
            answer(2),
            &none(),
        )
        .unwrap();
        assert_eq!(c.decided_expects().len(), 2);
    }

    #[test]
    fn taking_back_one_of_two_decisions_that_ask_a_target_leaves_what_the_other_asks() {
        let mut c = cr();
        let first = Revision::first();
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Keep),
            &now(first),
            answer(1),
            &none(),
        )
        .unwrap();
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Keep),
            &now(first.next()),
            answer(2),
            &none(),
        )
        .unwrap();
        c.take_back_decided_expects(answer(1)).unwrap();
        assert_eq!(
            c.decided_expects(),
            [DecidedExpectation {
                target: detail(),
                kind: ExpectKind::Keep,
                recorded: first.next(),
                answer: answer(2)
            }]
        );
        assert_eq!(c.kind_asked(&detail(), None), Some(ExpectKind::Keep));
        // With the second left out too, nothing asks it.
        assert_eq!(c.kind_asked(&detail(), Some(answer(2))), None);
    }

    #[test]
    fn a_target_the_request_asks_itself_with_the_same_kind_is_not_recorded_again() {
        let mut c = cr();
        c.record_decided_expects(
            &asks(line(), ExpectKind::Target),
            &now(Revision::first().next()),
            answer(1),
            &none(),
        )
        .unwrap();
        assert!(c.decided_expects().is_empty());
        assert_eq!(c.expectations().count(), 1);
        assert_eq!(c.expects()[&line()], change_at_first());
    }

    #[test]
    fn a_target_asked_with_another_kind_is_refused_and_nothing_is_recorded() {
        let mut c = cr();
        // The request asks `line` to be revised.
        let both = option_asking(
            "both",
            &[(detail(), ExpectKind::Target), (line(), ExpectKind::Keep)],
        );
        assert_eq!(
            c.record_decided_expects(&both, &now(Revision::first()), answer(1), &none()),
            Err(ChangeRequestError::ExpectationConflict {
                option: both.choice().clone(),
                target: line()
            })
        );
        assert!(c.decided_expects().is_empty());
        // An earlier decision asked `detail` to be revised.
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Target),
            &now(Revision::first()),
            answer(1),
            &none(),
        )
        .unwrap();
        let keeps = asks(detail(), ExpectKind::Keep);
        assert_eq!(
            c.record_decided_expects(&keeps, &now(Revision::first()), answer(2), &none()),
            Err(ChangeRequestError::ExpectationConflict {
                option: keeps.choice().clone(),
                target: detail()
            })
        );
        assert_eq!(c.decided_expects().len(), 1);
    }

    #[test]
    fn an_option_is_checked_against_the_request_and_the_decisions_that_stand() {
        let mut c = cr();
        // Asking nothing, or what the request asks, is no clash.
        assert_eq!(c.check_option(&option_asking("none", &[]), None), Ok(()));
        assert_eq!(
            c.check_option(&asks(line(), ExpectKind::Target), None),
            Ok(())
        );
        // The request asks `line` to be revised; keeping it is the clash, and the option and target are named.
        let keeps_line = asks(line(), ExpectKind::Keep);
        let clash = |option: &CruxOption, target| ChangeRequestError::ExpectationConflict {
            option: option.choice().clone(),
            target,
        };
        assert_eq!(
            c.check_option(&keeps_line, None),
            Err(clash(&keeps_line, line()))
        );
        // The first target asked with the other kind is the one named.
        let two = option_asking(
            "two",
            &[(line(), ExpectKind::Keep), (detail(), ExpectKind::Keep)],
        );
        assert_eq!(c.check_option(&two, None), Err(clash(&two, line())));
        // A decision that stands asks `detail` to be revised.
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Target),
            &now(Revision::first()),
            answer(1),
            &none(),
        )
        .unwrap();
        let keeps_detail = asks(detail(), ExpectKind::Keep);
        assert_eq!(
            c.check_option(&keeps_detail, None),
            Err(clash(&keeps_detail, detail()))
        );
        // Leaving that decision out (it is about to be superseded) frees `detail`, but not what the request asks.
        assert_eq!(c.check_option(&keeps_detail, Some(answer(1))), Ok(()));
        assert_eq!(
            c.check_option(&keeps_line, Some(answer(1))),
            Err(clash(&keeps_line, line()))
        );
        // Another decision left out frees nothing.
        assert_eq!(
            c.check_option(&keeps_detail, Some(answer(2))),
            Err(clash(&keeps_detail, detail()))
        );
        // It only reads.
        assert_eq!(c.decided_expects().len(), 1);
    }

    #[test]
    fn taking_back_a_decision_removes_what_it_asked_and_nothing_else() {
        let mut c = cr();
        c.record_decided_expects(
            &asks(detail(), ExpectKind::Target),
            &now(Revision::first()),
            answer(1),
            &none(),
        )
        .unwrap();
        let other = Target::Clause(ClauseId::from_uuid(test_uuid(32)));
        c.record_decided_expects(
            &asks(other, ExpectKind::Keep),
            &BTreeMap::from([(other, Revision::first())]),
            answer(2),
            &none(),
        )
        .unwrap();
        c.take_back_decided_expects(answer(1)).unwrap();
        assert_eq!(c.decided_expects().len(), 1);
        assert_eq!(c.decided_expects()[0].answer, answer(2));
        // What it was opened with is not a decision's.
        assert_eq!(c.expects()[&line()], change_at_first());
        // The kind a taken-back target was asked with is free again, and a decision can leave its own out.
        assert_eq!(c.kind_asked(&detail(), None), None);
        assert_eq!(c.kind_asked(&other, Some(answer(2))), None);
        assert_eq!(
            c.kind_asked(&other, Some(answer(1))),
            Some(ExpectKind::Keep)
        );
        assert_eq!(
            c.kind_asked(&line(), Some(answer(1))),
            Some(ExpectKind::Target)
        );
        c.resolve(done()).unwrap();
        assert_eq!(
            c.take_back_decided_expects(answer(2)),
            Err(ChangeRequestError::AlreadyResolved)
        );
        assert_eq!(c.decided_expects().len(), 1);
    }

    #[test]
    fn a_resolved_change_request_takes_no_decided_expectation() {
        let mut c = cr();
        c.resolve(done()).unwrap();
        assert_eq!(
            c.record_decided_expects(
                &asks(detail(), ExpectKind::Target),
                &now(Revision::first()),
                answer(1),
                &none()
            ),
            Err(ChangeRequestError::AlreadyResolved)
        );
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn a_stored_history_alternates_between_resolving_and_reopening() {
        let restore =
            |history| ChangeRequest::restore(id(1), draft(), Vec::new(), Vec::new(), history);
        let resolved = || Transition::Resolved(done());
        let reopened = || Transition::Reopened(reopening());
        assert!(restore(vec![resolved(), reopened(), resolved()]).is_ok());
        for out_of_order in [
            vec![reopened()],
            vec![resolved(), resolved()],
            vec![resolved(), reopened(), reopened()],
        ] {
            assert_eq!(
                restore(out_of_order),
                Err(ChangeRequestError::HistoryOutOfOrder)
            );
        }
        let reopened_once = restore(vec![resolved(), reopened()]).unwrap();
        assert!(!reopened_once.is_resolved());
        assert_eq!(reopened_once.history().len(), 2);
    }

    #[test]
    fn a_stored_change_request_keeps_the_expectations_it_recorded() {
        let recorded = vec![DecidedExpectation {
            target: detail(),
            kind: ExpectKind::Target,
            recorded: Revision::first(),
            answer: answer(1),
        }];
        let c = ChangeRequest::restore(id(1), draft(), recorded.clone(), Vec::new(), Vec::new())
            .unwrap();
        assert_eq!(c.decided_expects(), recorded);
    }

    #[test]
    fn a_change_request_resolves_and_then_takes_no_second_resolution() {
        let mut c = cr();
        c.resolve(done()).unwrap();
        assert!(c.is_resolved());
        assert_eq!(c.resolve(done()), Err(ChangeRequestError::AlreadyResolved));
    }

    #[test]
    fn a_resolution_keeps_its_reason_maker_and_time() {
        let mut c = cr();
        c.resolve(done()).unwrap();
        let r = c.resolution().unwrap();
        assert_eq!(r.reason().as_str(), "settled");
        assert_eq!(r.by().to_string(), "person alice");
        assert_eq!(r.at().millis(), 9);
    }

    #[test]
    fn reopening_keeps_the_history_and_takes_more_work_and_another_resolution() {
        let mut c = cr();
        assert_eq!(c.reopen(reopening()), Err(ChangeRequestError::NotResolved));
        c.resolve(done()).unwrap();
        assert_eq!(
            c.record_work(Footprint::new(2, Vec::new())),
            Err(ChangeRequestError::AlreadyResolved)
        );
        c.reopen(reopening()).unwrap();
        assert!(!c.is_resolved());
        assert!(c.resolution().is_none());
        // The resolution stays in the history, then the reopening.
        assert_eq!(c.history().len(), 2);
        assert!(matches!(&c.history()[0], Transition::Resolved(r) if r.kind().is_done()));
        assert!(
            matches!(&c.history()[1], Transition::Reopened(r) if r.by().to_string() == "person bob")
        );
        // It takes work and assumptions again, and can be resolved anew; the log grows.
        c.record_work(Footprint::new(3, Vec::new())).unwrap();
        c.assume(assumption("a half is a tie")).unwrap();
        c.resolve(resolution(ResolutionKind::WontDo)).unwrap();
        assert_eq!(c.history().len(), 3);
        assert_eq!(
            c.resolution().map(|r| r.kind()),
            Some(ResolutionKind::WontDo)
        );
        assert_eq!(c.reopen(reopening()), Ok(()));
        assert_eq!(c.history().len(), 4);
    }

    fn assumption(statement: &str) -> Assumption {
        Assumption::new(
            prose(statement),
            prose("the code does it this way"),
            vec![
                Evidence::Repo {
                    path: RepoPath::try_from("src/round.rs").unwrap(),
                    lines: None,
                },
                Evidence::Knowledge {
                    reference: KnowledgeRef::try_from("IEEE 754 rounding").unwrap(),
                },
            ],
        )
    }

    #[test]
    fn assumptions_are_appended_in_order_and_never_changed() {
        let mut c = cr();
        c.assume(assumption("a half is a tie")).unwrap();
        c.assume(assumption("a total is whole cents")).unwrap();
        let said: Vec<&str> = c
            .assumptions()
            .iter()
            .map(|a| a.statement().as_str())
            .collect();
        assert_eq!(said, ["a half is a tie", "a total is whole cents"]);
        assert_eq!(c.assumptions()[0].evidence().len(), 2);
        assert_eq!(c.assumptions()[0], assumption("a half is a tie"));
    }

    #[test]
    fn a_resolved_change_request_takes_no_assumption() {
        let mut c = cr();
        c.assume(assumption("a half is a tie")).unwrap();
        c.resolve(done()).unwrap();
        assert_eq!(
            c.assume(assumption("another")),
            Err(ChangeRequestError::AlreadyResolved)
        );
        assert_eq!(c.assumptions().len(), 1);
    }

    #[test]
    fn the_assumptions_it_is_opened_with_are_kept() {
        let mut d = draft();
        d.assumptions = vec![assumption("a half is a tie")];
        let c = ChangeRequest::open(id(1), d);
        assert_eq!(c.assumptions().len(), 1);
    }

    // --- correction

    use ChangeRequestField as Field;

    const TOKEN: &str = "tok-SECRET-123";

    fn sought() -> SoughtText {
        SoughtText::try_from(TOKEN).unwrap()
    }

    fn correction(fields: Vec<Field>) -> Correction<Field> {
        Correction::new(
            fields,
            prose("a token was pasted"),
            DecisionMaker::person("carol").unwrap(),
            Timestamp::from_millis(50),
        )
        .unwrap()
    }

    fn free(_: &Field, _: &str) -> bool {
        false
    }

    /// A change request that holds the token in every field a correction reaches, resolved, reopened, resolved.
    fn leaky() -> ChangeRequest {
        let leak = |s: &str| format!("{s} {TOKEN}");
        let mut d = draft();
        d.title = Title::try_from(leak("title").as_str()).unwrap();
        d.situation = Some(prose(&leak("situation")));
        d.issuer = Some(Issuer::try_from(leak("issuer").as_str()).unwrap());
        d.assumptions = vec![Assumption::new(
            prose(&leak("statement")),
            prose(&leak("reason")),
            vec![
                Evidence::Repo {
                    path: RepoPath::try_from(leak("src/p").replace(' ', "/").as_str()).unwrap(),
                    lines: None,
                },
                Evidence::Knowledge {
                    reference: KnowledgeRef::try_from(leak("rfc").as_str()).unwrap(),
                },
            ],
        )];
        let mut c = ChangeRequest::open(id(1), d);
        c.resolve(Resolution::new(
            ResolutionKind::Done,
            prose(&leak("shipped")),
            DecisionMaker::role(&leak("lead")).unwrap(),
            Timestamp::from_millis(9),
        ))
        .unwrap();
        c.reopen(Reopening::new(
            prose(&leak("again")),
            DecisionMaker::person(&leak("bob")).unwrap(),
            Timestamp::from_millis(10),
        ))
        .unwrap();
        c
    }

    fn every_field() -> Vec<Field> {
        leaky().texts().into_iter().map(|(f, _)| f).collect()
    }

    #[test]
    fn a_change_request_lists_every_text_it_holds_in_a_fixed_order() {
        let words: Vec<String> = every_field().iter().map(Field::word).collect();
        assert_eq!(
            words,
            [
                "title",
                "situation",
                "issuer",
                "assumption[0].statement",
                "assumption[0].reason",
                "assumption[0].evidence[0]",
                "assumption[0].evidence[1]",
                "step[0].reason",
                "step[0].by",
                "step[1].reason",
                "step[1].by",
            ]
        );
        // a field that is not set is not listed
        let mut d = draft();
        d.situation = None;
        d.issuer = None;
        let plain = ChangeRequest::open(id(1), d);
        assert_eq!(plain.texts().len(), 1);
    }

    #[test]
    fn a_correction_replaces_the_text_in_each_field_it_names_and_only_there() {
        let mut c = leaky();
        let before = c.clone();
        c.correct(
            &sought(),
            correction(vec![Field::Situation, Field::StepReason { step: 0 }]),
            &free,
        )
        .unwrap();
        assert_eq!(c.situation().unwrap().as_str(), "situation [redacted]");
        let Transition::Resolved(r) = &c.history()[0] else {
            panic!("the first step stays a resolution")
        };
        assert_eq!(r.reason().as_str(), "shipped [redacted]");
        assert_eq!(r.kind(), ResolutionKind::Done);
        assert_eq!(r.at().millis(), 9);
        // the others still hold it
        assert!(c.title().as_str().contains(TOKEN));
        assert!(c.issuer().unwrap().as_str().contains(TOKEN));
        assert!(c.assumptions()[0].statement().as_str().contains(TOKEN));
        assert_eq!(c.history().len(), before.history().len() + 1);
        // the step keeps the fields, why, who and when, and not the text
        let Some(Transition::Corrected(step)) = c.history().last() else {
            panic!("the correction is the last step")
        };
        assert_eq!(
            step.fields(),
            [Field::Situation, Field::StepReason { step: 0 }]
        );
        assert_eq!(step.reason().as_str(), "a token was pasted");
        assert_eq!(step.by().to_string(), "person carol");
        assert_eq!(step.at().millis(), 50);
        assert!(!format!("{step:?}").contains("SECRET"));
    }

    #[test]
    fn a_correction_reaches_every_field_and_rebuilds_the_values_it_replaced() {
        let mut c = leaky();
        c.correct(&sought(), correction(every_field()), &free)
            .unwrap();
        assert_eq!(c.title().as_str(), "title [redacted]");
        assert_eq!(c.issuer().unwrap().as_str(), "issuer [redacted]");
        let a = &c.assumptions()[0];
        assert_eq!(a.statement().as_str(), "statement [redacted]");
        assert_eq!(a.reason().as_str(), "reason [redacted]");
        assert!(
            matches!(&a.evidence()[0], Evidence::Repo { path, .. } if path.as_str() == "src/p/[redacted]")
        );
        assert!(
            matches!(&a.evidence()[1], Evidence::Knowledge { reference } if reference.as_str() == "rfc [redacted]")
        );
        // the kind of a maker is kept; only the name is corrected
        assert!(
            matches!(&c.history()[0], Transition::Resolved(r) if r.by().to_string() == "role lead [redacted]")
        );
        assert!(
            matches!(&c.history()[1], Transition::Reopened(r) if r.by().to_string() == "person bob [redacted]")
        );
        // nothing of the request is left to find
        assert!(c.texts().iter().all(|(_, t)| !t.contains(TOKEN)));
        // the state is not changed by a correction
        assert!(!c.is_resolved());
    }

    #[test]
    fn a_correction_is_all_or_nothing() {
        let mut c = leaky();
        let before = c.clone();
        // a field that does not hold the text
        let mut d = draft();
        d.situation = Some(prose("plain"));
        let mut plain = ChangeRequest::open(id(1), d);
        let plain_before = plain.clone();
        assert_eq!(
            plain.correct(&sought(), correction(vec![Field::Situation]), &free),
            Err(CorrectionError::TextAbsent("situation".into()))
        );
        assert_eq!(plain, plain_before);
        // a field that is not there
        assert_eq!(
            c.correct(
                &sought(),
                correction(vec![
                    Field::Title,
                    Field::AssumptionReason { assumption: 9 }
                ]),
                &free
            ),
            Err(CorrectionError::NoSuchField("assumption[9].reason".into()))
        );
        assert_eq!(
            c.correct(
                &sought(),
                correction(vec![Field::AssumptionEvidence {
                    assumption: 0,
                    evidence: 2
                }]),
                &free
            ),
            Err(CorrectionError::NoSuchField(
                "assumption[0].evidence[2]".into()
            ))
        );
        assert_eq!(c, before);
    }

    #[test]
    fn a_correction_step_has_no_reason_or_maker_to_correct() {
        let mut c = leaky();
        c.correct(&sought(), correction(vec![Field::Title]), &free)
            .unwrap();
        // the step is the third (index 2) and holds no text a correction reaches
        let before = c.clone();
        for field in [Field::StepReason { step: 2 }, Field::StepBy { step: 2 }] {
            assert_eq!(
                c.correct(&sought(), correction(vec![field]), &free),
                Err(CorrectionError::NoSuchField(field.word()))
            );
        }
        assert_eq!(c, before);
    }

    #[test]
    fn a_title_that_would_equal_another_takes_a_numbered_mark() {
        let mut c = leaky();
        let others = ["title [redacted]", "title [redacted 2]"];
        let taken =
            |field: &Field, candidate: &str| *field == Field::Title && others.contains(&candidate);
        c.correct(&sought(), correction(vec![Field::Title]), &taken)
            .unwrap();
        assert_eq!(c.title().as_str(), "title [redacted 3]");
        // another field uses the plain mark, whatever `taken` says
        let mut c = leaky();
        c.correct(&sought(), correction(vec![Field::Situation]), &|_, _| true)
            .unwrap();
        assert_eq!(c.situation().unwrap().as_str(), "situation [redacted]");
    }

    #[test]
    fn a_correction_stands_between_a_resolution_and_a_reopening_without_changing_the_state() {
        let resolved = || Transition::Resolved(done());
        let reopened = || Transition::Reopened(reopening());
        let corrected = || Transition::Corrected(correction(vec![Field::Title]));
        let restore =
            |history| ChangeRequest::restore(id(1), draft(), Vec::new(), Vec::new(), history);
        let c = restore(vec![resolved(), corrected(), reopened(), corrected()]).unwrap();
        assert!(!c.is_resolved());
        assert!(c.resolution().is_none());
        // a correction after a resolution: still resolved, and the resolution is the one before it
        let c = restore(vec![resolved(), corrected()]).unwrap();
        assert!(c.is_resolved());
        assert_eq!(c.resolution().map(|r| r.kind()), Some(ResolutionKind::Done));
        // a correction first: nothing resolved yet, and a resolution may follow
        let mut c = restore(vec![corrected()]).unwrap();
        assert!(!c.is_resolved());
        c.resolve(done()).unwrap();
        assert!(c.is_resolved());
        // corrections do not hide an order that is wrong
        for out_of_order in [
            vec![corrected(), reopened()],
            vec![resolved(), corrected(), resolved()],
        ] {
            assert_eq!(
                restore(out_of_order),
                Err(ChangeRequestError::HistoryOutOfOrder)
            );
        }
        // a resolved request that was corrected can be reopened
        let mut c = restore(vec![resolved(), corrected()]).unwrap();
        assert_eq!(c.reopen(reopening()), Ok(()));
        assert_eq!(c.history().len(), 3);
    }
}
