//! The change request: a request that the definitions change (or a confirmation that they need not). It names the
//! targets it asks to be revised or retired, and the targets to keep. A question that only a decision maker can
//! answer is a [`Crux`](super::crux::Crux), a record of its own that is raised in a change request and stops it; the
//! change request does not hold it. The option a decision maker chooses may name more targets, and the change
//! request records those when it is decided.
//!
//! A request ends by an explicit resolution (how it ended, why, who, when), and can be opened again; the history of
//! both is only appended to. Neither the targets nor the revisions of the definitions decide whether it may end.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::assumption::Assumption;
use crate::domain::vo::choice::Choice;
use crate::domain::vo::finding::FindingRef;
use crate::domain::vo::footprint::Footprint;
use crate::domain::vo::id::{ChangeRequestId, CruxId, CruxOptionId};
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
    /// A recommendation or a decision names an option that the Crux does not have.
    #[error("the option {0} is not one of the options of the crux")]
    NotAnOption(CruxOptionId),
    /// A model question is about words; it cannot be left unspecified.
    #[error("a {0} question cannot be left unspecified")]
    UnspecifiedNotAllowed(&'static str),
    /// The change request is resolved; it takes no more decisions and no more assumptions.
    #[error("the change request is resolved")]
    AlreadyResolved,
    /// A Crux raised in the change request has no decision yet, so the change request cannot be resolved, whatever
    /// the resolution.
    #[error("a crux raised in the change request has no decision yet")]
    Undecided,
    /// The Crux is answered already. An answer is an external input that cannot be reproduced, so it is never
    /// rewritten: a different decision is a new Crux that supersedes this one.
    #[error("the crux is answered already; a different decision is a new crux that supersedes it")]
    AlreadyAnswered,
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
    /// A resolution points at a change request that does not exist.
    #[error("the change request {0} that the resolution points at does not exist")]
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
    /// A stored expectation was not asked by any decision of the Cruxes raised in the change request.
    #[error("{0:?} is not an expectation the decision asks")]
    UnaskedExpectation(Target),
    /// The option chosen asks of a target the other of being revised or retired and kept, where the change request
    /// (or an earlier decision) asks it already.
    #[error("{0:?} is asked to be revised or retired and to be kept")]
    ExpectationConflict(Target),
    /// A stored Crux was raised in a change request that does not exist.
    #[error("the change request {0} that a crux was raised in does not exist")]
    RaisedInUnknown(ChangeRequestId),
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
    /// What the options chosen expect, with the revision each target had when it was decided.
    decided_expects: BTreeMap<Target, Expectation>,
    assumptions: Vec<Assumption>,
    addresses: Vec<FindingRef>,
    /// What the change sets applied for it moved, in the order they were kept.
    work: Vec<Footprint>,
    /// Resolutions and reopenings, in the order they happened; resolved when the last is a resolution.
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
            decided_expects: BTreeMap::new(),
            assumptions: draft.assumptions,
            addresses: distinct(draft.addresses),
            work: Vec::new(),
            history: Vec::new(),
        }
    }

    /// A change request as a store kept it: the expectations the decisions asked, with the revisions recorded, and
    /// its history. A history alternates between a resolution and a reopening, starting with a resolution. Whether
    /// the expectations recorded are ones a decision asks, and whether a request resolved as done has its Cruxes
    /// decided, are read with the Cruxes, which are other records
    /// (`service::change_request::check_requests_and_cruxes`). Whether the parts of a resolved one are resolved is
    /// not asked either.
    pub(crate) fn restore(
        id: ChangeRequestId,
        draft: ChangeRequestDraft,
        decided_expects: BTreeMap<Target, Expectation>,
        work: Vec<Footprint>,
        history: Vec<Transition>,
    ) -> Result<ChangeRequest, ChangeRequestError> {
        let in_order = history
            .iter()
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

    /// What the options chosen for the Cruxes raised in it expect of targets, with the revision each had when the
    /// Crux was decided. Empty until decided, and when the decisions expect nothing more.
    pub fn decided_expects(&self) -> &BTreeMap<Target, Expectation> {
        &self.decided_expects
    }

    /// Every expectation that holds now: its own, then those the decisions asked. A target is in one of them, or in
    /// both with the same kind: a decision that would add it to the other with another kind is refused.
    pub fn expectations(&self) -> impl Iterator<Item = (Target, Expectation)> + '_ {
        self.expects
            .iter()
            .chain(&self.decided_expects)
            .map(|(t, e)| (*t, *e))
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
    /// more work (until it is reopened).
    pub fn record_work(&mut self, footprint: Footprint) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        self.work.push(footprint);
        Ok(())
    }

    /// Whether the last step of its history is a resolution.
    pub fn is_resolved(&self) -> bool {
        matches!(self.history.last(), Some(Transition::Resolved(_)))
    }

    /// The resolution it stands under, when it is resolved.
    pub fn resolution(&self) -> Option<&Resolution> {
        match self.history.last() {
            Some(Transition::Resolved(r)) => Some(r),
            _ => None,
        }
    }

    /// Every resolution and reopening, in the order they happened. It only grows: a reopening does not take the
    /// resolution it follows out of it.
    pub fn history(&self) -> &[Transition] {
        &self.history
    }

    /// Records what the option chosen for a Crux raised in it expects: each target in `asked` takes its kind and the
    /// revision in `now`, the one it has as the decision is taken (a target missing from `now` is gone: it is not
    /// recorded). A target recorded already with the same kind keeps the revision it was first recorded at, and a
    /// keep goes on being compared with that one; a target this request asks itself with the same kind is not
    /// recorded again. A target that is asked with another kind, by the request or by an earlier decision, is
    /// refused, and nothing is recorded. A resolved request is a closed record and takes no more (until it is
    /// reopened).
    pub fn record_decided_expects(
        &mut self,
        asked: &BTreeMap<Target, ExpectKind>,
        now: &BTreeMap<Target, Revision>,
    ) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        let mut add = Vec::new();
        for (target, kind) in asked {
            let held = self
                .expects
                .get(target)
                .or_else(|| self.decided_expects.get(target));
            match held {
                Some(e) if e.kind != *kind => {
                    return Err(ChangeRequestError::ExpectationConflict(*target));
                }
                Some(_) => {}
                None => {
                    if let Some(recorded) = now.get(target) {
                        add.push((
                            *target,
                            Expectation {
                                kind: *kind,
                                recorded: *recorded,
                            },
                        ));
                    }
                }
            }
        }
        self.decided_expects.extend(add);
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

    fn asks(target: Target, kind: ExpectKind) -> BTreeMap<Target, ExpectKind> {
        BTreeMap::from([(target, kind)])
    }

    fn now(revision: Revision) -> BTreeMap<Target, Revision> {
        BTreeMap::from([(line(), revision), (detail(), revision)])
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
        c.record_decided_expects(&asks(detail(), ExpectKind::Target), &now(later))
            .unwrap();
        assert_eq!(
            c.decided_expects(),
            &BTreeMap::from([(
                detail(),
                Expectation {
                    kind: ExpectKind::Target,
                    recorded: later
                }
            )])
        );
        // What it was opened with keeps its revision: it holds whatever is decided.
        assert_eq!(c.expects()[&line()], change_at_first());
        assert_eq!(c.expectations().count(), 2);
    }

    #[test]
    fn an_option_may_name_a_target_or_a_keep_and_records_the_kind() {
        for kind in [ExpectKind::Target, ExpectKind::Keep] {
            let mut c = cr();
            c.record_decided_expects(&asks(detail(), kind), &now(Revision::first()))
                .unwrap();
            assert_eq!(c.decided_expects()[&detail()].kind, kind);
        }
    }

    #[test]
    fn a_target_gone_when_decided_is_not_recorded() {
        let mut c = cr();
        c.record_decided_expects(&asks(detail(), ExpectKind::Target), &BTreeMap::new())
            .unwrap();
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn a_target_asked_again_with_the_same_kind_keeps_the_revision_it_was_first_recorded_at() {
        let mut c = cr();
        let first = Revision::first();
        c.record_decided_expects(&asks(detail(), ExpectKind::Keep), &now(first))
            .unwrap();
        // A later decision asks the same, when the target has moved on.
        c.record_decided_expects(&asks(detail(), ExpectKind::Keep), &now(first.next()))
            .unwrap();
        assert_eq!(c.decided_expects().len(), 1);
        assert_eq!(c.decided_expects()[&detail()].recorded, first);
    }

    #[test]
    fn a_target_the_request_asks_itself_with_the_same_kind_is_not_recorded_again() {
        let mut c = cr();
        c.record_decided_expects(
            &asks(line(), ExpectKind::Target),
            &now(Revision::first().next()),
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
        let both = BTreeMap::from([(detail(), ExpectKind::Target), (line(), ExpectKind::Keep)]);
        assert_eq!(
            c.record_decided_expects(&both, &now(Revision::first())),
            Err(ChangeRequestError::ExpectationConflict(line()))
        );
        assert!(c.decided_expects().is_empty());
        // An earlier decision asked `detail` to be revised.
        c.record_decided_expects(&asks(detail(), ExpectKind::Target), &now(Revision::first()))
            .unwrap();
        assert_eq!(
            c.record_decided_expects(&asks(detail(), ExpectKind::Keep), &now(Revision::first())),
            Err(ChangeRequestError::ExpectationConflict(detail()))
        );
        assert_eq!(c.decided_expects().len(), 1);
    }

    #[test]
    fn a_resolved_change_request_takes_no_decided_expectation() {
        let mut c = cr();
        c.resolve(done()).unwrap();
        assert_eq!(
            c.record_decided_expects(&asks(detail(), ExpectKind::Target), &now(Revision::first())),
            Err(ChangeRequestError::AlreadyResolved)
        );
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn a_stored_history_alternates_between_resolving_and_reopening() {
        let restore =
            |history| ChangeRequest::restore(id(1), draft(), BTreeMap::new(), Vec::new(), history);
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
        let recorded = BTreeMap::from([(detail(), change_at_first())]);
        let c = ChangeRequest::restore(id(1), draft(), recorded.clone(), Vec::new(), Vec::new())
            .unwrap();
        assert_eq!(c.decided_expects(), &recorded);
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
}
