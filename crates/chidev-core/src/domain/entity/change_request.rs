//! The change request: a request that the definitions change (or a confirmation that they need not). Its lines are
//! always to follow; a Crux, when it has one, is a question a person settles, and the option chosen may ask more
//! definitions to change.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::choice::Choice;
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned};
use crate::domain::vo::finding::FindingRef;
use crate::domain::vo::id::ChangeRequestId;
use crate::domain::vo::intent::{Intent, Issuer};
use crate::domain::vo::prose::Prose;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;
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
    /// The definitions have not followed yet: `unchanged` are still as they were, `gone` are no more.
    #[error("the change is not applied yet: {} unchanged, {} gone", .unchanged.len(), .gone.len())]
    NotApplied {
        /// The definitions still at the revision they were recorded at, in order.
        unchanged: Vec<Target>,
        /// The definitions no asset of the Project has any more, in order.
        gone: Vec<Target>,
    },
    /// A stored change was not asked by the decision stored with it.
    #[error("{0:?} is not a change the decision asks")]
    UnaskedChange(Target),
}

/// The question a person settles for a change request: its kind, the options with what each changes, the default
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

    /// The definitions the decision asks to change besides the lines of the change request: those of the option
    /// chosen. The person's own wording asks the lines to change (already listed) and no more, leaving it
    /// unspecified asks nothing, and no decision yet asks nothing.
    pub fn decided_changes(&self) -> Vec<Target> {
        let chosen = match &self.decision {
            Some(Decision::Chosen(c)) => self.option(c.choice()),
            _ => None,
        };
        chosen.map_or_else(Vec::new, |o| o.changes().iter().copied().collect())
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
    /// default, if it has one. The lines are to follow all the same.
    Open,
    /// Its Crux is decided; the definitions may still have to follow.
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
    /// Whether it means to change the behaviour the specification promises.
    pub intent: Intent,
    /// The broader change request it is a part of: a change request is often split.
    pub parent: Option<ChangeRequestId>,
    /// Who raised it.
    pub issuer: Option<Issuer>,
    /// The definitions it asks to change, with the revision each has now.
    pub lines: BTreeMap<Target, Revision>,
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
/// satisfy. It means either to change the behaviour the specification promises or to keep it; it may be part of a
/// broader one. Its lines, the definitions it asks to change, are always to follow. It may carry a Crux, a question
/// a person settles; the option chosen may ask more definitions to change, which are to follow too. The revision of
/// each definition is kept, so that whether it has followed is seen by comparing revisions; once all have, and the
/// Crux is decided, it is resolved and its record stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    id: ChangeRequestId,
    title: Title,
    situation: Option<Prose>,
    intent: Intent,
    parent: Option<ChangeRequestId>,
    issuer: Option<Issuer>,
    lines: BTreeMap<Target, Revision>,
    crux: Option<Crux>,
    /// The definitions the chosen option asks to change, with the revision each had when it was decided.
    decided_changes: BTreeMap<Target, Revision>,
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
            lines: draft.lines,
            crux,
            decided_changes: BTreeMap::new(),
            addresses: distinct(draft.addresses),
            resolved: false,
        }
    }

    /// A change request as a store kept it: its Crux with the decision taken, the changes that decision asked with
    /// the revisions recorded, and whether it is resolved. A resolved one with a Crux has a decision, and every
    /// change recorded is one the decision asks.
    pub(crate) fn restore(
        id: ChangeRequestId,
        draft: ChangeRequestDraft,
        decision: Option<Decision>,
        decided_changes: BTreeMap<Target, Revision>,
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
        let asked: Vec<Target> = crux.iter().flat_map(|c| c.decided_changes()).collect();
        if let Some(t) = decided_changes.keys().find(|t| !asked.contains(t)) {
            return Err(ChangeRequestError::UnaskedChange(*t));
        }
        Ok(ChangeRequest {
            id,
            title: draft.title,
            situation: draft.situation,
            intent: draft.intent,
            parent: draft.parent.filter(|p| *p != id),
            issuer: draft.issuer,
            lines: draft.lines,
            crux,
            decided_changes,
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

    /// Whether it means to change the behaviour.
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

    /// The definitions it asks to change, with the revision each had when it was opened. They are to follow
    /// whatever its Crux decides.
    pub fn lines(&self) -> &BTreeMap<Target, Revision> {
        &self.lines
    }

    /// The definitions the chosen option of its Crux asks to change, with the revision each had when the Crux was
    /// last decided. Empty until decided, and when the decision asks nothing more than the lines.
    pub fn decided_changes(&self) -> &BTreeMap<Target, Revision> {
        &self.decided_changes
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

    /// Takes the person's decision on its Crux; a later one replaces an earlier one, and the changes the earlier
    /// one asked, until it is resolved. Each definition the option chosen asks to change takes the revision in
    /// `now`, the one it has as the decision is taken (one missing from `now` is gone: it is not recorded, and the
    /// caller reports it). The lines keep the revisions they were opened with.
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
        self.decided_changes = crux
            .decided_changes()
            .into_iter()
            .filter_map(|t| now.get(&t).map(|r| (t, *r)))
            .collect();
        Ok(())
    }

    /// Closes the record. Whether the definitions have followed is read against the Project, so the caller checks
    /// it first (`service::change_request::resolve`). A Crux must be decided first.
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
    use std::collections::BTreeSet;

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

    fn crux(kind: CruxKind, default: Option<&str>) -> Result<Crux, ChangeRequestError> {
        Crux::new(
            kind,
            vec![
                CruxOption::new(choice("up"), None, BTreeSet::new(), None),
                CruxOption::new(
                    choice("to even"),
                    Some(prose("C1: a total is rounded half to even")),
                    BTreeSet::from([detail()]),
                    Some(prose("the rounding tests change")),
                ),
                CruxOption::new(choice("keep"), None, BTreeSet::new(), None),
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
                lines: BTreeMap::from([(line(), Revision::first())]),
                crux,
                addresses: Vec::new(),
            },
        )
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
        assert!(c.decided_changes().is_empty());
        assert_eq!(c.intent(), Intent::ChangeBehavior);
        assert_eq!(c.parent(), Some(id(9)));
        assert_eq!(c.issuer().unwrap().as_str(), "designer");
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
        let d = ChangeRequestDraft {
            title: Title::try_from("T").unwrap(),
            situation: None,
            intent: Intent::KeepBehavior,
            parent: None,
            issuer: None,
            lines: BTreeMap::new(),
            crux: None,
            addresses: vec![a.clone(), b.clone(), a.clone()],
        };
        assert_eq!(ChangeRequest::open(id(1), d).addresses(), [a, b]);
    }

    #[test]
    fn a_change_request_is_not_its_own_parent() {
        let mut d = ChangeRequestDraft {
            title: Title::try_from("T").unwrap(),
            situation: None,
            intent: Intent::KeepBehavior,
            parent: Some(id(1)),
            issuer: None,
            lines: BTreeMap::new(),
            crux: None,
            addresses: Vec::new(),
        };
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
                CruxOption::new(choice("a"), None, BTreeSet::new(), None),
                CruxOption::new(choice("a"), None, BTreeSet::new(), None),
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
    fn a_decision_records_the_revisions_of_the_changes_the_option_asks_and_leaves_the_lines() {
        let mut c = person();
        let now = BTreeMap::from([
            (line(), Revision::first().next()),
            (detail(), Revision::first().next()),
        ]);
        assert_eq!(
            c.decide(chosen("down"), &now),
            Err(ChangeRequestError::NotAnOption(choice("down")))
        );
        assert!(c.decided_changes().is_empty());
        c.decide(chosen("to even"), &now).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Decided);
        assert!(!c.awaits_decision());
        assert_eq!(
            c.decided_changes(),
            &BTreeMap::from([(detail(), Revision::first().next())])
        );
        // The lines keep the revision they were opened with: they are to follow whatever is decided.
        assert_eq!(c.lines()[&line()], Revision::first());
    }

    #[test]
    fn a_later_decision_replaces_the_changes_of_the_earlier_one() {
        let mut c = person();
        let now = BTreeMap::from([(detail(), Revision::first().next())]);
        c.decide(chosen("to even"), &now).unwrap();
        assert_eq!(c.decided_changes().len(), 1);
        c.decide(chosen("up"), &now).unwrap();
        assert!(c.decided_changes().is_empty());
        c.decide(chosen("to even"), &now).unwrap();
        let worded = Decision::Worded {
            wording: prose("round half away from zero"),
            reason: None,
        };
        c.decide(worded, &now).unwrap();
        assert!(c.decided_changes().is_empty());
        c.decide(chosen("to even"), &now).unwrap();
        let free = Decision::Unspecified {
            keep: vec![prose("a total is whole cents")],
        };
        c.decide(free, &now).unwrap();
        assert!(c.decided_changes().is_empty());
        assert_eq!(c.state(), ChangeRequestState::Decided);
    }

    #[test]
    fn a_change_gone_when_decided_is_not_recorded() {
        let mut c = person();
        c.decide(chosen("to even"), &BTreeMap::new()).unwrap();
        assert!(c.decided_changes().is_empty());
    }

    #[test]
    fn a_stored_change_must_be_one_the_decision_asks() {
        let draft = |crux| ChangeRequestDraft {
            title: Title::try_from("T").unwrap(),
            situation: None,
            intent: Intent::ChangeBehavior,
            parent: None,
            issuer: None,
            lines: BTreeMap::new(),
            crux,
            addresses: Vec::new(),
        };
        let recorded = BTreeMap::from([(detail(), Revision::first())]);
        let crux = || Some(crux(CruxKind::Ambiguity, Some("up")).unwrap());
        // No decision, or a decision whose option asks nothing: the change is unasked.
        assert_eq!(
            ChangeRequest::restore(id(1), draft(crux()), None, recorded.clone(), false),
            Err(ChangeRequestError::UnaskedChange(detail()))
        );
        assert_eq!(
            ChangeRequest::restore(
                id(1),
                draft(crux()),
                Some(chosen("up")),
                recorded.clone(),
                false
            ),
            Err(ChangeRequestError::UnaskedChange(detail()))
        );
        assert!(
            ChangeRequest::restore(
                id(1),
                draft(crux()),
                Some(chosen("to even")),
                recorded,
                true
            )
            .is_ok()
        );
        assert_eq!(
            ChangeRequest::restore(id(1), draft(crux()), None, BTreeMap::new(), true),
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
