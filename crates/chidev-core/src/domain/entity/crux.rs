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
use std::fmt;

use super::change_request::ChangeRequestError;
use crate::domain::vo::answer::{Answer, Basis, Beyond};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::correction::{
    Correctable, Correction, CorrectionError, SoughtText, check_places, masked, masked_distinct,
};
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
    /// The corrections of its texts, in the order they were made: the Crux's history of corrections. Empty when
    /// raised.
    corrections: Vec<Correction<CruxField>>,
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
            corrections: Vec::new(),
        })
    }

    /// A Crux as a store kept it, in the `state` it was in, with the `corrections` made of its texts. Every rule of
    /// raising and of answering is read again.
    pub(crate) fn restore(
        id: CruxId,
        draft: CruxDraft,
        state: CruxState,
        corrections: Vec<Correction<CruxField>>,
    ) -> Result<Crux, ChangeRequestError> {
        let mut crux = Crux::raise(id, draft)?;
        if let CruxState::Answered(answer) = state {
            crux.answer(answer)?;
        }
        crux.corrections = corrections;
        Ok(crux)
    }

    /// The corrections made of its texts, in the order they were made: each keeps the fields, why, who and when,
    /// and no text.
    pub fn corrections(&self) -> &[Correction<CruxField>] {
        &self.corrections
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

    /// The option the decision chose; none while it is open, and when the decision chose no option (the decision
    /// maker's own wording, leaving it unspecified, or withdrawing it).
    pub fn chosen_option(&self) -> Option<&CruxOption> {
        match self.decision() {
            Some(Decision::Chosen { option, .. }) => self.option(*option),
            _ => None,
        }
    }

    /// What the decision asks of targets besides what the change request asks: those of the option chosen. The
    /// decision maker's own wording names nothing more, leaving it unspecified or withdrawing it names nothing, and
    /// no decision yet names nothing.
    pub fn decided_expects(&self) -> BTreeMap<Target, ExpectKind> {
        self.chosen_option()
            .map_or_else(BTreeMap::new, |o| o.expects().clone())
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
                return Err(ChangeRequestError::UnspecifiedNotAllowed {
                    crux: self.id,
                    kind: self.kind,
                });
            }
            _ => {}
        }
        self.state = CruxState::Answered(answer);
        Ok(())
    }
}

/// A place of text in a Crux, for a correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CruxField {
    /// The question.
    Question,
    /// The choice of an option.
    OptionChoice {
        /// The option.
        option: CruxOptionId,
    },
    /// The change an option makes.
    OptionChange {
        /// The option.
        option: CruxOptionId,
    },
    /// What an option affects besides.
    OptionConsequence {
        /// The option.
        option: CruxOptionId,
    },
    /// Why the option is recommended.
    RecommendationReason,
    /// The reason of a decision that chose an option or worded the behaviour.
    DecisionReason,
    /// The behaviour a decision worded itself.
    DecisionWording,
    /// One of what must still hold when a decision left the behaviour unspecified.
    DecisionKeep {
        /// Which, in the order given.
        keep: usize,
    },
    /// The reason of an arbitrary basis.
    BasisReason,
    /// The reason an answer declined what lies outside its change request.
    BeyondReason,
    /// The name of who answered. The kind of the maker (role or person) is kept.
    AnswerBy,
}

impl CruxField {
    /// The words for the field, which a message and an answer name it by: `question`, `option[<id>].choice`,
    /// `decision.reason`.
    pub fn word(&self) -> String {
        match self {
            CruxField::Question => "question".to_string(),
            CruxField::OptionChoice { option } => format!("option[{option}].choice"),
            CruxField::OptionChange { option } => format!("option[{option}].change"),
            CruxField::OptionConsequence { option } => format!("option[{option}].consequence"),
            CruxField::RecommendationReason => "recommendation.reason".to_string(),
            CruxField::DecisionReason => "decision.reason".to_string(),
            CruxField::DecisionWording => "decision.wording".to_string(),
            CruxField::DecisionKeep { keep } => format!("decision.keep[{keep}]"),
            CruxField::BasisReason => "basis.reason".to_string(),
            CruxField::BeyondReason => "beyond.reason".to_string(),
            CruxField::AnswerBy => "answer.by".to_string(),
        }
    }
}

impl fmt::Display for CruxField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.word())
    }
}

impl Correctable for Crux {
    type Field = CruxField;

    fn texts(&self) -> Vec<(CruxField, &str)> {
        let mut out = vec![(CruxField::Question, self.question.as_str())];
        for o in &self.options {
            let option = o.id();
            out.push((CruxField::OptionChoice { option }, o.choice().as_str()));
            if let Some(change) = o.change() {
                out.push((CruxField::OptionChange { option }, change.as_str()));
            }
            if let Some(consequence) = o.consequence() {
                out.push((
                    CruxField::OptionConsequence { option },
                    consequence.as_str(),
                ));
            }
        }
        if let Some(reason) = self.recommended.as_ref().and_then(Recommended::reason) {
            out.push((CruxField::RecommendationReason, reason.as_str()));
        }
        if let CruxState::Answered(answer) = &self.state {
            match answer.decision() {
                Decision::Chosen { reason, .. } => {
                    if let Some(reason) = reason {
                        out.push((CruxField::DecisionReason, reason.as_str()));
                    }
                }
                Decision::Worded { wording, reason } => {
                    out.push((CruxField::DecisionWording, wording.as_str()));
                    if let Some(reason) = reason {
                        out.push((CruxField::DecisionReason, reason.as_str()));
                    }
                }
                Decision::Unspecified { keep } => {
                    for (k, held) in keep.iter().enumerate() {
                        out.push((CruxField::DecisionKeep { keep: k }, held.as_str()));
                    }
                }
                Decision::Withdrawn => {}
            }
            if let Basis::Arbitrary { reason } = answer.basis() {
                out.push((CruxField::BasisReason, reason.as_str()));
            }
            if let Some(Beyond::Declined { reason }) = answer.beyond() {
                out.push((CruxField::BeyondReason, reason.as_str()));
            }
            out.push((CruxField::AnswerBy, answer.by().name().as_str()));
        }
        out
    }

    fn correct(
        &mut self,
        sought: &SoughtText,
        correction: Correction<CruxField>,
        taken: &dyn Fn(&CruxField, &str) -> bool,
    ) -> Result<(), CorrectionError> {
        check_places(self, sought, correction.fields())?;
        let mut next = self.clone();
        for field in correction.fields() {
            next.replace_text(*field, sought, taken)?;
        }
        next.corrections.push(correction);
        *self = next;
        Ok(())
    }
}

impl Crux {
    /// Replaces the text sought in one field, and builds the value again with its own constructor. Called on a
    /// copy: an error leaves it half done, and the copy is dropped.
    fn replace_text(
        &mut self,
        field: CruxField,
        sought: &SoughtText,
        taken: &dyn Fn(&CruxField, &str) -> bool,
    ) -> Result<(), CorrectionError> {
        let word = field.word();
        let text = self
            .texts()
            .into_iter()
            .find(|(f, _)| *f == field)
            .map(|(_, t)| t.to_string())
            .ok_or_else(|| CorrectionError::NoSuchField(word.clone()))?;
        let new = match field {
            // a choice stays different from the others of its Crux, and from what the caller holds
            CruxField::OptionChoice { option } => masked_distinct(sought, &word, &text, |c| {
                taken(&field, c)
                    || self
                        .options
                        .iter()
                        .any(|o| o.id() != option && o.choice().as_str() == c)
            })?,
            _ => masked(sought, &word, &text)?,
        };
        let invalid = || CorrectionError::Invalid(word.clone());
        let prose = |s: &str| Prose::try_from(s).map_err(|_| invalid());
        let no_such = || CorrectionError::NoSuchField(word.clone());
        let option_at = |options: &[CruxOption], id: CruxOptionId| {
            options
                .iter()
                .position(|o| o.id() == id)
                .ok_or_else(no_such)
        };
        match field {
            CruxField::Question => self.question = prose(&new)?,
            CruxField::OptionChoice { option } => {
                let k = option_at(&self.options, option)?;
                let o = &self.options[k];
                self.options[k] = CruxOption::new(
                    o.id(),
                    Choice::try_from(new.as_str()).map_err(|_| invalid())?,
                    o.change().cloned(),
                    o.expects().clone(),
                    o.consequence().cloned(),
                );
            }
            CruxField::OptionChange { option } => {
                let k = option_at(&self.options, option)?;
                let o = &self.options[k];
                self.options[k] = CruxOption::new(
                    o.id(),
                    o.choice().clone(),
                    Some(prose(&new)?),
                    o.expects().clone(),
                    o.consequence().cloned(),
                );
            }
            CruxField::OptionConsequence { option } => {
                let k = option_at(&self.options, option)?;
                let o = &self.options[k];
                self.options[k] = CruxOption::new(
                    o.id(),
                    o.choice().clone(),
                    o.change().cloned(),
                    o.expects().clone(),
                    Some(prose(&new)?),
                );
            }
            CruxField::RecommendationReason => {
                let r = self.recommended.as_ref().ok_or_else(no_such)?;
                self.recommended = Some(Recommended::new(
                    r.option(),
                    Some(prose(&new)?),
                    r.confidence(),
                ));
            }
            CruxField::DecisionReason
            | CruxField::DecisionWording
            | CruxField::DecisionKeep { .. }
            | CruxField::BasisReason
            | CruxField::BeyondReason
            | CruxField::AnswerBy => {
                let CruxState::Answered(a) = &self.state else {
                    return Err(no_such());
                };
                let mut decision = a.decision().clone();
                let mut basis = a.basis().clone();
                let mut by = a.by().clone();
                let mut beyond = a.beyond().cloned();
                match (field, &mut decision, &mut basis, &mut beyond) {
                    (
                        CruxField::DecisionReason,
                        Decision::Chosen { reason, .. } | Decision::Worded { reason, .. },
                        _,
                        _,
                    ) => *reason = Some(prose(&new)?),
                    (CruxField::DecisionWording, Decision::Worded { wording, .. }, _, _) => {
                        *wording = prose(&new)?;
                    }
                    (
                        CruxField::DecisionKeep { keep },
                        Decision::Unspecified { keep: held },
                        _,
                        _,
                    ) => {
                        *held.get_mut(keep).ok_or_else(no_such)? = prose(&new)?;
                    }
                    (CruxField::BasisReason, _, Basis::Arbitrary { reason }, _) => {
                        *reason = prose(&new)?;
                    }
                    (CruxField::BeyondReason, _, _, Some(Beyond::Declined { reason })) => {
                        *reason = prose(&new)?;
                    }
                    (CruxField::AnswerBy, _, _, _) => {
                        by = by.renamed(&new).map_err(|_| invalid())?;
                    }
                    _ => return Err(no_such()),
                }
                let mut answer = Answer::new(a.id(), decision, basis, by, a.at());
                if let Some(beyond) = beyond {
                    answer = answer.with_beyond(beyond).map_err(|_| invalid())?;
                }
                self.state = CruxState::Answered(answer);
            }
        }
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
            Crux::restore(
                CruxId::from_uuid(test_uuid(500)),
                same_id,
                CruxState::Open,
                Vec::new()
            ),
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
            Err(ChangeRequestError::UnspecifiedNotAllowed {
                crux: c.id(),
                kind: CruxKind::Model
            })
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
    fn the_option_chosen_is_the_one_the_decision_names_and_none_otherwise() {
        let mut c = raised(CruxKind::Ambiguity);
        assert_eq!(c.chosen_option(), None);
        c.answer(chosen(2)).unwrap();
        assert_eq!(c.chosen_option(), c.option(option_id(2)));
        assert!(c.chosen_option().is_some());
        for decision in [
            Decision::Worded {
                wording: prose("w"),
                reason: None,
            },
            Decision::Unspecified { keep: Vec::new() },
            Decision::Withdrawn,
        ] {
            let mut c = raised(CruxKind::Ambiguity);
            c.answer(answer_of(decision)).unwrap();
            assert_eq!(c.chosen_option(), None);
        }
    }

    #[test]
    fn a_stored_crux_is_checked_like_a_new_one() {
        let id = CruxId::from_uuid(test_uuid(500));
        let state = |a: Answer| CruxState::Answered(a);
        let restore = |d: CruxDraft, s: CruxState| Crux::restore(id, d, s, Vec::new());
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
            Err(ChangeRequestError::UnspecifiedNotAllowed {
                crux: id,
                kind: CruxKind::Model
            })
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
            Crux::restore(
                CruxId::from_uuid(test_uuid(500)),
                same_id,
                CruxState::Open,
                Vec::new()
            ),
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

    // --- correction

    const TOKEN: &str = "tok-SECRET-123";

    fn sought() -> SoughtText {
        SoughtText::try_from(TOKEN).unwrap()
    }

    fn leak(s: &str) -> String {
        format!("{s} {TOKEN}")
    }

    fn correction(fields: Vec<CruxField>) -> Correction<CruxField> {
        Correction::new(
            fields,
            prose("a token was pasted"),
            DecisionMaker::role("lead").unwrap(),
            Timestamp::from_millis(77),
        )
        .unwrap()
    }

    fn free(_: &CruxField, _: &str) -> bool {
        false
    }

    /// A Crux with the token in its question, in two parts of an option and in the recommendation's reason.
    fn leaky_draft() -> CruxDraft {
        let mut d = draft(CruxKind::Ambiguity);
        d.question = prose(&leak("What is"));
        d.options = vec![
            CruxOption::new(
                option_id(1),
                choice(&leak("up")),
                Some(prose(&leak("change"))),
                BTreeMap::from([(detail(), ExpectKind::Keep)]),
                Some(prose(&leak("then"))),
            ),
            CruxOption::new(option_id(2), choice("plain"), None, BTreeMap::new(), None),
        ];
        d.recommended = Some(Recommended::new(
            option_id(1),
            Some(prose(&leak("because"))),
            Some(Confidence::try_from(60).unwrap()),
        ));
        d
    }

    /// An answer worded, with the token in the wording, the reason, the basis, what is declined, and the maker.
    fn leaky_answer() -> Answer {
        Answer::new(
            AnswerId::from_uuid(test_uuid(900)),
            Decision::Worded {
                wording: prose(&leak("wording")),
                reason: Some(prose(&leak("why"))),
            },
            Basis::Arbitrary {
                reason: prose(&leak("basis")),
            },
            DecisionMaker::person(&leak("alice")).unwrap(),
            Timestamp::from_millis(1_000),
        )
        .with_beyond(Beyond::declined(prose(&leak("declined"))))
        .unwrap()
    }

    fn leaky() -> Crux {
        Crux::restore(
            CruxId::from_uuid(test_uuid(500)),
            leaky_draft(),
            CruxState::Answered(leaky_answer()),
            Vec::new(),
        )
        .unwrap()
    }

    fn every_field(c: &Crux) -> Vec<CruxField> {
        c.texts().into_iter().map(|(f, _)| f).collect()
    }

    #[test]
    fn a_crux_lists_every_text_it_holds_in_a_fixed_order() {
        let words: Vec<String> = every_field(&leaky()).iter().map(CruxField::word).collect();
        let o1 = option_id(1);
        let o2 = option_id(2);
        assert_eq!(
            words,
            [
                "question".to_string(),
                format!("option[{o1}].choice"),
                format!("option[{o1}].change"),
                format!("option[{o1}].consequence"),
                format!("option[{o2}].choice"),
                "recommendation.reason".to_string(),
                "decision.wording".to_string(),
                "decision.reason".to_string(),
                "basis.reason".to_string(),
                "beyond.reason".to_string(),
                "answer.by".to_string(),
            ]
        );
        // an open Crux has no answer to list, and an option with nothing more lists its choice only
        let open = raised(CruxKind::Ambiguity);
        assert!(
            every_field(&open)
                .iter()
                .all(|f| !matches!(f, CruxField::AnswerBy | CruxField::DecisionReason))
        );
        assert!(open.corrections().is_empty());
    }

    #[test]
    fn a_correction_reaches_every_field_and_keeps_everything_else_of_the_crux() {
        let mut c = leaky();
        let before = c.clone();
        let leaking: Vec<CruxField> = c
            .texts()
            .into_iter()
            .filter(|(_, t)| t.contains(TOKEN))
            .map(|(f, _)| f)
            .collect();
        c.correct(&sought(), correction(leaking), &free).unwrap();
        assert!(c.texts().iter().all(|(_, t)| !t.contains(TOKEN)));
        assert_eq!(c.question().as_str(), "What is [redacted]");
        assert_eq!(c.options()[0].choice().as_str(), "up [redacted]");
        assert_eq!(
            c.options()[0].change().unwrap().as_str(),
            "change [redacted]"
        );
        assert_eq!(
            c.options()[0].consequence().unwrap().as_str(),
            "then [redacted]"
        );
        assert_eq!(c.options()[0].expects(), before.options()[0].expects());
        assert_eq!(c.options()[1], before.options()[1]);
        let r = c.recommended().unwrap();
        assert_eq!(r.reason().unwrap().as_str(), "because [redacted]");
        assert_eq!(r.option(), option_id(1));
        assert_eq!(r.confidence().map(Confidence::percent), Some(60));
        let a = c.answer_given().unwrap();
        assert_eq!(a.id(), before.answer_given().unwrap().id());
        assert_eq!(a.at(), Timestamp::from_millis(1_000));
        assert!(matches!(a.decision(), Decision::Worded { wording, reason }
            if wording.as_str() == "wording [redacted]" && reason.as_ref().unwrap().as_str() == "why [redacted]"));
        assert!(
            matches!(a.basis(), Basis::Arbitrary { reason } if reason.as_str() == "basis [redacted]")
        );
        assert!(
            matches!(a.beyond(), Some(Beyond::Declined { reason }) if reason.as_str() == "declined [redacted]")
        );
        assert_eq!(a.by().to_string(), "person alice [redacted]");
        // the Crux keeps the correction as its history, and no text
        assert_eq!(c.corrections().len(), 1);
        assert_eq!(c.corrections()[0].by().to_string(), "role lead");
        assert_eq!(c.corrections()[0].at().millis(), 77);
        assert!(!format!("{:?}", c.corrections()).contains("SECRET"));
        assert!(before.corrections().is_empty());
        // what is corrected can be corrected again, with the history kept
        assert_eq!(
            c.correct(&sought(), correction(vec![CruxField::Question]), &free),
            Err(CorrectionError::TextAbsent("question".into()))
        );
        assert_eq!(c.corrections().len(), 1);
    }

    #[test]
    fn a_choice_stays_different_from_the_others_of_its_crux_and_from_what_the_caller_holds() {
        let mut d = leaky_draft();
        // the sibling is already named as the choice would be
        d.options[1] = CruxOption::new(
            option_id(2),
            choice("up [redacted]"),
            None,
            BTreeMap::new(),
            None,
        );
        let mut c = Crux::restore(
            CruxId::from_uuid(test_uuid(500)),
            d,
            CruxState::Answered(leaky_answer()),
            Vec::new(),
        )
        .unwrap();
        let field = CruxField::OptionChoice {
            option: option_id(1),
        };
        c.correct(&sought(), correction(vec![field]), &free)
            .unwrap();
        assert_eq!(c.options()[0].choice().as_str(), "up [redacted 2]");
        assert_eq!(c.options()[1].choice().as_str(), "up [redacted]");
        // and the caller's own list is read too
        let mut c = leaky();
        let held = |f: &CruxField, candidate: &str| {
            matches!(f, CruxField::OptionChoice { .. }) && candidate == "up [redacted]"
        };
        c.correct(&sought(), correction(vec![field]), &held)
            .unwrap();
        assert_eq!(c.options()[0].choice().as_str(), "up [redacted 2]");
        // other fields take the plain mark whatever the caller holds
        let mut c = leaky();
        c.correct(&sought(), correction(vec![CruxField::Question]), &|_, _| {
            true
        })
        .unwrap();
        assert_eq!(c.question().as_str(), "What is [redacted]");
    }

    #[test]
    fn a_choice_that_is_only_the_text_meets_the_mark_another_option_has() {
        // the first choice is the text alone, and another option is called `[redacted]` already
        let mut d = leaky_draft();
        d.options[0] = CruxOption::new(option_id(1), choice(TOKEN), None, BTreeMap::new(), None);
        d.options[1] = CruxOption::new(
            option_id(2),
            choice("[redacted]"),
            None,
            BTreeMap::new(),
            None,
        );
        let mut c = Crux::raise(
            CruxId::from_uuid(test_uuid(501)),
            CruxDraft {
                recommended: None,
                ..d
            },
        )
        .unwrap();
        let field = CruxField::OptionChoice {
            option: option_id(1),
        };
        c.correct(&sought(), correction(vec![field]), &free)
            .unwrap();
        let choices: Vec<&str> = c.options().iter().map(|o| o.choice().as_str()).collect();
        assert_eq!(choices, ["[redacted 2]", "[redacted]"]);
    }

    #[test]
    fn a_correction_of_a_crux_is_all_or_nothing_and_names_only_fields_that_are_there() {
        let mut c = leaky();
        let before = c.clone();
        // the answer was worded: it has no `keep`, and no chosen option
        for field in [
            CruxField::DecisionKeep { keep: 0 },
            CruxField::OptionChange {
                option: option_id(2),
            },
            CruxField::OptionChoice {
                option: option_id(9),
            },
        ] {
            assert_eq!(
                c.correct(
                    &sought(),
                    correction(vec![CruxField::Question, field]),
                    &free
                ),
                Err(CorrectionError::NoSuchField(field.word()))
            );
            assert_eq!(c, before);
        }
        // a field that does not hold the text
        let mut plain = raised(CruxKind::Ambiguity);
        let plain_before = plain.clone();
        assert_eq!(
            plain.correct(&sought(), correction(vec![CruxField::Question]), &free),
            Err(CorrectionError::TextAbsent("question".into()))
        );
        // an open Crux has no maker to correct
        assert_eq!(
            plain.correct(&sought(), correction(vec![CruxField::AnswerBy]), &free),
            Err(CorrectionError::NoSuchField("answer.by".into()))
        );
        assert_eq!(plain, plain_before);
    }

    #[test]
    fn the_reason_of_a_chosen_option_and_what_must_still_hold_are_corrected() {
        let chosen_with_token = Answer::new(
            AnswerId::from_uuid(test_uuid(900)),
            Decision::Chosen {
                option: option_id(1),
                reason: Some(prose(&leak("fairer"))),
            },
            Basis::Arbitrary {
                reason: prose("settled"),
            },
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(5),
        );
        let mut c = Crux::restore(
            CruxId::from_uuid(test_uuid(500)),
            leaky_draft(),
            CruxState::Answered(chosen_with_token),
            Vec::new(),
        )
        .unwrap();
        c.correct(
            &sought(),
            correction(vec![CruxField::DecisionReason]),
            &free,
        )
        .unwrap();
        assert!(
            matches!(c.decision(), Some(Decision::Chosen { option, reason })
            if *option == option_id(1) && reason.as_ref().unwrap().as_str() == "fairer [redacted]")
        );

        let keeps = Answer::new(
            AnswerId::from_uuid(test_uuid(900)),
            Decision::Unspecified {
                keep: vec![prose("stays whole"), prose(&leak("never"))],
            },
            Basis::Arbitrary {
                reason: prose("settled"),
            },
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(5),
        );
        let mut c = Crux::restore(
            CruxId::from_uuid(test_uuid(500)),
            leaky_draft(),
            CruxState::Answered(keeps),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            c.correct(
                &sought(),
                correction(vec![CruxField::DecisionKeep { keep: 0 }]),
                &free
            ),
            Err(CorrectionError::TextAbsent("decision.keep[0]".into()))
        );
        c.correct(
            &sought(),
            correction(vec![CruxField::DecisionKeep { keep: 1 }]),
            &free,
        )
        .unwrap();
        assert!(matches!(c.decision(), Some(Decision::Unspecified { keep })
            if keep[0].as_str() == "stays whole" && keep[1].as_str() == "never [redacted]"));
    }

    #[test]
    fn a_stored_crux_keeps_its_corrections() {
        let mut c = leaky();
        c.correct(&sought(), correction(vec![CruxField::Question]), &free)
            .unwrap();
        let kept = Crux::restore(
            c.id(),
            CruxDraft {
                raised_in: c.raised_in(),
                findings: c.findings().to_vec(),
                kind: c.kind(),
                question: c.question().clone(),
                options: c.options().to_vec(),
                recommended: c.recommended().cloned(),
                importance: c.importance(),
                supersedes: c.supersedes(),
            },
            c.state().clone(),
            c.corrections().to_vec(),
        )
        .unwrap();
        assert_eq!(kept, c);
    }
}
