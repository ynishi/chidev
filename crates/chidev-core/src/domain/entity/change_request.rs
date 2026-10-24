//! The change request: a request that the definitions change (or a confirmation that they need not). It names the
//! targets it asks to be revised or retired, and the targets to keep. A Crux, when it has one, is a choice that
//! cannot be derived from the repository and general knowledge (those are Assumptions, recorded on the request) nor
//! from the Project's decision Policy, so a decision maker picks; the option chosen may name more targets, and the
//! answer is recorded once, with its basis, who gave it and when.
//!
//! A request ends by an explicit resolution (how it ended, why, who, when), and can be opened again; the history of
//! both is only appended to. Neither the targets nor the revisions of the definitions decide whether it may end.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::answer::Answer;
use crate::domain::vo::assumption::Assumption;
use crate::domain::vo::choice::Choice;
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Importance, Recommended};
use crate::domain::vo::finding::FindingRef;
use crate::domain::vo::footprint::Footprint;
use crate::domain::vo::id::ChangeRequestId;
use crate::domain::vo::intent::{Intent, Issuer};
use crate::domain::vo::policy::PolicyLineId;
use crate::domain::vo::prose::Prose;
use crate::domain::vo::resolution::{Reopening, Resolution, Transition};
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::{ExpectKind, Expectation, Target};
use crate::domain::vo::title::Title;

/// Why a change request cannot be opened, decided, resolved or reopened. Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChangeRequestError {
    /// A Crux has no option to choose.
    #[error("a crux needs at least one option")]
    NoOption,
    /// Two options are the same choice.
    #[error("the option `{0}` is given twice")]
    DuplicateOption(Choice),
    /// A recommendation or a decision names a choice that is not an option.
    #[error("`{0}` is not one of the options")]
    NotAnOption(Choice),
    /// A model question is about words; it cannot be left unspecified.
    #[error("a {0} question cannot be left unspecified")]
    UnspecifiedNotAllowed(&'static str),
    /// The change request has no Crux; there is no choice for a decision maker to make.
    #[error("the change request has no crux to decide")]
    NoCrux,
    /// The change request is resolved; it takes no more decisions and no more assumptions.
    #[error("the change request is resolved")]
    AlreadyResolved,
    /// The Crux has no decision yet.
    #[error("the crux has no decision yet")]
    Undecided,
    /// The Crux is answered already. An answer is an external input that cannot be reproduced, so it is never
    /// rewritten: a different decision is a new change request whose Crux supersedes this one.
    #[error("the crux is answered already; a different decision is a new crux that supersedes it")]
    AlreadyAnswered,
    /// A Crux is to supersede the Crux of a change request that does not exist.
    #[error("the change request {0} that the crux supersedes does not exist")]
    SupersedesUnknown(ChangeRequestId),
    /// A Crux is to supersede a Crux that has no answer (or a change request that has no Crux): there is nothing
    /// to replace.
    #[error("the change request {0} that the crux supersedes has no answered crux")]
    SupersedesUnanswered(ChangeRequestId),
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
    /// A stored expectation was not asked by the decision stored with it.
    #[error("{0:?} is not an expectation the decision asks")]
    UnaskedExpectation(Target),
}

/// The choice a decision maker must make for a change request: its kind, the options with what each expects, the
/// recommendation (with how sure it is) and the importance that let the Policy settle it, the Crux it supersedes
/// when it replaces a decision, and the answer once given. There is no default to go on with: work that depends on
/// the answer waits for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crux {
    kind: CruxKind,
    options: Vec<CruxOption>,
    recommended: Option<Recommended>,
    importance: Option<Importance>,
    supersedes: Option<ChangeRequestId>,
    answer: Option<Answer>,
}

impl Crux {
    /// An unanswered Crux: one option at least, no choice twice, the recommendation among the options. It
    /// `supersedes` the answered Crux of a change request when it replaces that decision (whether that one exists and
    /// is answered is read against the other change requests; see `service::change_request::check_supersedes`).
    pub fn new(
        kind: CruxKind,
        options: Vec<CruxOption>,
        recommended: Option<Recommended>,
        importance: Option<Importance>,
        supersedes: Option<ChangeRequestId>,
    ) -> Result<Crux, ChangeRequestError> {
        if options.is_empty() {
            return Err(ChangeRequestError::NoOption);
        }
        for (k, o) in options.iter().enumerate() {
            if options[..k].iter().any(|p| p.choice() == o.choice()) {
                return Err(ChangeRequestError::DuplicateOption(o.choice().clone()));
            }
        }
        if let Some(r) = recommended
            .as_ref()
            .filter(|r| !options.iter().any(|o| o.choice() == r.choice()))
        {
            return Err(ChangeRequestError::NotAnOption(r.choice().clone()));
        }
        Ok(Crux {
            kind,
            options,
            recommended,
            importance,
            supersedes,
            answer: None,
        })
    }

    /// What kind of question it is.
    pub fn kind(&self) -> CruxKind {
        self.kind
    }

    /// The options, in the order given.
    pub fn options(&self) -> &[CruxOption] {
        &self.options
    }

    /// The option with this choice.
    pub fn option(&self, choice: &Choice) -> Option<&CruxOption> {
        self.options.iter().find(|o| o.choice() == choice)
    }

    /// The option recommended, why, and how sure.
    pub fn recommended(&self) -> Option<&Recommended> {
        self.recommended.as_ref()
    }

    /// How much the answer matters, when it was said.
    pub fn importance(&self) -> Option<Importance> {
        self.importance
    }

    /// The change request whose answered Crux this one replaces.
    pub fn supersedes(&self) -> Option<ChangeRequestId> {
        self.supersedes
    }

    /// The answer, once given.
    pub fn answer(&self) -> Option<&Answer> {
        self.answer.as_ref()
    }

    /// The decision, once taken.
    pub fn decision(&self) -> Option<&Decision> {
        self.answer.as_ref().map(Answer::decision)
    }

    /// What the decision names besides the targets and keeps of the change request: those of the option chosen.
    /// The decision maker's own wording names nothing more, leaving it unspecified names nothing, and no decision
    /// yet names nothing.
    pub fn decided_expects(&self) -> Vec<(Target, ExpectKind)> {
        let chosen = match self.decision() {
            Some(Decision::Chosen(c)) => self.option(c.choice()),
            _ => None,
        };
        chosen.map_or_else(Vec::new, |o| {
            o.expects().iter().map(|(t, k)| (*t, *k)).collect()
        })
    }

    /// Checks `answer` against the options and the kind, and takes it. An answer already taken stays.
    fn take(&mut self, answer: Answer) -> Result<(), ChangeRequestError> {
        if self.answer.is_some() {
            return Err(ChangeRequestError::AlreadyAnswered);
        }
        match answer.decision() {
            Decision::Chosen(c) if self.option(c.choice()).is_none() => {
                return Err(ChangeRequestError::NotAnOption(c.choice().clone()));
            }
            Decision::Unspecified { .. } if !self.kind.allows_unspecified() => {
                return Err(ChangeRequestError::UnspecifiedNotAllowed(self.kind.word()));
            }
            _ => {}
        }
        self.answer = Some(answer);
        Ok(())
    }
}

/// Where a change request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRequestState {
    /// Not resolved, and its Crux (if any) not answered: a decision maker is awaited, and what depends on the answer
    /// waits.
    Open,
    /// Its Crux is answered; it is not resolved.
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
    /// The choice for a decision maker, when there is one. A Crux given here is unanswered.
    pub crux: Option<Crux>,
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
/// had, so a keep that moved is seen). It may be part of a broader request, which need not name any target. It may
/// carry a Crux, a choice a decision maker makes; the answer is recorded once and never rewritten, and the option
/// chosen may name more targets, recorded when it is decided. It records the Assumptions made while raising it,
/// which only grow.
///
/// It is closed by a [`Resolution`] a decision maker gives (how it ended, why, who, when), and can be opened again
/// by a [`Reopening`]; the history keeps every step. Only explicit states refuse a resolution: a Crux unanswered
/// (to resolve it as done), a part not resolved, the request resolved already. The revisions of the targets and the
/// work recorded for it are read by checks, not by the resolution. Its intent (whether it means to change the
/// behaviour) is a record too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    id: ChangeRequestId,
    title: Title,
    situation: Option<Prose>,
    intent: Intent,
    parent: Option<ChangeRequestId>,
    issuer: Option<Issuer>,
    expects: BTreeMap<Target, Expectation>,
    crux: Option<Crux>,
    /// What the chosen option expects, with the revision each target had when it was decided.
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
        let crux = draft.crux.map(|mut crux| {
            crux.answer = None;
            crux
        });
        ChangeRequest {
            id,
            title: draft.title,
            situation: draft.situation,
            intent: draft.intent,
            parent: draft.parent.filter(|p| *p != id),
            issuer: draft.issuer,
            expects: draft.expects,
            crux,
            decided_expects: BTreeMap::new(),
            assumptions: draft.assumptions,
            addresses: distinct(draft.addresses),
            work: Vec::new(),
            history: Vec::new(),
        }
    }

    /// A change request as a store kept it: its Crux with the answer given, the expectations that decision asked
    /// with the revisions recorded, and its history. A history alternates between a resolution and a reopening,
    /// starting with a resolution; a resolution as done has the Crux answered (if it has one), and every
    /// expectation recorded is one the decision asks. Whether the parts of a resolved one are resolved is not
    /// asked here: they are other records.
    pub(crate) fn restore(
        id: ChangeRequestId,
        draft: ChangeRequestDraft,
        answer: Option<Answer>,
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
        let resolved_as_done = history
            .iter()
            .any(|t| matches!(t, Transition::Resolved(r) if r.kind().is_done()));
        let crux = match draft.crux {
            None if answer.is_some() => return Err(ChangeRequestError::NoCrux),
            None => None,
            Some(mut crux) => {
                crux.answer = None;
                if let Some(a) = answer {
                    crux.take(a)?;
                }
                if resolved_as_done && crux.answer.is_none() {
                    return Err(ChangeRequestError::Undecided);
                }
                Some(crux)
            }
        };
        let asked: Vec<(Target, ExpectKind)> =
            crux.iter().flat_map(|c| c.decided_expects()).collect();
        if let Some(t) = decided_expects
            .iter()
            .find(|(t, e)| !asked.contains(&(**t, e.kind)))
            .map(|(t, _)| *t)
        {
            return Err(ChangeRequestError::UnaskedExpectation(t));
        }
        Ok(ChangeRequest {
            id,
            title: draft.title,
            situation: draft.situation,
            intent: draft.intent,
            parent: draft.parent.filter(|p| *p != id),
            issuer: draft.issuer,
            expects: draft.expects,
            crux,
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
    /// They hold whatever its Crux decides.
    pub fn expects(&self) -> &BTreeMap<Target, Expectation> {
        &self.expects
    }

    /// What the chosen option of its Crux expects of targets, with the revision each had when the Crux was
    /// decided. Empty until decided, and when the decision expects nothing more.
    pub fn decided_expects(&self) -> &BTreeMap<Target, Expectation> {
        &self.decided_expects
    }

    /// Every expectation that holds now: its own, then those the decision asked. A target in both is listed twice.
    pub fn expectations(&self) -> impl Iterator<Item = (Target, Expectation)> + '_ {
        self.expects
            .iter()
            .chain(&self.decided_expects)
            .map(|(t, e)| (*t, *e))
    }

    /// Every expectation it could come to hold: its own, and those of every option of its Crux, whichever is
    /// chosen. The options' are the kind alone: they are given a revision when one is chosen.
    pub fn possible_kinds(&self) -> impl Iterator<Item = (Target, ExpectKind)> + '_ {
        self.expects.iter().map(|(t, e)| (*t, e.kind)).chain(
            self.crux
                .iter()
                .flat_map(|q| q.options())
                .flat_map(|o| o.expects().iter().map(|(t, k)| (*t, *k))),
        )
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

    /// Its Crux, when it has a choice for a decision maker.
    pub fn crux(&self) -> Option<&Crux> {
        self.crux.as_ref()
    }

    /// Whether it is unresolved and its Crux is not answered yet: a decision maker is awaited.
    pub fn awaits_decision(&self) -> bool {
        !self.is_resolved() && self.crux.as_ref().is_some_and(|c| c.answer.is_none())
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

    /// Where it stands.
    pub fn state(&self) -> ChangeRequestState {
        if self.is_resolved() {
            return ChangeRequestState::Resolved;
        }
        match self.crux().and_then(Crux::answer) {
            Some(_) => ChangeRequestState::Decided,
            None => ChangeRequestState::Open,
        }
    }

    /// Records the answer to its Crux, once: an answered Crux is never answered again (`AlreadyAnswered`), so the
    /// record of who decided what, on which basis, stays as it was given. Each target the option chosen expects
    /// something of takes the revision in `now`, the one it has as the decision is taken (one missing from `now` is
    /// gone: it is not recorded). The expectations of the change request keep the revisions they were opened
    /// with. Whether a Policy basis names a line the Policy has is read against the Project
    /// (`service::change_request::decide`).
    pub fn decide(
        &mut self,
        answer: Answer,
        now: &BTreeMap<Target, Revision>,
    ) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        let Some(crux) = &mut self.crux else {
            return Err(ChangeRequestError::NoCrux);
        };
        crux.take(answer)?;
        self.decided_expects = crux
            .decided_expects()
            .into_iter()
            .filter_map(|(t, kind)| {
                now.get(&t).map(|recorded| {
                    (
                        t,
                        Expectation {
                            kind,
                            recorded: *recorded,
                        },
                    )
                })
            })
            .collect();
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

    /// Closes the record with `resolution`. Resolving as done needs the Crux answered (a request that is not done
    /// may end with a question open); nothing else of the request is read. Whether its parts are resolved, and
    /// whether the request a resolution points at exists, are read against the other change requests, so the
    /// caller checks both first (`service::change_request::resolve`).
    pub fn resolve(&mut self, resolution: Resolution) -> Result<(), ChangeRequestError> {
        if self.is_resolved() {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        if resolution.kind().is_done() && self.awaits_decision() {
            return Err(ChangeRequestError::Undecided);
        }
        self.history.push(Transition::Resolved(resolution));
        Ok(())
    }

    /// Opens a resolved request again. The resolution stays in the history; the request takes work, decisions and
    /// assumptions again, and can be resolved anew.
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
    use crate::domain::vo::answer::{Basis, DecisionMaker};
    use crate::domain::vo::assumption::{Evidence, KnowledgeRef, RepoPath};
    use crate::domain::vo::confidence::Confidence;
    use crate::domain::vo::crux::Reasoned;
    use crate::domain::vo::id::{ClauseId, test_uuid};
    use crate::domain::vo::resolution::ResolutionKind;
    use crate::domain::vo::timestamp::Timestamp;

    fn choice(s: &str) -> Choice {
        Choice::try_from(s).unwrap()
    }

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

    fn crux(kind: CruxKind) -> Result<Crux, ChangeRequestError> {
        Crux::new(
            kind,
            vec![
                CruxOption::new(choice("up"), None, BTreeMap::new(), None),
                CruxOption::new(
                    choice("to even"),
                    Some(prose("C1: a total is rounded half to even")),
                    BTreeMap::from([(detail(), ExpectKind::Target)]),
                    Some(prose("the rounding tests change")),
                ),
                CruxOption::new(choice("keep"), None, BTreeMap::new(), None),
            ],
            Some(Recommended::new(
                choice("to even"),
                Some(prose("the literal reading")),
                Some(Confidence::try_from(85).unwrap()),
            )),
            Some(Importance::Normal),
            None,
        )
    }

    fn id(n: u128) -> ChangeRequestId {
        ChangeRequestId::from_uuid(test_uuid(n))
    }

    fn cr(crux: Option<Crux>) -> ChangeRequest {
        ChangeRequest::open(
            id(1),
            ChangeRequestDraft {
                title: Title::try_from("How is a half rounded?").unwrap(),
                situation: Some(prose("The requirement says rounded, not how.")),
                intent: Intent::ChangeBehavior,
                parent: Some(id(9)),
                issuer: Some(Issuer::try_from("designer").unwrap()),
                expects: BTreeMap::from([(line(), change_at_first())]),
                crux,
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        )
    }

    fn draft(crux: Option<Crux>) -> ChangeRequestDraft {
        ChangeRequestDraft {
            title: Title::try_from("T").unwrap(),
            situation: None,
            intent: Intent::KeepBehavior,
            parent: None,
            issuer: None,
            expects: BTreeMap::new(),
            crux,
            assumptions: Vec::new(),
            addresses: Vec::new(),
        }
    }

    fn person() -> ChangeRequest {
        cr(Some(crux(CruxKind::Ambiguity).unwrap()))
    }

    /// An answer on an arbitrary basis, by a person.
    fn answer(decision: Decision) -> Answer {
        Answer::new(
            decision,
            Basis::Arbitrary {
                reason: prose("no line of the policy covers it"),
            },
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(1_000),
        )
    }

    fn chosen(c: &str) -> Answer {
        answer(Decision::Chosen(Reasoned::new(
            choice(c),
            Some(prose("fairer")),
        )))
    }

    #[test]
    fn a_new_change_request_is_open_and_keeps_what_it_was_opened_with() {
        let c = person();
        assert_eq!(c.state(), ChangeRequestState::Open);
        assert!(c.awaits_decision());
        assert!(c.decided_expects().is_empty());
        assert!(c.assumptions().is_empty());
        assert_eq!(c.intent(), Intent::ChangeBehavior);
        assert_eq!(c.parent(), Some(id(9)));
        assert_eq!(c.issuer().unwrap().as_str(), "designer");
        assert_eq!(c.expects()[&line()], change_at_first());
        let plain = cr(None);
        assert_eq!(plain.state(), ChangeRequestState::Open);
        assert!(!plain.awaits_decision());
        assert!(plain.crux().is_none());
    }

    #[test]
    fn a_change_request_keeps_the_findings_it_addresses_once_each() {
        use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
        let a = FindingRef::new(
            RuleId::from_static("cite.uncited-clause"),
            Some(Subject::Clause(ClauseId::from_uuid(test_uuid(30)))),
        );
        let b = FindingRef::new(RuleId::from_static("context.no-harness"), None);
        let mut d = draft(None);
        d.addresses = vec![a.clone(), b.clone(), a.clone()];
        assert_eq!(ChangeRequest::open(id(1), d).addresses(), [a, b]);
    }

    #[test]
    fn a_change_request_is_not_its_own_parent() {
        let mut d = draft(None);
        d.parent = Some(id(1));
        assert_eq!(ChangeRequest::open(id(1), d.clone()).parent(), None);
        d.parent = Some(id(2));
        assert_eq!(ChangeRequest::open(id(1), d).parent(), Some(id(2)));
    }

    #[test]
    fn a_crux_has_distinct_options_and_its_recommendation_among_them() {
        assert_eq!(
            Crux::new(CruxKind::Ambiguity, Vec::new(), None, None, None),
            Err(ChangeRequestError::NoOption)
        );
        let option = |c: &str| CruxOption::new(choice(c), None, BTreeMap::new(), None);
        let twice = Crux::new(
            CruxKind::Ambiguity,
            vec![option("a"), option("a")],
            None,
            None,
            None,
        );
        assert_eq!(twice, Err(ChangeRequestError::DuplicateOption(choice("a"))));
        let stray = Crux::new(
            CruxKind::Ambiguity,
            vec![option("a")],
            Some(Recommended::new(choice("down"), None, None)),
            None,
            None,
        );
        assert_eq!(stray, Err(ChangeRequestError::NotAnOption(choice("down"))));
    }

    #[test]
    fn a_crux_keeps_its_recommendation_with_the_confidence_its_importance_and_what_it_supersedes() {
        let option = |c: &str| CruxOption::new(choice(c), None, BTreeMap::new(), None);
        let q = Crux::new(
            CruxKind::Conflict,
            vec![option("a"), option("b")],
            Some(Recommended::new(
                choice("b"),
                None,
                Some(Confidence::try_from(100).unwrap()),
            )),
            Some(Importance::High),
            Some(id(7)),
        )
        .unwrap();
        assert_eq!(
            q.recommended().and_then(Recommended::confidence),
            Some(Confidence::try_from(100).unwrap())
        );
        assert_eq!(q.importance(), Some(Importance::High));
        assert_eq!(q.supersedes(), Some(id(7)));
        assert!(q.answer().is_none());
        // A recommendation may say nothing of its confidence.
        let plain = Crux::new(
            CruxKind::Conflict,
            vec![option("a")],
            Some(Recommended::new(choice("a"), None, None)),
            None,
            None,
        )
        .unwrap();
        assert_eq!(plain.recommended().and_then(Recommended::confidence), None);
        assert_eq!(plain.importance(), None);
        assert_eq!(plain.supersedes(), None);
    }

    #[test]
    fn a_decision_records_the_revisions_of_what_the_option_expects_and_leaves_its_own() {
        let mut c = person();
        let now = BTreeMap::from([
            (line(), Revision::first().next()),
            (detail(), Revision::first().next()),
        ]);
        assert_eq!(
            c.decide(chosen("down"), &now),
            Err(ChangeRequestError::NotAnOption(choice("down")))
        );
        assert!(c.decided_expects().is_empty());
        assert!(c.awaits_decision());
        c.decide(chosen("to even"), &now).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Decided);
        assert!(!c.awaits_decision());
        assert_eq!(
            c.decided_expects(),
            &BTreeMap::from([(
                detail(),
                Expectation {
                    kind: ExpectKind::Target,
                    recorded: Revision::first().next()
                }
            )])
        );
        // What it was opened with keeps its revision: it holds whatever is decided.
        assert_eq!(c.expects()[&line()], change_at_first());
        assert_eq!(c.expectations().count(), 2);
    }

    #[test]
    fn an_option_may_name_a_target_or_a_keep_and_records_the_kind() {
        let q = || {
            Crux::new(
                CruxKind::Conflict,
                vec![
                    CruxOption::new(
                        choice("go"),
                        None,
                        BTreeMap::from([(detail(), ExpectKind::Target)]),
                        None,
                    ),
                    CruxOption::new(
                        choice("stay"),
                        None,
                        BTreeMap::from([(detail(), ExpectKind::Keep)]),
                        None,
                    ),
                ],
                None,
                None,
                None,
            )
            .unwrap()
        };
        let now = BTreeMap::from([(detail(), Revision::first())]);
        for (c, kind) in [("go", ExpectKind::Target), ("stay", ExpectKind::Keep)] {
            let mut request = cr(Some(q()));
            request.decide(chosen(c), &now).unwrap();
            assert_eq!(request.decided_expects()[&detail()].kind, kind);
        }
    }

    #[test]
    fn an_answer_is_given_once_and_never_rewritten() {
        let mut c = person();
        let now = BTreeMap::from([(detail(), Revision::first().next())]);
        let first = chosen("to even");
        c.decide(first.clone(), &now).unwrap();
        let asked = c.decided_expects().clone();
        // Any other answer, of any kind, is refused, and the first stays whole.
        let worded = answer(Decision::Worded {
            wording: prose("round half away from zero"),
            reason: None,
        });
        let free = answer(Decision::Unspecified {
            keep: vec![prose("a total is whole cents")],
        });
        for again in [chosen("up"), chosen("to even"), worded, free] {
            assert_eq!(
                c.decide(again, &BTreeMap::new()),
                Err(ChangeRequestError::AlreadyAnswered)
            );
        }
        assert_eq!(c.crux().and_then(Crux::answer), Some(&first));
        assert_eq!(c.decided_expects(), &asked);
        assert_eq!(c.state(), ChangeRequestState::Decided);
    }

    #[test]
    fn an_answer_keeps_its_decision_basis_maker_and_time() {
        let mut c = person();
        let given = Answer::new(
            Decision::Chosen(Reasoned::new(choice("up"), None)),
            Basis::Policy {
                line: PolicyLineId::try_from("P1").unwrap(),
                policy_revision: Revision::first().next(),
            },
            DecisionMaker::role("orchestrator").unwrap(),
            Timestamp::from_millis(42),
        );
        c.decide(given.clone(), &BTreeMap::new()).unwrap();
        let kept = c.crux().and_then(Crux::answer).unwrap();
        assert_eq!(kept, &given);
        assert_eq!(kept.by().to_string(), "role orchestrator");
        assert_eq!(kept.at().millis(), 42);
        assert!(matches!(kept.basis(), Basis::Policy { .. }));
    }

    #[test]
    fn a_target_gone_when_decided_is_not_recorded() {
        let mut c = person();
        c.decide(chosen("to even"), &BTreeMap::new()).unwrap();
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn what_a_change_request_could_come_to_name_includes_every_option() {
        let c = person();
        let mut kinds: Vec<_> = c.possible_kinds().collect();
        kinds.sort();
        assert_eq!(
            kinds,
            [(line(), ExpectKind::Target), (detail(), ExpectKind::Target)]
        );
    }

    #[test]
    fn a_stored_expectation_must_be_one_the_decision_asks() {
        let recorded = BTreeMap::from([(detail(), change_at_first())]);
        let q = || Some(crux(CruxKind::Ambiguity).unwrap());
        let resolved = || vec![Transition::Resolved(done())];
        // No decision, or a decision whose option asks nothing: the expectation is unasked.
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                None,
                recorded.clone(),
                Vec::new(),
                Vec::new()
            ),
            Err(ChangeRequestError::UnaskedExpectation(detail()))
        );
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                Some(chosen("up")),
                recorded.clone(),
                Vec::new(),
                Vec::new()
            ),
            Err(ChangeRequestError::UnaskedExpectation(detail()))
        );
        // The target is asked, but not with this kind.
        let keep = BTreeMap::from([(
            detail(),
            Expectation {
                kind: ExpectKind::Keep,
                recorded: Revision::first(),
            },
        )]);
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                Some(chosen("to even")),
                keep,
                Vec::new(),
                Vec::new()
            ),
            Err(ChangeRequestError::UnaskedExpectation(detail()))
        );
        assert!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                Some(chosen("to even")),
                recorded,
                Vec::new(),
                resolved()
            )
            .is_ok()
        );
        // Resolved as done, the Crux has its answer; resolved as anything else, it need not.
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                None,
                BTreeMap::new(),
                Vec::new(),
                resolved()
            ),
            Err(ChangeRequestError::Undecided)
        );
        assert!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                None,
                BTreeMap::new(),
                Vec::new(),
                vec![Transition::Resolved(resolution(ResolutionKind::WontDo))]
            )
            .is_ok()
        );
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(None),
                Some(chosen("up")),
                BTreeMap::new(),
                Vec::new(),
                Vec::new()
            ),
            Err(ChangeRequestError::NoCrux)
        );
        // A stored answer is checked against the options like a new one.
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                Some(chosen("down")),
                BTreeMap::new(),
                Vec::new(),
                Vec::new()
            ),
            Err(ChangeRequestError::NotAnOption(choice("down")))
        );
    }

    #[test]
    fn a_stored_history_alternates_between_resolving_and_reopening() {
        let restore = |history| {
            ChangeRequest::restore(
                id(1),
                draft(None),
                None,
                BTreeMap::new(),
                Vec::new(),
                history,
            )
        };
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
        assert_eq!(reopened_once.state(), ChangeRequestState::Open);
        assert_eq!(reopened_once.history().len(), 2);
    }

    #[test]
    fn a_model_question_cannot_be_left_unspecified() {
        let mut c = cr(Some(crux(CruxKind::Model).unwrap()));
        assert_eq!(
            c.decide(
                answer(Decision::Unspecified { keep: Vec::new() }),
                &BTreeMap::new()
            ),
            Err(ChangeRequestError::UnspecifiedNotAllowed("model"))
        );
        // Refused, it did not take the answer.
        assert!(c.awaits_decision());
    }

    #[test]
    fn a_change_request_without_a_crux_has_nothing_to_decide() {
        let mut c = cr(None);
        assert_eq!(
            c.decide(chosen("up"), &BTreeMap::new()),
            Err(ChangeRequestError::NoCrux)
        );
        c.resolve(done()).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn a_crux_resolves_as_done_once_decided_and_then_takes_no_decision() {
        let mut c = person();
        assert_eq!(c.resolve(done()), Err(ChangeRequestError::Undecided));
        assert_eq!(c.state(), ChangeRequestState::Open);
        c.decide(chosen("to even"), &BTreeMap::new()).unwrap();
        c.resolve(done()).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
        assert_eq!(c.resolve(done()), Err(ChangeRequestError::AlreadyResolved));
        assert_eq!(
            c.decide(chosen("up"), &BTreeMap::new()),
            Err(ChangeRequestError::AlreadyResolved)
        );
    }

    #[test]
    fn a_request_that_is_not_done_may_end_with_its_question_open() {
        for kind in [ResolutionKind::WontDo, ResolutionKind::Invalid] {
            let mut c = person();
            c.resolve(resolution(kind)).unwrap();
            assert_eq!(c.state(), ChangeRequestState::Resolved);
            assert!(!c.awaits_decision());
            assert_eq!(c.resolution().map(|r| r.kind()), Some(kind));
        }
    }

    #[test]
    fn a_resolution_keeps_its_reason_maker_and_time() {
        let mut c = cr(None);
        c.resolve(done()).unwrap();
        let r = c.resolution().unwrap();
        assert_eq!(r.reason().as_str(), "settled");
        assert_eq!(r.by().to_string(), "person alice");
        assert_eq!(r.at().millis(), 9);
    }

    #[test]
    fn reopening_keeps_the_history_and_takes_more_work_and_another_resolution() {
        let mut c = cr(None);
        assert_eq!(c.reopen(reopening()), Err(ChangeRequestError::NotResolved));
        c.resolve(done()).unwrap();
        assert_eq!(
            c.record_work(Footprint::new(2, Vec::new())),
            Err(ChangeRequestError::AlreadyResolved)
        );
        c.reopen(reopening()).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Open);
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
        let mut c = person();
        c.assume(assumption("a half is a tie")).unwrap();
        c.assume(assumption("a total is whole cents")).unwrap();
        let said: Vec<&str> = c
            .assumptions()
            .iter()
            .map(|a| a.statement().as_str())
            .collect();
        assert_eq!(said, ["a half is a tie", "a total is whole cents"]);
        assert_eq!(c.assumptions()[0].evidence().len(), 2);
        // Deciding does not touch them.
        c.decide(chosen("up"), &BTreeMap::new()).unwrap();
        assert_eq!(c.assumptions().len(), 2);
        assert_eq!(c.assumptions()[0], assumption("a half is a tie"));
    }

    #[test]
    fn a_resolved_change_request_takes_no_assumption() {
        let mut c = cr(None);
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
        let mut d = draft(None);
        d.assumptions = vec![assumption("a half is a tie")];
        let c = ChangeRequest::open(id(1), d);
        assert_eq!(c.assumptions().len(), 1);
    }
}
