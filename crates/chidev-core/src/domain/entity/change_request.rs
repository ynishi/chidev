//! The change request: a request that the definitions change (or a confirmation that they need not), resolved by
//! the machine or, through a Crux, by a person.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::choice::Choice;
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned};
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
    /// The change request is resolved by the machine; there is no question for a person to decide.
    #[error("the change request has no crux to decide")]
    NoCrux,
    /// The change request is resolved; it takes no more decisions.
    #[error("the change request is resolved")]
    AlreadyResolved,
    /// The Crux has no decision yet.
    #[error("the crux has no decision yet")]
    Undecided,
    /// The definitions have not followed yet: these are still as they were, or gone.
    #[error("the change is not applied yet: {0:?}")]
    NotApplied(Vec<Target>),
}

/// The question a person settles for a change request the machine cannot resolve: its kind, the options with what
/// each changes, the default work goes on with, the recommendation, and the decision once taken.
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

    /// Whether the decision asks the definitions to change: an option other than the default that changes them, or
    /// the person's own wording. The default, an option that changes nothing, leaving it unspecified, or no
    /// decision yet asks for no change.
    pub fn asks_change(&self) -> bool {
        match &self.decision {
            None | Some(Decision::Unspecified { .. }) => false,
            Some(Decision::Worded { .. }) => true,
            Some(Decision::Chosen(c)) => {
                self.default.as_ref().map(Reasoned::choice) != Some(c.choice())
                    && self
                        .option(c.choice())
                        .is_some_and(|o| o.change().is_some())
            }
        }
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

/// Who resolves a change request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The machine: the change follows from the definitions, nobody has to choose.
    Machine,
    /// A person, by settling this question.
    Person(Crux),
}

/// Where a change request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRequestState {
    /// Not resolved, and its Crux (if any) not decided: work goes on with the Crux's default, if it has one.
    Open,
    /// Its Crux is decided; the definitions may still have to follow.
    Decided,
    /// The change is applied: the record is closed.
    Resolved,
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
    /// The definitions it is about, with the revision each has now.
    pub lines: BTreeMap<Target, Revision>,
    /// Who resolves it. A Crux given here is undecided.
    pub route: Route,
}

/// A request that the definitions change, or a confirmation that they need not: whatever is there now does not
/// satisfy. It means either to change the behaviour the specification promises or to keep it; it may be part of a
/// broader one; it is resolved by the machine or, through its Crux, by a person. The definitions it is about are
/// kept with a revision each, so that whether they have followed is seen by comparing revisions; once they have,
/// it is resolved and its record stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    id: ChangeRequestId,
    title: Title,
    situation: Option<Prose>,
    intent: Intent,
    parent: Option<ChangeRequestId>,
    issuer: Option<Issuer>,
    lines: BTreeMap<Target, Revision>,
    route: Route,
    resolved: bool,
}

impl ChangeRequest {
    /// An open change request. It cannot be its own parent.
    pub fn open(id: ChangeRequestId, draft: ChangeRequestDraft) -> ChangeRequest {
        let route = match draft.route {
            Route::Person(mut crux) => {
                crux.decision = None;
                Route::Person(crux)
            }
            Route::Machine => Route::Machine,
        };
        ChangeRequest {
            id,
            title: draft.title,
            situation: draft.situation,
            intent: draft.intent,
            parent: draft.parent.filter(|p| *p != id),
            issuer: draft.issuer,
            lines: draft.lines,
            route,
            resolved: false,
        }
    }

    /// A change request as a store kept it: its Crux with the decision taken, and whether it is resolved. A
    /// resolved one with a Crux has a decision.
    pub(crate) fn restore(
        id: ChangeRequestId,
        draft: ChangeRequestDraft,
        decision: Option<Decision>,
        resolved: bool,
    ) -> Result<ChangeRequest, ChangeRequestError> {
        let route = match draft.route {
            Route::Machine if decision.is_some() => return Err(ChangeRequestError::NoCrux),
            Route::Machine => Route::Machine,
            Route::Person(mut crux) => {
                crux.decision = None;
                if let Some(d) = decision {
                    crux.decide(d)?;
                }
                if resolved && crux.decision.is_none() {
                    return Err(ChangeRequestError::Undecided);
                }
                Route::Person(crux)
            }
        };
        Ok(ChangeRequest {
            id,
            title: draft.title,
            situation: draft.situation,
            intent: draft.intent,
            parent: draft.parent.filter(|p| *p != id),
            issuer: draft.issuer,
            lines: draft.lines,
            route,
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

    /// The definitions it is about, with the revision each had when it was opened or its Crux last decided.
    pub fn lines(&self) -> &BTreeMap<Target, Revision> {
        &self.lines
    }

    /// Who resolves it.
    pub fn route(&self) -> &Route {
        &self.route
    }

    /// Its Crux, when a person resolves it.
    pub fn crux(&self) -> Option<&Crux> {
        match &self.route {
            Route::Machine => None,
            Route::Person(c) => Some(c),
        }
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

    /// Whether it asks its definitions to change now: one the machine resolves does; one a person resolves does
    /// once its Crux is decided for a change (see [`Crux::asks_change`]).
    pub fn asks_change(&self) -> bool {
        match &self.route {
            Route::Machine => true,
            Route::Person(c) => c.asks_change(),
        }
    }

    /// Takes the person's decision on its Crux; a later one replaces an earlier one until it is resolved. Each line
    /// takes the revision in `now`, the one its definition has as the decision is taken (a line missing from `now`
    /// keeps its revision: that its definition is gone is for the caller to report).
    pub fn decide(
        &mut self,
        decision: Decision,
        now: &BTreeMap<Target, Revision>,
    ) -> Result<(), ChangeRequestError> {
        if self.resolved {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        let Route::Person(crux) = &mut self.route else {
            return Err(ChangeRequestError::NoCrux);
        };
        crux.decide(decision)?;
        for (target, revision) in &mut self.lines {
            if let Some(r) = now.get(target) {
                *revision = *r;
            }
        }
        Ok(())
    }

    /// Closes the record. Whether the definitions have followed is read against the Project, so the caller checks
    /// it first (`service::change_request::resolve`). A Crux must be decided first.
    pub fn resolve(&mut self) -> Result<(), ChangeRequestError> {
        if self.resolved {
            return Err(ChangeRequestError::AlreadyResolved);
        }
        if self.crux().is_some_and(|c| c.decision.is_none()) {
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

    fn crux(kind: CruxKind, default: Option<&str>) -> Result<Crux, ChangeRequestError> {
        Crux::new(
            kind,
            vec![
                CruxOption::new(choice("up"), None, None),
                CruxOption::new(
                    choice("to even"),
                    Some(prose("C1: a total is rounded half to even")),
                    Some(prose("the rounding tests change")),
                ),
                CruxOption::new(choice("keep"), None, None),
            ],
            default.map(|d| Reasoned::new(choice(d), Some(prose("the literal reading")))),
            Some(Reasoned::new(choice("to even"), None)),
        )
    }

    fn id(n: u128) -> ChangeRequestId {
        ChangeRequestId::from_uuid(test_uuid(n))
    }

    fn cr(route: Route) -> ChangeRequest {
        ChangeRequest::open(
            id(1),
            ChangeRequestDraft {
                title: Title::try_from("How is a half rounded?").unwrap(),
                situation: Some(prose("The requirement says rounded, not how.")),
                intent: Intent::ChangeBehavior,
                parent: Some(id(9)),
                issuer: Some(Issuer::try_from("designer").unwrap()),
                lines: BTreeMap::from([(line(), Revision::first())]),
                route,
            },
        )
    }

    fn person() -> ChangeRequest {
        cr(Route::Person(
            crux(CruxKind::Ambiguity, Some("up")).unwrap(),
        ))
    }

    fn chosen(c: &str) -> Decision {
        Decision::Chosen(Reasoned::new(choice(c), Some(prose("fairer"))))
    }

    #[test]
    fn a_new_change_request_is_open_and_keeps_what_it_was_opened_with() {
        let c = person();
        assert_eq!(c.state(), ChangeRequestState::Open);
        assert!(!c.asks_change());
        assert_eq!(c.intent(), Intent::ChangeBehavior);
        assert_eq!(c.parent(), Some(id(9)));
        assert_eq!(c.issuer().unwrap().as_str(), "designer");
        let machine = cr(Route::Machine);
        assert_eq!(machine.state(), ChangeRequestState::Open);
        assert!(machine.asks_change());
        assert!(machine.crux().is_none());
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
            route: Route::Machine,
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
                CruxOption::new(choice("a"), None, None),
                CruxOption::new(choice("a"), None, None),
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
    fn a_decision_takes_the_revisions_now_and_says_whether_it_asks_a_change() {
        let mut c = person();
        let now = BTreeMap::from([(line(), Revision::first().next())]);
        assert_eq!(
            c.decide(chosen("down"), &now),
            Err(ChangeRequestError::NotAnOption(choice("down")))
        );
        assert_eq!(c.lines()[&line()], Revision::first());
        c.decide(chosen("to even"), &now).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Decided);
        assert!(c.asks_change());
        assert_eq!(c.lines()[&line()], Revision::first().next());
        c.decide(chosen("up"), &now).unwrap();
        assert!(!c.asks_change());
        c.decide(chosen("keep"), &now).unwrap();
        assert!(!c.asks_change());
        let worded = Decision::Worded {
            wording: prose("round half away from zero"),
            reason: None,
        };
        c.decide(worded, &now).unwrap();
        assert!(c.asks_change());
        let free = Decision::Unspecified {
            keep: vec![prose("a total is whole cents")],
        };
        c.decide(free, &now).unwrap();
        assert!(!c.asks_change());
    }

    #[test]
    fn a_model_question_cannot_be_left_unspecified() {
        let mut c = cr(Route::Person(crux(CruxKind::Model, None).unwrap()));
        assert_eq!(
            c.decide(Decision::Unspecified { keep: Vec::new() }, &BTreeMap::new()),
            Err(ChangeRequestError::UnspecifiedNotAllowed("model"))
        );
    }

    #[test]
    fn a_change_request_the_machine_resolves_has_nothing_to_decide() {
        let mut c = cr(Route::Machine);
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
