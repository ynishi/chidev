//! The Crux: a question only a person can settle, kept as a decision record: why it came up, where from, the
//! options with what each would change, the default work goes on with, the recommendation, the definitions it is
//! about, and the decision once taken.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::choice::Choice;
use crate::domain::vo::crux::{CruxKind, CruxOption, CruxSource, Decision, Reasoned};
use crate::domain::vo::id::CruxId;
use crate::domain::vo::prose::Prose;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// Why a Crux cannot be made, decided or resolved. Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CruxError {
    /// There is no option to choose.
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
    /// The Crux is resolved; its decision stands.
    #[error("the crux is resolved")]
    AlreadyResolved,
    /// The Crux has no decision to resolve.
    #[error("the crux has no decision yet")]
    Undecided,
    /// The definitions have not followed the decision yet: these are still as they were, or gone.
    #[error("the decision is not applied yet: {0:?}")]
    NotApplied(Vec<Target>),
}

/// Where a Crux stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CruxState {
    /// No decision yet; work goes on with the default, if the kind has one.
    Open,
    /// Decided; the definitions may still have to follow.
    Decided,
    /// Decided and applied: the record is closed.
    Resolved,
}

/// What a Crux is made with, before its rules are checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CruxDraft {
    /// What kind of question it is.
    pub kind: CruxKind,
    /// The question.
    pub title: Title,
    /// Why it is a question.
    pub situation: Option<Prose>,
    /// Where it came from.
    pub source: Option<CruxSource>,
    /// The options, in the order given (one at least, no choice twice).
    pub options: Vec<CruxOption>,
    /// The option work goes on with until the person decides, and why (only for a kind that has one).
    pub default: Option<Reasoned>,
    /// The option recommended, and why.
    pub recommended: Option<Reasoned>,
    /// The definitions the question is about, with the revision each has now.
    pub lines: BTreeMap<Target, Revision>,
}

/// A question only a person can settle, and its record. Work goes on with the default (when the kind has one) until
/// the person decides. The definitions the question is about are kept with a revision each, so that whether they
/// have followed the decision is seen by comparing revisions; once they have, the Crux is resolved and its decision
/// stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crux {
    id: CruxId,
    kind: CruxKind,
    title: Title,
    situation: Option<Prose>,
    source: Option<CruxSource>,
    options: Vec<CruxOption>,
    default: Option<Reasoned>,
    recommended: Option<Reasoned>,
    lines: BTreeMap<Target, Revision>,
    decision: Option<Decision>,
    resolved: bool,
}

impl Crux {
    /// An open Crux.
    pub fn new(id: CruxId, draft: CruxDraft) -> Result<Crux, CruxError> {
        if draft.options.is_empty() {
            return Err(CruxError::NoOption);
        }
        for (k, o) in draft.options.iter().enumerate() {
            if draft.options[..k].iter().any(|p| p.choice() == o.choice()) {
                return Err(CruxError::DuplicateOption(o.choice().clone()));
            }
        }
        let is_option = |c: &Choice| draft.options.iter().any(|o| o.choice() == c);
        if let Some(d) = &draft.default {
            if !draft.kind.allows_default() {
                return Err(CruxError::DefaultNotAllowed(draft.kind.word()));
            }
            if !is_option(d.choice()) {
                return Err(CruxError::NotAnOption(d.choice().clone()));
            }
        }
        if let Some(r) = draft
            .recommended
            .as_ref()
            .filter(|r| !is_option(r.choice()))
        {
            return Err(CruxError::NotAnOption(r.choice().clone()));
        }
        Ok(Crux {
            id,
            kind: draft.kind,
            title: draft.title,
            situation: draft.situation,
            source: draft.source,
            options: draft.options,
            default: draft.default,
            recommended: draft.recommended,
            lines: draft.lines,
            decision: None,
            resolved: false,
        })
    }

    /// Its id.
    pub fn id(&self) -> CruxId {
        self.id
    }

    /// What kind of question it is.
    pub fn kind(&self) -> CruxKind {
        self.kind
    }

    /// The question.
    pub fn title(&self) -> &Title {
        &self.title
    }

    /// Why it is a question.
    pub fn situation(&self) -> Option<&Prose> {
        self.situation.as_ref()
    }

    /// Where it came from.
    pub fn source(&self) -> Option<&CruxSource> {
        self.source.as_ref()
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

    /// The definitions it is about, with the revision each had when it was raised or last decided.
    pub fn lines(&self) -> &BTreeMap<Target, Revision> {
        &self.lines
    }

    /// The decision, once taken.
    pub fn decision(&self) -> Option<&Decision> {
        self.decision.as_ref()
    }

    /// Where it stands.
    pub fn state(&self) -> CruxState {
        match (&self.decision, self.resolved) {
            (None, _) => CruxState::Open,
            (Some(_), false) => CruxState::Decided,
            (Some(_), true) => CruxState::Resolved,
        }
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

    /// Takes the person's decision; a later one replaces an earlier one until the Crux is resolved. Each line takes
    /// the revision in `now`, the one its definition has as the decision is taken (a line missing from `now` keeps
    /// its revision: that its definition is gone is for the caller to report).
    pub fn decide(
        &mut self,
        decision: Decision,
        now: &BTreeMap<Target, Revision>,
    ) -> Result<(), CruxError> {
        if self.resolved {
            return Err(CruxError::AlreadyResolved);
        }
        match &decision {
            Decision::Chosen(c) if self.option(c.choice()).is_none() => {
                return Err(CruxError::NotAnOption(c.choice().clone()));
            }
            Decision::Unspecified { .. } if !self.kind.allows_unspecified() => {
                return Err(CruxError::UnspecifiedNotAllowed(self.kind.word()));
            }
            _ => {}
        }
        self.decision = Some(decision);
        for (target, revision) in &mut self.lines {
            if let Some(r) = now.get(target) {
                *revision = *r;
            }
        }
        Ok(())
    }

    /// Closes the record. Whether the definitions have followed the decision is read against the Project, so the
    /// caller checks it first (`service::crux::resolve`).
    pub fn resolve(&mut self) -> Result<(), CruxError> {
        if self.resolved {
            return Err(CruxError::AlreadyResolved);
        }
        if self.decision.is_none() {
            return Err(CruxError::Undecided);
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

    fn draft(kind: CruxKind, default: Option<&str>) -> CruxDraft {
        CruxDraft {
            kind,
            title: Title::try_from("How is a half rounded?").unwrap(),
            situation: Some(prose("The requirement says rounded, not how.")),
            source: Some(CruxSource::Request(prose("totals must be fair"))),
            options: vec![
                CruxOption::new(choice("up"), None, None),
                CruxOption::new(
                    choice("to even"),
                    Some(prose("C1: a total is rounded half to even")),
                    Some(prose("the rounding tests change")),
                ),
                CruxOption::new(choice("keep"), None, None),
            ],
            default: default.map(|d| Reasoned::new(choice(d), Some(prose("the literal reading")))),
            recommended: Some(Reasoned::new(choice("to even"), None)),
            lines: BTreeMap::from([(line(), Revision::first())]),
        }
    }

    fn crux() -> Crux {
        Crux::new(
            CruxId::from_uuid(test_uuid(1)),
            draft(CruxKind::Ambiguity, Some("up")),
        )
        .unwrap()
    }

    fn chosen(c: &str) -> Decision {
        Decision::Chosen(Reasoned::new(choice(c), Some(prose("fairer"))))
    }

    #[test]
    fn a_new_crux_is_open_and_asks_no_change() {
        let c = crux();
        assert_eq!(c.state(), CruxState::Open);
        assert!(!c.asks_change());
        assert_eq!(c.default().unwrap().choice(), &choice("up"));
        assert_eq!(c.options().len(), 3);
    }

    #[test]
    fn options_are_distinct_and_the_default_and_recommendation_are_among_them() {
        let id = CruxId::from_uuid(test_uuid(1));
        let mut d = draft(CruxKind::Ambiguity, Some("up"));
        d.options.clear();
        assert_eq!(Crux::new(id, d), Err(CruxError::NoOption));
        let mut d = draft(CruxKind::Ambiguity, Some("up"));
        d.options.push(CruxOption::new(choice("up"), None, None));
        assert_eq!(
            Crux::new(id, d),
            Err(CruxError::DuplicateOption(choice("up")))
        );
        assert_eq!(
            Crux::new(id, draft(CruxKind::Ambiguity, Some("down"))),
            Err(CruxError::NotAnOption(choice("down")))
        );
        let mut d = draft(CruxKind::Ambiguity, Some("up"));
        d.recommended = Some(Reasoned::new(choice("down"), None));
        assert_eq!(
            Crux::new(id, d),
            Err(CruxError::NotAnOption(choice("down")))
        );
    }

    #[test]
    fn a_conflict_or_a_model_question_takes_no_default() {
        let id = CruxId::from_uuid(test_uuid(1));
        assert_eq!(
            Crux::new(id, draft(CruxKind::Conflict, Some("up"))),
            Err(CruxError::DefaultNotAllowed("conflict"))
        );
        assert!(Crux::new(id, draft(CruxKind::Model, None)).is_ok());
    }

    #[test]
    fn a_decision_takes_the_revisions_now_and_says_whether_it_asks_a_change() {
        let mut c = crux();
        let now = BTreeMap::from([(line(), Revision::first().next())]);
        assert_eq!(
            c.decide(chosen("down"), &now),
            Err(CruxError::NotAnOption(choice("down")))
        );
        assert_eq!(c.lines()[&line()], Revision::first());
        c.decide(chosen("to even"), &now).unwrap();
        assert_eq!(c.state(), CruxState::Decided);
        assert!(c.asks_change());
        assert_eq!(c.lines()[&line()], Revision::first().next());
        // the default, or an option that changes nothing, asks no change
        c.decide(chosen("up"), &now).unwrap();
        assert!(!c.asks_change());
        c.decide(chosen("keep"), &now).unwrap();
        assert!(!c.asks_change());
        c.decide(
            Decision::Worded {
                wording: prose("round half away from zero"),
                reason: None,
            },
            &now,
        )
        .unwrap();
        assert!(c.asks_change());
        c.decide(
            Decision::Unspecified {
                keep: vec![prose("a total is whole cents")],
            },
            &now,
        )
        .unwrap();
        assert!(!c.asks_change());
    }

    #[test]
    fn a_model_question_cannot_be_left_unspecified() {
        let mut c = Crux::new(
            CruxId::from_uuid(test_uuid(1)),
            draft(CruxKind::Model, None),
        )
        .unwrap();
        assert_eq!(
            c.decide(Decision::Unspecified { keep: Vec::new() }, &BTreeMap::new()),
            Err(CruxError::UnspecifiedNotAllowed("model"))
        );
    }

    #[test]
    fn a_decided_crux_resolves_once_and_then_takes_no_decision() {
        let mut c = crux();
        assert_eq!(c.resolve(), Err(CruxError::Undecided));
        c.decide(chosen("to even"), &BTreeMap::new()).unwrap();
        c.resolve().unwrap();
        assert_eq!(c.state(), CruxState::Resolved);
        assert_eq!(c.resolve(), Err(CruxError::AlreadyResolved));
        assert_eq!(
            c.decide(chosen("up"), &BTreeMap::new()),
            Err(CruxError::AlreadyResolved)
        );
    }
}
