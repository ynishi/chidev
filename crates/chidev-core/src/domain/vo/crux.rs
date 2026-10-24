//! The values of a Crux, a choice that can be derived neither from the repository and general knowledge (that is an
//! Assumption) nor from the Project's decision Policy, so a decision maker must pick: its kind, its options, the
//! recommendation and importance that let a Policy settle it, and the decision taken.

use std::collections::BTreeMap;

use super::choice::Choice;
use super::confidence::Confidence;
use super::prose::Prose;
use super::target::{ExpectKind, Target};

/// What kind of question a Crux is. The kind says whether the question may be left unspecified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CruxKind {
    /// The text can be read more than one way.
    Ambiguity,
    /// Two promises cannot both hold.
    Conflict,
    /// The words of the model are in question (one word for two things, two words for one). It cannot be left
    /// unspecified: it is about words, not behaviour.
    Model,
    /// A test and the promise it checks read differently.
    Disagreement,
}

impl CruxKind {
    /// Whether the question may be left unspecified (the behaviour left free, within invariants).
    pub fn allows_unspecified(self) -> bool {
        self != CruxKind::Model
    }

    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            CruxKind::Ambiguity => "ambiguity",
            CruxKind::Conflict => "conflict",
            CruxKind::Model => "model",
            CruxKind::Disagreement => "disagreement",
        }
    }
}

/// One answer a Crux offers: the choice, the change it makes to the definitions (in words, and as the targets it
/// names: to revise or retire, or to keep; none when it names nothing more), and what else it affects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CruxOption {
    choice: Choice,
    change: Option<Prose>,
    expects: BTreeMap<Target, ExpectKind>,
    consequence: Option<Prose>,
}

impl CruxOption {
    /// An option. `expects` is what the option asks of targets once it is chosen, besides what the change request
    /// asks, which holds whatever is chosen (one kind for each target).
    pub fn new(
        choice: Choice,
        change: Option<Prose>,
        expects: BTreeMap<Target, ExpectKind>,
        consequence: Option<Prose>,
    ) -> CruxOption {
        CruxOption {
            choice,
            change,
            expects,
            consequence,
        }
    }

    /// The choice.
    pub fn choice(&self) -> &Choice {
        &self.choice
    }

    /// The change it makes to the definitions (the exact new wording, ideally); `None`: it changes none.
    pub fn change(&self) -> Option<&Prose> {
        self.change.as_ref()
    }

    /// What it asks of targets when it is chosen; empty: it asks no more than the change request does.
    pub fn expects(&self) -> &BTreeMap<Target, ExpectKind> {
        &self.expects
    }

    /// What else it affects.
    pub fn consequence(&self) -> Option<&Prose> {
        self.consequence.as_ref()
    }
}

/// How much the answer matters. A Policy can settle a Crux of low importance by its recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Importance {
    /// The options are close to equal in what they cost.
    Low,
    /// The usual.
    Normal,
    /// The answer shapes much that follows.
    High,
}

impl Importance {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Importance::Low => "low",
            Importance::Normal => "normal",
            Importance::High => "high",
        }
    }

    /// The importance a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<Importance> {
        [Importance::Low, Importance::Normal, Importance::High]
            .into_iter()
            .find(|i| i.word() == word)
    }
}

/// The option the one who raised the Crux recommends, why, and how sure they are. Derived from the inside, so it
/// is not a decision: a Policy line may take it, a decision maker may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommended {
    choice: Choice,
    reason: Option<Prose>,
    confidence: Option<Confidence>,
}

impl Recommended {
    /// A recommendation.
    pub fn new(
        choice: Choice,
        reason: Option<Prose>,
        confidence: Option<Confidence>,
    ) -> Recommended {
        Recommended {
            choice,
            reason,
            confidence,
        }
    }

    /// The choice recommended.
    pub fn choice(&self) -> &Choice {
        &self.choice
    }

    /// Why.
    pub fn reason(&self) -> Option<&Prose> {
        self.reason.as_ref()
    }

    /// How sure, from 0 to 100; none: not said.
    pub fn confidence(&self) -> Option<Confidence> {
        self.confidence
    }
}

/// A choice with the reason it was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reasoned {
    choice: Choice,
    reason: Option<Prose>,
}

impl Reasoned {
    /// A choice and why.
    pub fn new(choice: Choice, reason: Option<Prose>) -> Reasoned {
        Reasoned { choice, reason }
    }

    /// The choice.
    pub fn choice(&self) -> &Choice {
        &self.choice
    }

    /// Why.
    pub fn reason(&self) -> Option<&Prose> {
        self.reason.as_ref()
    }
}

/// What was decided, apart from the basis, the decision maker and the time (see `Answer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// One of the options, and why.
    Chosen(Reasoned),
    /// None of the options: the behaviour in the decision maker's own words, and why.
    Worded {
        /// The behaviour.
        wording: Prose,
        /// Why.
        reason: Option<Prose>,
    },
    /// The behaviour is left free, as long as these still hold (none: nothing more).
    Unspecified {
        /// What must still hold.
        keep: Vec<Prose>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_question_is_never_unspecified() {
        assert!(CruxKind::Ambiguity.allows_unspecified());
        assert!(CruxKind::Conflict.allows_unspecified());
        assert!(CruxKind::Disagreement.allows_unspecified());
        assert!(!CruxKind::Model.allows_unspecified());
    }

    #[test]
    fn an_importance_is_one_of_three_words() {
        for i in [Importance::Low, Importance::Normal, Importance::High] {
            assert_eq!(Importance::from_word(i.word()), Some(i));
        }
        assert_eq!(Importance::from_word("urgent"), None);
    }

    #[test]
    fn a_recommendation_keeps_its_confidence() {
        let r = Recommended::new(
            Choice::try_from("up").unwrap(),
            None,
            Some(Confidence::try_from(85).unwrap()),
        );
        assert_eq!(r.confidence().map(Confidence::percent), Some(85));
        assert_eq!(r.choice().as_str(), "up");
        assert!(r.reason().is_none());
    }
}
