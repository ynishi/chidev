//! The Crux: a question a person has to answer, with the choices, the default taken until they do, and the
//! definitions that were written on the default.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::choice::Choice;
use crate::domain::vo::id::CruxId;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// Why a Crux cannot be made, or an answer cannot be taken. Nothing is changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CruxError {
    /// There is no choice to make.
    #[error("a crux needs at least one choice")]
    NoChoice,
    /// Two choices are the same.
    #[error("the choice `{0}` is given twice")]
    DuplicateChoice(Choice),
    /// The default is not one of the choices.
    #[error("the default `{0}` is not one of the choices")]
    DefaultNotAChoice(Choice),
    /// The answer is not one of the choices.
    #[error("`{0}` is not one of the choices")]
    NotAChoice(Choice),
}

/// A question and where it stands. Work goes on with the default until the person answers. The definitions
/// written on the default (`lines`: Items and Clauses) are kept with a revision each, so that when the answer
/// differs from the default, whether those definitions have changed since can be seen without asking again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crux {
    id: CruxId,
    title: Title,
    choices: Vec<Choice>,
    default: Choice,
    answer: Option<Choice>,
    lines: BTreeMap<Target, Revision>,
}

impl Crux {
    /// A question with no answer yet: its choices (at least one, none twice), the default among them, and the
    /// definitions written on the default with their revisions now.
    pub fn new(
        id: CruxId,
        title: Title,
        choices: Vec<Choice>,
        default: Choice,
        lines: BTreeMap<Target, Revision>,
    ) -> Result<Crux, CruxError> {
        if choices.is_empty() {
            return Err(CruxError::NoChoice);
        }
        for (k, c) in choices.iter().enumerate() {
            if choices[..k].contains(c) {
                return Err(CruxError::DuplicateChoice(c.clone()));
            }
        }
        if !choices.contains(&default) {
            return Err(CruxError::DefaultNotAChoice(default));
        }
        Ok(Crux {
            id,
            title,
            choices,
            default,
            answer: None,
            lines,
        })
    }

    /// Its id.
    pub fn id(&self) -> CruxId {
        self.id
    }

    /// The question.
    pub fn title(&self) -> &Title {
        &self.title
    }

    /// The choices, in the order given.
    pub fn choices(&self) -> &[Choice] {
        &self.choices
    }

    /// The choice taken until the person answers.
    pub fn default(&self) -> &Choice {
        &self.default
    }

    /// The person's answer, once given.
    pub fn answer(&self) -> Option<&Choice> {
        self.answer.as_ref()
    }

    /// The definitions written on the default, with the revision each had when the Crux was raised or last
    /// answered.
    pub fn lines(&self) -> &BTreeMap<Target, Revision> {
        &self.lines
    }

    /// Whether the person answered something other than the default. An unanswered question does not differ: the
    /// default stands.
    pub fn differs_from_default(&self) -> bool {
        self.answer.as_ref().is_some_and(|a| *a != self.default)
    }

    /// Takes the person's answer; a later one replaces an earlier one. Each line takes the revision in `now`, the
    /// one its definition has as the answer is given (a line missing from `now` keeps its revision: that its
    /// definition is gone is for the caller to report, not a reason to refuse the answer).
    pub fn take_answer(
        &mut self,
        choice: Choice,
        now: &BTreeMap<Target, Revision>,
    ) -> Result<(), CruxError> {
        if !self.choices.contains(&choice) {
            return Err(CruxError::NotAChoice(choice));
        }
        self.answer = Some(choice);
        for (target, revision) in &mut self.lines {
            if let Some(r) = now.get(target) {
                *revision = *r;
            }
        }
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

    fn line() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(30)))
    }

    fn crux() -> Crux {
        Crux::new(
            CruxId::from_uuid(test_uuid(1)),
            Title::try_from("How is a half rounded?").unwrap(),
            vec![choice("up"), choice("to even")],
            choice("up"),
            BTreeMap::from([(line(), Revision::first())]),
        )
        .unwrap()
    }

    #[test]
    fn a_new_crux_has_no_answer_and_does_not_differ() {
        let c = crux();
        assert_eq!(c.answer(), None);
        assert_eq!(c.default().as_str(), "up");
        assert!(!c.differs_from_default());
    }

    #[test]
    fn a_crux_needs_distinct_choices_and_a_default_among_them() {
        let id = CruxId::from_uuid(test_uuid(1));
        let t = || Title::try_from("Q").unwrap();
        assert_eq!(
            Crux::new(id, t(), Vec::new(), choice("a"), BTreeMap::new()),
            Err(CruxError::NoChoice)
        );
        assert_eq!(
            Crux::new(
                id,
                t(),
                vec![choice("a"), choice("a")],
                choice("a"),
                BTreeMap::new()
            ),
            Err(CruxError::DuplicateChoice(choice("a")))
        );
        assert_eq!(
            Crux::new(id, t(), vec![choice("a")], choice("b"), BTreeMap::new()),
            Err(CruxError::DefaultNotAChoice(choice("b")))
        );
    }

    #[test]
    fn an_answer_is_one_of_the_choices_and_takes_the_revisions_now() {
        let mut c = crux();
        let now = BTreeMap::from([(line(), Revision::first().next())]);
        assert_eq!(
            c.take_answer(choice("down"), &now),
            Err(CruxError::NotAChoice(choice("down")))
        );
        assert_eq!(c.lines()[&line()], Revision::first());
        c.take_answer(choice("to even"), &now).unwrap();
        assert!(c.differs_from_default());
        assert_eq!(c.lines()[&line()], Revision::first().next());
        c.take_answer(choice("up"), &BTreeMap::new()).unwrap();
        assert!(!c.differs_from_default());
        assert_eq!(c.lines()[&line()], Revision::first().next());
    }
}
