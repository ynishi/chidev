//! The Crux: a question that no repository and no general knowledge settles (those are Assumptions, recorded on the
//! change request) and the Project's decision Policy does not settle either, so a decision maker answers it. It is
//! an aggregate of its own, raised in the change request it stops; the change request does not hold it, the Crux
//! points at the change request (`raised_in`), so the relation is kept in one place. A change request can be stopped
//! by several Cruxes; a Crux stops one change request.
//!
//! A Crux is raised open and answered once. Its answer is a decision of the decision maker, recorded with its basis,
//! who gave it and when, and never rewritten: a different decision is a new Crux that supersedes the one answered.
//! The decision is required, and "left unspecified" and "withdrawn" are decisions too. The one exception is a
//! question about the words of the model (kind `model`), which cannot be left unspecified: without the word, the
//! model cannot be written.
//!
//! Who may decide (`CruxAnswerable`) is managed outside chidev, which holds no identity of its users; the answer
//! keeps the claim of who decided (`by`) and nothing checks it.

use std::collections::BTreeMap;

use super::change_request::ChangeRequestError;
use crate::domain::vo::answer::Answer;
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Importance, Recommended};
use crate::domain::vo::finding::FindingRef;
use crate::domain::vo::id::{ChangeRequestId, CruxId, CruxOptionId};
use crate::domain::vo::prose::Prose;
use crate::domain::vo::target::{ExpectKind, Target};

/// Whether a Crux has been answered. The answer is part of the state, so a Crux cannot be open and have one, or be
/// answered and lack it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CruxState {
    /// No decision yet: a decision maker is awaited, and the change request it was raised in waits.
    Open,
    /// Decided, once.
    Answered(Answer),
}

impl CruxState {
    /// The word for it: `open` or `answered`.
    pub fn word(&self) -> &'static str {
        match self {
            CruxState::Open => "open",
            CruxState::Answered(_) => "answered",
        }
    }
}

/// What a Crux is raised with, before its rules are checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CruxDraft {
    /// The change request it stops: normally the one in whose work the finding arose, but any change request will do.
    pub raised_in: ChangeRequestId,
    /// The findings it was raised from; none for a question that arises from the text of the change request.
    pub findings: Vec<FindingRef>,
    /// What kind of question it is.
    pub kind: CruxKind,
    /// The question, in a sentence.
    pub question: Prose,
    /// The answers it offers, in the order given.
    pub options: Vec<CruxOption>,
    /// The option recommended.
    pub recommended: Option<Recommended>,
    /// How much the answer matters.
    pub importance: Option<Importance>,
    /// The Crux whose decision this one replaces (whether it exists and has a decision to replace is read against
    /// the other Cruxes: see `service::crux::check_supersedes`).
    pub supersedes: Option<CruxId>,
}

/// The question a decision maker must answer for a change request: its kind and text, the options with what each
/// expects, the recommendation (with how sure it is) and the importance that let the Policy settle it, the Crux it
/// supersedes when it replaces a decision, and the answer once given. There is no default to go on with: what the
/// change request needs the answer for waits for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crux {
    id: CruxId,
    raised_in: ChangeRequestId,
    findings: Vec<FindingRef>,
    kind: CruxKind,
    question: Prose,
    options: Vec<CruxOption>,
    recommended: Option<Recommended>,
    importance: Option<Importance>,
    supersedes: Option<CruxId>,
    state: CruxState,
}

impl Crux {
    /// An open Crux: one option at least, no option id and no choice twice, the recommendation among the options. Only its own rules
    /// are read here; the change request it is raised in, and the Crux it supersedes, are other records.
    pub fn raise(id: CruxId, draft: CruxDraft) -> Result<Crux, ChangeRequestError> {
        if draft.options.is_empty() {
            return Err(ChangeRequestError::NoOption);
        }
        for (k, o) in draft.options.iter().enumerate() {
            if draft.options[..k].iter().any(|p| p.id() == o.id()) {
                return Err(ChangeRequestError::DuplicateOptionId(o.id()));
            }
            if draft.options[..k].iter().any(|p| p.choice() == o.choice()) {
                return Err(ChangeRequestError::DuplicateOption(o.choice().clone()));
            }
        }
        if let Some(r) = draft
            .recommended
            .as_ref()
            .filter(|r| !draft.options.iter().any(|o| o.id() == r.option()))
        {
            return Err(ChangeRequestError::NotAnOption(r.option()));
        }
        Ok(Crux {
            id,
            raised_in: draft.raised_in,
            findings: draft.findings,
            kind: draft.kind,
            question: draft.question,
            options: draft.options,
            recommended: draft.recommended,
            importance: draft.importance,
            supersedes: draft.supersedes,
            state: CruxState::Open,
        })
    }

    /// A Crux as a store kept it, in the `state` it was in. Every rule of raising and of answering is read again.
    pub(crate) fn restore(
        id: CruxId,
        draft: CruxDraft,
        state: CruxState,
    ) -> Result<Crux, ChangeRequestError> {
        let mut crux = Crux::raise(id, draft)?;
        if let CruxState::Answered(answer) = state {
            crux.answer(answer)?;
        }
        Ok(crux)
    }

    /// Its id.
    pub fn id(&self) -> CruxId {
        self.id
    }

    /// The change request it stops.
    pub fn raised_in(&self) -> ChangeRequestId {
        self.raised_in
    }

    /// The findings it was raised from.
    pub fn findings(&self) -> &[FindingRef] {
        &self.findings
    }

    /// What kind of question it is.
    pub fn kind(&self) -> CruxKind {
        self.kind
    }

    /// The question.
    pub fn question(&self) -> &Prose {
        &self.question
    }

    /// The options, in the order given.
    pub fn options(&self) -> &[CruxOption] {
        &self.options
    }

    /// The option with this id.
    pub fn option(&self, id: CruxOptionId) -> Option<&CruxOption> {
        self.options.iter().find(|o| o.id() == id)
    }

    /// The option recommended, why, and how sure.
    pub fn recommended(&self) -> Option<&Recommended> {
        self.recommended.as_ref()
    }

    /// How much the answer matters, when it was said.
    pub fn importance(&self) -> Option<Importance> {
        self.importance
    }

    /// The Crux whose decision this one replaces.
    pub fn supersedes(&self) -> Option<CruxId> {
        self.supersedes
    }

    /// Whether it is answered.
    pub fn state(&self) -> &CruxState {
        &self.state
    }

    /// Whether no decision has been taken yet.
    pub fn is_open(&self) -> bool {
        matches!(self.state, CruxState::Open)
    }

    /// The answer, once given.
    pub fn answer_given(&self) -> Option<&Answer> {
        match &self.state {
            CruxState::Open => None,
            CruxState::Answered(a) => Some(a),
        }
    }

    /// The decision, once taken.
    pub fn decision(&self) -> Option<&Decision> {
        self.answer_given().map(Answer::decision)
    }

    /// What the decision asks of targets besides what the change request asks: those of the option chosen. The
    /// decision maker's own wording names nothing more, leaving it unspecified or withdrawing it names nothing, and
    /// no decision yet names nothing.
    pub fn decided_expects(&self) -> BTreeMap<Target, ExpectKind> {
        let chosen = match self.decision() {
            Some(Decision::Chosen { option, .. }) => self.option(*option),
            _ => None,
        };
        chosen.map_or_else(BTreeMap::new, |o| o.expects().clone())
    }

    /// Takes the answer, once. It is read against the options and the kind: a chosen option is one of the options,
    /// and a question about the words of the model is not left unspecified. Whether a Policy basis names a line the
    /// Policy has is read against the Project (`service::crux::decide`). An answer is never rewritten, so a second
    /// one is refused whatever it says, and the first stays.
    pub fn answer(&mut self, answer: Answer) -> Result<(), ChangeRequestError> {
        if !self.is_open() {
            return Err(ChangeRequestError::AlreadyAnswered(self.id));
        }
        match answer.decision() {
            Decision::Chosen { option, .. } if self.option(*option).is_none() => {
                return Err(ChangeRequestError::NotAnOption(*option));
            }
            Decision::Unspecified { .. } if !self.kind.allows_unspecified() => {
                return Err(ChangeRequestError::UnspecifiedNotAllowed(self.kind.word()));
            }
            _ => {}
        }
        self.state = CruxState::Answered(answer);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::answer::{Basis, DecisionMaker};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::confidence::Confidence;
    use crate::domain::vo::id::{AnswerId, ClauseId, test_uuid};
    use crate::domain::vo::timestamp::Timestamp;

    pub(crate) fn choice(s: &str) -> Choice {
        Choice::try_from(s).unwrap()
    }

    pub(crate) fn prose(s: &str) -> Prose {
        Prose::try_from(s).unwrap()
    }

    fn option_id(n: u128) -> CruxOptionId {
        CruxOptionId::from_uuid(test_uuid(1000 + n))
    }

    fn detail() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(31)))
    }

    /// Three options, `up` (1), `to even` (2, which asks for `detail`) and `keep` (3); `to even` is recommended.
    fn draft(kind: CruxKind) -> CruxDraft {
        CruxDraft {
            raised_in: ChangeRequestId::from_uuid(test_uuid(1)),
            findings: Vec::new(),
            kind,
            question: prose("How is a half rounded?"),
            options: vec![
                CruxOption::new(option_id(1), choice("up"), None, BTreeMap::new(), None),
                CruxOption::new(
                    option_id(2),
                    choice("to even"),
                    Some(prose("C1: a total is rounded half to even")),
                    BTreeMap::from([(detail(), ExpectKind::Target)]),
                    Some(prose("the rounding tests change")),
                ),
                CruxOption::new(option_id(3), choice("keep"), None, BTreeMap::new(), None),
            ],
            recommended: Some(Recommended::new(
                option_id(2),
                Some(prose("the literal reading")),
                Some(Confidence::try_from(85).unwrap()),
            )),
            importance: Some(Importance::Normal),
            supersedes: None,
        }
    }

    fn raised(kind: CruxKind) -> Crux {
        Crux::raise(CruxId::from_uuid(test_uuid(500)), draft(kind)).unwrap()
    }

    fn answer_of(decision: Decision) -> Answer {
        Answer::new(
            AnswerId::from_uuid(test_uuid(900)),
            decision,
            Basis::Arbitrary {
                reason: prose("no line of the policy covers it"),
            },
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(1_000),
        )
    }

    fn chosen(n: u128) -> Answer {
        answer_of(Decision::Chosen {
            option: option_id(n),
            reason: Some(prose("fairer")),
        })
    }

    #[test]
    fn a_crux_is_raised_open_and_keeps_what_it_was_raised_with() {
        let c = raised(CruxKind::Ambiguity);
        assert!(c.is_open());
        assert_eq!(c.state().word(), "open");
        assert!(c.answer_given().is_none() && c.decision().is_none());
        assert_eq!(c.id(), CruxId::from_uuid(test_uuid(500)));
        assert_eq!(c.raised_in(), ChangeRequestId::from_uuid(test_uuid(1)));
        assert!(c.findings().is_empty());
        assert_eq!(c.kind(), CruxKind::Ambiguity);
        assert_eq!(c.question().as_str(), "How is a half rounded?");
        assert_eq!(c.options().len(), 3);
        assert_eq!(c.option(option_id(2)).unwrap().choice().as_str(), "to even");
        assert!(c.option(option_id(9)).is_none());
        assert_eq!(c.recommended().map(Recommended::option), Some(option_id(2)));
        assert_eq!(c.importance(), Some(Importance::Normal));
        assert_eq!(c.supersedes(), None);
        assert!(c.decided_expects().is_empty());
    }

    #[test]
    fn a_crux_has_distinct_options_and_its_recommendation_among_them() {
        let mut none = draft(CruxKind::Ambiguity);
        none.options.clear();
        none.recommended = None;
        assert_eq!(
            Crux::raise(CruxId::from_uuid(test_uuid(500)), none),
            Err(ChangeRequestError::NoOption)
        );
        let mut twice = draft(CruxKind::Ambiguity);
        twice.options.push(CruxOption::new(
            option_id(4),
            choice("up"),
            None,
            BTreeMap::new(),
            None,
        ));
        assert_eq!(
            Crux::raise(CruxId::from_uuid(test_uuid(500)), twice),
            Err(ChangeRequestError::DuplicateOption(choice("up")))
        );
        // The same id for two options with other choices: the id names one option, so it is refused on its own.
        let mut same_id = draft(CruxKind::Ambiguity);
        same_id.options.push(CruxOption::new(
            option_id(1),
            choice("down"),
            None,
            BTreeMap::new(),
            None,
        ));
        assert_eq!(
            Crux::raise(CruxId::from_uuid(test_uuid(500)), same_id.clone()),
            Err(ChangeRequestError::DuplicateOptionId(option_id(1)))
        );
        // A stored Crux is read by the same rules.
        assert_eq!(
            Crux::restore(CruxId::from_uuid(test_uuid(500)), same_id, CruxState::Open),
            Err(ChangeRequestError::DuplicateOptionId(option_id(1)))
        );
        let mut stray = draft(CruxKind::Ambiguity);
        stray.recommended = Some(Recommended::new(option_id(9), None, None));
        assert_eq!(
            Crux::raise(CruxId::from_uuid(test_uuid(500)), stray),
            Err(ChangeRequestError::NotAnOption(option_id(9)))
        );
    }

    #[test]
    fn a_crux_may_have_no_recommendation_importance_findings_or_superseded_crux() {
        let mut d = draft(CruxKind::Conflict);
        d.recommended = None;
        d.importance = None;
        let c = Crux::raise(CruxId::from_uuid(test_uuid(500)), d).unwrap();
        assert!(c.recommended().is_none() && c.importance().is_none());
        let mut d = draft(CruxKind::Conflict);
        d.supersedes = Some(CruxId::from_uuid(test_uuid(7)));
        d.findings = vec![FindingRef::new(
            crate::domain::vo::finding::RuleId::from_static("context.no-harness"),
            None,
        )];
        let c = Crux::raise(CruxId::from_uuid(test_uuid(500)), d).unwrap();
        assert_eq!(c.supersedes(), Some(CruxId::from_uuid(test_uuid(7))));
        assert_eq!(c.findings().len(), 1);
    }

    #[test]
    fn an_answer_is_given_once_and_never_rewritten() {
        let mut c = raised(CruxKind::Ambiguity);
        let first = chosen(2);
        c.answer(first.clone()).unwrap();
        assert!(!c.is_open());
        assert_eq!(c.answer_given(), Some(&first));
        // Any other answer, of any kind, is refused, and the first stays whole.
        let worded = answer_of(Decision::Worded {
            wording: prose("round half away from zero"),
            reason: None,
        });
        let free = answer_of(Decision::Unspecified {
            keep: vec![prose("a total is whole cents")],
        });
        for again in [
            chosen(1),
            chosen(2),
            worded,
            free,
            answer_of(Decision::Withdrawn),
        ] {
            assert_eq!(
                c.answer(again),
                Err(ChangeRequestError::AlreadyAnswered(c.id()))
            );
        }
        assert_eq!(c.answer_given(), Some(&first));
        assert_eq!(c.state().word(), "answered");
    }

    #[test]
    fn a_chosen_option_is_one_of_the_options_and_the_wrong_one_leaves_it_open() {
        let mut c = raised(CruxKind::Ambiguity);
        assert_eq!(
            c.answer(chosen(9)),
            Err(ChangeRequestError::NotAnOption(option_id(9)))
        );
        assert!(c.is_open());
    }

    #[test]
    fn a_model_question_cannot_be_left_unspecified_but_can_be_worded_or_withdrawn() {
        let free = answer_of(Decision::Unspecified { keep: Vec::new() });
        let mut c = raised(CruxKind::Model);
        assert_eq!(
            c.answer(free.clone()),
            Err(ChangeRequestError::UnspecifiedNotAllowed("model"))
        );
        // Refused, it did not take the answer.
        assert!(c.is_open());
        c.answer(answer_of(Decision::Withdrawn)).unwrap();
        let mut c = raised(CruxKind::Model);
        c.answer(answer_of(Decision::Worded {
            wording: prose("a Cart is a Basket"),
            reason: None,
        }))
        .unwrap();
        // The other kinds may be left unspecified.
        for kind in [
            CruxKind::Ambiguity,
            CruxKind::Conflict,
            CruxKind::Disagreement,
        ] {
            raised(kind).answer(free.clone()).unwrap();
        }
    }

    #[test]
    fn what_a_decision_asks_is_what_the_option_chosen_expects_and_nothing_otherwise() {
        let mut c = raised(CruxKind::Ambiguity);
        c.answer(chosen(2)).unwrap();
        assert_eq!(
            c.decided_expects(),
            BTreeMap::from([(detail(), ExpectKind::Target)])
        );
        for decision in [
            Decision::Worded {
                wording: prose("w"),
                reason: None,
            },
            Decision::Unspecified { keep: Vec::new() },
            Decision::Withdrawn,
            Decision::Chosen {
                option: option_id(1),
                reason: None,
            },
        ] {
            let mut c = raised(CruxKind::Ambiguity);
            c.answer(answer_of(decision)).unwrap();
            assert!(c.decided_expects().is_empty());
        }
    }

    #[test]
    fn a_stored_crux_is_checked_like_a_new_one() {
        let id = CruxId::from_uuid(test_uuid(500));
        let state = |a: Answer| CruxState::Answered(a);
        let restore = |d: CruxDraft, s: CruxState| Crux::restore(id, d, s);
        assert!(restore(draft(CruxKind::Ambiguity), CruxState::Open).is_ok());
        assert!(restore(draft(CruxKind::Ambiguity), state(chosen(2))).is_ok());
        // an answer to an option the Crux does not have
        assert_eq!(
            restore(draft(CruxKind::Ambiguity), state(chosen(9))),
            Err(ChangeRequestError::NotAnOption(option_id(9)))
        );
        // a model question left unspecified
        assert_eq!(
            restore(
                draft(CruxKind::Model),
                state(answer_of(Decision::Unspecified { keep: Vec::new() }))
            ),
            Err(ChangeRequestError::UnspecifiedNotAllowed("model"))
        );
        // a recommendation outside the options, and no option at all
        // The same id for two options with other choices: the id names one option, so it is refused on its own.
        let mut same_id = draft(CruxKind::Ambiguity);
        same_id.options.push(CruxOption::new(
            option_id(1),
            choice("down"),
            None,
            BTreeMap::new(),
            None,
        ));
        assert_eq!(
            Crux::raise(CruxId::from_uuid(test_uuid(500)), same_id.clone()),
            Err(ChangeRequestError::DuplicateOptionId(option_id(1)))
        );
        // A stored Crux is read by the same rules.
        assert_eq!(
            Crux::restore(CruxId::from_uuid(test_uuid(500)), same_id, CruxState::Open),
            Err(ChangeRequestError::DuplicateOptionId(option_id(1)))
        );
        let mut stray = draft(CruxKind::Ambiguity);
        stray.recommended = Some(Recommended::new(option_id(9), None, None));
        assert_eq!(
            restore(stray, CruxState::Open),
            Err(ChangeRequestError::NotAnOption(option_id(9)))
        );
        let mut none = draft(CruxKind::Ambiguity);
        none.options.clear();
        none.recommended = None;
        assert_eq!(
            restore(none, CruxState::Open),
            Err(ChangeRequestError::NoOption)
        );
        // what is restored is what was stored
        let c = restore(draft(CruxKind::Ambiguity), state(chosen(2))).unwrap();
        assert_eq!(c.answer_given(), Some(&chosen(2)));
    }
}
