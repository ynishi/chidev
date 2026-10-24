//! The change request: a request that the definitions change (or a confirmation that they need not). It says what
//! it expects of the targets it points at (change, remove or keep); a Crux, when it has one, is a question a person
//! settles, and the option chosen may expect more of other targets.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::choice::Choice;
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned};
use crate::domain::vo::finding::FindingRef;
use crate::domain::vo::id::ChangeRequestId;
use crate::domain::vo::intent::{Intent, Issuer};
use crate::domain::vo::prose::Prose;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::{ExpectKind, Expectation, Target};
use crate::domain::vo::title::Title;

/// Why a change request cannot be opened, decided or resolved. Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChangeRequestError {
    /// A Crux has no option to choose.
    #[error("a crux needs at least one option")]
    NoOption,
    /// Two options are the same choice.
    #[error("the option `{0}` is given twice")]
    DuplicateOption(Choice),
    /// The kind of the Crux takes no default (a conflict, a model question): the person decides.
    #[error("a {0} question takes no default; it stays open until the person decides")]
    DefaultNotAllowed(&'static str),
    /// A default, a recommendation or a decision names a choice that is not an option.
    #[error("`{0}` is not one of the options")]
    NotAnOption(Choice),
    /// A model question is about words; it cannot be left unspecified.
    #[error("a {0} question cannot be left unspecified")]
    UnspecifiedNotAllowed(&'static str),
    /// The change request has no Crux; there is no question for a person to decide.
    #[error("the change request has no crux to decide")]
    NoCrux,
    /// The change request is resolved; it takes no more decisions.
    #[error("the change request is resolved")]
    AlreadyResolved,
    /// The Crux has no decision yet.
    #[error("the crux has no decision yet")]
    Undecided,
    /// The targets have not followed yet: `unchanged` were to change and are still as they were, `gone` were to
    /// change and are no more, `not_removed` were to go and are still there.
    #[error(
        "the change is not applied yet: {} unchanged, {} gone, {} not removed",
        .unchanged.len(), .gone.len(), .not_removed.len()
    )]
    NotApplied {
        /// The targets to change that are still at the revision they were recorded at, in order.
        unchanged: Vec<Target>,
        /// The targets to change that no asset of the Project has any more, in order.
        gone: Vec<Target>,
        /// The targets to remove that are still in the Project, in order.
        not_removed: Vec<Target>,
    },
    /// Targets the change request expected to stay as they are have changed: it cannot close a contradiction.
    #[error("{} target(s) it expected to keep have moved", .moved.len())]
    KeepMoved {
        /// The targets to keep that are not at the revision they were recorded at (or are gone), in order.
        moved: Vec<Target>,
    },
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

/// The question a person settles for a change request: its kind, the options with what each expects, the default
/// work goes on with, the recommendation, and the decision once taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crux {
    kind: CruxKind,
    options: Vec<CruxOption>,
    default: Option<Reasoned>,
    recommended: Option<Reasoned>,
    decision: Option<Decision>,
}

impl Crux {
    /// An undecided question: one option at least, no choice twice, the default (only for a kind that has one) and
    /// the recommendation among the options.
    pub fn new(
        kind: CruxKind,
        options: Vec<CruxOption>,
        default: Option<Reasoned>,
        recommended: Option<Reasoned>,
    ) -> Result<Crux, ChangeRequestError> {
        if options.is_empty() {
            return Err(ChangeRequestError::NoOption);
        }
        for (k, o) in options.iter().enumerate() {
            if options[..k].iter().any(|p| p.choice() == o.choice()) {
                return Err(ChangeRequestError::DuplicateOption(o.choice().clone()));
            }
        }
        let is_option = |c: &Choice| options.iter().any(|o| o.choice() == c);
        if let Some(d) = &default {
            if !kind.allows_default() {
                return Err(ChangeRequestError::DefaultNotAllowed(kind.word()));
            }
            if !is_option(d.choice()) {
                return Err(ChangeRequestError::NotAnOption(d.choice().clone()));
            }
        }
        if let Some(r) = recommended.as_ref().filter(|r| !is_option(r.choice())) {
            return Err(ChangeRequestError::NotAnOption(r.choice().clone()));
        }
        Ok(Crux {
            kind,
            options,
            default,
            recommended,
            decision: None,
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

    /// The option work goes on with until the person decides.
    pub fn default(&self) -> Option<&Reasoned> {
        self.default.as_ref()
    }

    /// The option recommended.
    pub fn recommended(&self) -> Option<&Reasoned> {
        self.recommended.as_ref()
    }

    /// The decision, once taken.
    pub fn decision(&self) -> Option<&Decision> {
        self.decision.as_ref()
    }

    /// What the decision expects of targets besides the expectations of the change request: those of the option
    /// chosen. The person's own wording expects nothing more, leaving it unspecified expects nothing, and no
    /// decision yet expects nothing.
    pub fn decided_expects(&self) -> Vec<(Target, ExpectKind)> {
        let chosen = match &self.decision {
            Some(Decision::Chosen(c)) => self.option(c.choice()),
            _ => None,
        };
        chosen.map_or_else(Vec::new, |o| {
            o.expects().iter().map(|(t, k)| (*t, *k)).collect()
        })
    }

    /// Checks `decision` against the options and the kind, and takes it.
    fn decide(&mut self, decision: Decision) -> Result<(), ChangeRequestError> {
        match &decision {
            Decision::Chosen(c) if self.option(c.choice()).is_none() => {
                return Err(ChangeRequestError::NotAnOption(c.choice().clone()));
            }
            Decision::Unspecified { .. } if !self.kind.allows_unspecified() => {
                return Err(ChangeRequestError::UnspecifiedNotAllowed(self.kind.word()));
            }
            _ => {}
        }
        self.decision = Some(decision);
        Ok(())
    }
}

/// Where a change request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRequestState {
    /// Not resolved, and its Crux (if any) not decided: the person is awaited, and work goes on with the Crux's
    /// default, if it has one. What it expects is to hold all the same.
    Open,
    /// Its Crux is decided; the targets may still have to follow.
    Decided,
    /// The change is applied: the record is closed.
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
    /// What it expects of targets, one expectation for each, with the revision each target has now.
    pub expects: BTreeMap<Target, Expectation>,
    /// The question for a person, when there is one. A Crux given here is undecided.
    pub crux: Option<Crux>,
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
/// satisfy. It says what it expects of each target it points at: to change, to go, or to stay as it is. Each
/// expectation keeps the revision the target had, so that whether it is met is seen by comparing revisions. It may
/// be part of a broader request. It may carry a Crux, a question a person settles; the option chosen may expect
/// more of other targets, recorded when it is decided. Once every expectation to change or remove is met, no
/// expectation to keep has been broken, the Crux is decided, and every part of it is resolved, it is resolved and its
/// record stands. Its intent (whether it means to change the behaviour) is a record: no rule of resolving reads it.
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
    addresses: Vec<FindingRef>,
    resolved: bool,
}

impl ChangeRequest {
    /// An open change request. It cannot be its own parent.
    pub fn open(id: ChangeRequestId, draft: ChangeRequestDraft) -> ChangeRequest {
        let crux = draft.crux.map(|mut crux| {
            crux.decision = None;
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
            addresses: distinct(draft.addresses),
            resolved: false,
        }
    }

    /// A change request as a store kept it: its Crux with the decision taken, the expectations that decision asked
    /// with the revisions recorded, and whether it is resolved. A resolved one with a Crux has a decision, and every
    /// expectation recorded is one the decision asks. Whether the parts of a resolved one are resolved is not
    /// asked here: they are other records.
    pub(crate) fn restore(
        id: ChangeRequestId,
        draft: ChangeRequestDraft,
        decision: Option<Decision>,
        decided_expects: BTreeMap<Target, Expectation>,
        resolved: bool,
    ) -> Result<ChangeRequest, ChangeRequestError> {
        let crux = match draft.crux {
            None if decision.is_some() => return Err(ChangeRequestError::NoCrux),
            None => None,
            Some(mut crux) => {
                crux.decision = None;
                if let Some(d) = decision {
                    crux.decide(d)?;
                }
                if resolved && crux.decision.is_none() {
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
            addresses: distinct(draft.addresses),
            resolved,
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

    /// What it expects of targets, with the revision each had when it was opened. They hold whatever its Crux
    /// decides.
    pub fn expects(&self) -> &BTreeMap<Target, Expectation> {
        &self.expects
    }

    /// What the chosen option of its Crux expects of targets, with the revision each had when the Crux was last
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

    /// The findings it deals with: a finding listed here is addressed while the change request is unresolved and
    /// decided once it is resolved (see `Finding::state_in`).
    pub fn addresses(&self) -> &[FindingRef] {
        &self.addresses
    }

    /// Its Crux, when it has a question for a person.
    pub fn crux(&self) -> Option<&Crux> {
        self.crux.as_ref()
    }

    /// Whether it is unresolved and its Crux is not decided yet: a person is awaited.
    pub fn awaits_decision(&self) -> bool {
        !self.resolved && self.crux.as_ref().is_some_and(|c| c.decision.is_none())
    }

    /// Where it stands.
    pub fn state(&self) -> ChangeRequestState {
        if self.resolved {
            return ChangeRequestState::Resolved;
        }
        match self.crux().and_then(Crux::decision) {
            Some(_) => ChangeRequestState::Decided,
            None => ChangeRequestState::Open,
        }
    }

    /// Takes the person's decision on its Crux; a later one replaces an earlier one, and the expectations the
    /// earlier one asked, until it is resolved. Each target the option chosen expects something of takes the
    /// revision in `now`, the one it has as the decision is taken (one missing from `now` is gone: it is not
    /// recorded). The expectations of the change request keep the revisions they were opened with.
    pub fn decide(
        &mut self,
        decision: Decision,
        now: &BTreeMap<Target, Revision>,
    ) -> Result<(), ChangeRequestError> {
        if self.resolved {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        let Some(crux) = &mut self.crux else {
            return Err(ChangeRequestError::NoCrux);
        };
        crux.decide(decision)?;
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

    /// Closes the record. Whether the targets have followed is read against the Project, and whether its parts are
    /// resolved against the other change requests, so the caller checks both first
    /// (`service::change_request::resolve`). A Crux must be decided first.
    pub fn resolve(&mut self) -> Result<(), ChangeRequestError> {
        if self.resolved {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        if self.awaits_decision() {
            return Err(ChangeRequestError::Undecided);
        }
        self.resolved = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::{ClauseId, test_uuid};

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
            kind: ExpectKind::Change,
            recorded: Revision::first(),
        }
    }

    fn crux(kind: CruxKind, default: Option<&str>) -> Result<Crux, ChangeRequestError> {
        Crux::new(
            kind,
            vec![
                CruxOption::new(choice("up"), None, BTreeMap::new(), None),
                CruxOption::new(
                    choice("to even"),
                    Some(prose("C1: a total is rounded half to even")),
                    BTreeMap::from([(detail(), ExpectKind::Change)]),
                    Some(prose("the rounding tests change")),
                ),
                CruxOption::new(choice("keep"), None, BTreeMap::new(), None),
            ],
            default.map(|d| Reasoned::new(choice(d), Some(prose("the literal reading")))),
            Some(Reasoned::new(choice("to even"), None)),
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
            addresses: Vec::new(),
        }
    }

    fn person() -> ChangeRequest {
        cr(Some(crux(CruxKind::Ambiguity, Some("up")).unwrap()))
    }

    fn chosen(c: &str) -> Decision {
        Decision::Chosen(Reasoned::new(choice(c), Some(prose("fairer"))))
    }

    #[test]
    fn a_new_change_request_is_open_and_keeps_what_it_was_opened_with() {
        let c = person();
        assert_eq!(c.state(), ChangeRequestState::Open);
        assert!(c.awaits_decision());
        assert!(c.decided_expects().is_empty());
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
    fn a_crux_has_distinct_options_and_its_default_and_recommendation_among_them() {
        assert_eq!(
            Crux::new(CruxKind::Ambiguity, Vec::new(), None, None),
            Err(ChangeRequestError::NoOption)
        );
        let twice = Crux::new(
            CruxKind::Ambiguity,
            vec![
                CruxOption::new(choice("a"), None, BTreeMap::new(), None),
                CruxOption::new(choice("a"), None, BTreeMap::new(), None),
            ],
            None,
            None,
        );
        assert_eq!(twice, Err(ChangeRequestError::DuplicateOption(choice("a"))));
        assert_eq!(
            crux(CruxKind::Ambiguity, Some("down")),
            Err(ChangeRequestError::NotAnOption(choice("down")))
        );
    }

    #[test]
    fn a_conflict_or_a_model_question_takes_no_default() {
        assert_eq!(
            crux(CruxKind::Conflict, Some("up")),
            Err(ChangeRequestError::DefaultNotAllowed("conflict"))
        );
        assert!(crux(CruxKind::Model, None).is_ok());
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
        c.decide(chosen("to even"), &now).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Decided);
        assert!(!c.awaits_decision());
        assert_eq!(
            c.decided_expects(),
            &BTreeMap::from([(
                detail(),
                Expectation {
                    kind: ExpectKind::Change,
                    recorded: Revision::first().next()
                }
            )])
        );
        // What it was opened with keeps its revision: it holds whatever is decided.
        assert_eq!(c.expects()[&line()], change_at_first());
        assert_eq!(c.expectations().count(), 2);
    }

    #[test]
    fn an_option_may_expect_a_removal_or_a_keep_and_records_the_kind() {
        let q = Crux::new(
            CruxKind::Conflict,
            vec![
                CruxOption::new(
                    choice("go"),
                    None,
                    BTreeMap::from([(detail(), ExpectKind::Remove)]),
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
        )
        .unwrap();
        let mut c = cr(Some(q));
        let now = BTreeMap::from([(detail(), Revision::first())]);
        c.decide(chosen("go"), &now).unwrap();
        assert_eq!(c.decided_expects()[&detail()].kind, ExpectKind::Remove);
        c.decide(chosen("stay"), &now).unwrap();
        assert_eq!(c.decided_expects()[&detail()].kind, ExpectKind::Keep);
    }

    #[test]
    fn a_later_decision_replaces_the_expectations_of_the_earlier_one() {
        let mut c = person();
        let now = BTreeMap::from([(detail(), Revision::first().next())]);
        c.decide(chosen("to even"), &now).unwrap();
        assert_eq!(c.decided_expects().len(), 1);
        c.decide(chosen("up"), &now).unwrap();
        assert!(c.decided_expects().is_empty());
        c.decide(chosen("to even"), &now).unwrap();
        let worded = Decision::Worded {
            wording: prose("round half away from zero"),
            reason: None,
        };
        c.decide(worded, &now).unwrap();
        assert!(c.decided_expects().is_empty());
        c.decide(chosen("to even"), &now).unwrap();
        let free = Decision::Unspecified {
            keep: vec![prose("a total is whole cents")],
        };
        c.decide(free, &now).unwrap();
        assert!(c.decided_expects().is_empty());
        assert_eq!(c.state(), ChangeRequestState::Decided);
    }

    #[test]
    fn a_target_gone_when_decided_is_not_recorded() {
        let mut c = person();
        c.decide(chosen("to even"), &BTreeMap::new()).unwrap();
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn what_a_change_request_could_come_to_expect_includes_every_option() {
        let c = person();
        let mut kinds: Vec<_> = c.possible_kinds().collect();
        kinds.sort();
        assert_eq!(
            kinds,
            [(line(), ExpectKind::Change), (detail(), ExpectKind::Change)]
        );
    }

    #[test]
    fn a_stored_expectation_must_be_one_the_decision_asks() {
        let recorded = BTreeMap::from([(detail(), change_at_first())]);
        let q = || Some(crux(CruxKind::Ambiguity, Some("up")).unwrap());
        // No decision, or a decision whose option asks nothing: the expectation is unasked.
        assert_eq!(
            ChangeRequest::restore(id(1), draft(q()), None, recorded.clone(), false),
            Err(ChangeRequestError::UnaskedExpectation(detail()))
        );
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(q()),
                Some(chosen("up")),
                recorded.clone(),
                false
            ),
            Err(ChangeRequestError::UnaskedExpectation(detail()))
        );
        // The target is asked, but not with this kind.
        let removal = BTreeMap::from([(
            detail(),
            Expectation {
                kind: ExpectKind::Remove,
                recorded: Revision::first(),
            },
        )]);
        assert_eq!(
            ChangeRequest::restore(id(1), draft(q()), Some(chosen("to even")), removal, false),
            Err(ChangeRequestError::UnaskedExpectation(detail()))
        );
        assert!(
            ChangeRequest::restore(id(1), draft(q()), Some(chosen("to even")), recorded, true)
                .is_ok()
        );
        assert_eq!(
            ChangeRequest::restore(id(1), draft(q()), None, BTreeMap::new(), true),
            Err(ChangeRequestError::Undecided)
        );
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(None),
                Some(chosen("up")),
                BTreeMap::new(),
                false
            ),
            Err(ChangeRequestError::NoCrux)
        );
    }

    #[test]
    fn a_model_question_cannot_be_left_unspecified() {
        let mut c = cr(Some(crux(CruxKind::Model, None).unwrap()));
        assert_eq!(
            c.decide(Decision::Unspecified { keep: Vec::new() }, &BTreeMap::new()),
            Err(ChangeRequestError::UnspecifiedNotAllowed("model"))
        );
    }

    #[test]
    fn a_change_request_without_a_crux_has_nothing_to_decide() {
        let mut c = cr(None);
        assert_eq!(
            c.decide(chosen("up"), &BTreeMap::new()),
            Err(ChangeRequestError::NoCrux)
        );
        c.resolve().unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn a_crux_resolves_once_decided_and_then_takes_no_decision() {
        let mut c = person();
        assert_eq!(c.resolve(), Err(ChangeRequestError::Undecided));
        c.decide(chosen("to even"), &BTreeMap::new()).unwrap();
        c.resolve().unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
        assert_eq!(c.resolve(), Err(ChangeRequestError::AlreadyResolved));
        assert_eq!(
            c.decide(chosen("up"), &BTreeMap::new()),
            Err(ChangeRequestError::AlreadyResolved)
        );
    }
}
