//! The values of a Crux, the question a person settles for a change request: its kind, its options, and the
//! decision taken.

use std::collections::BTreeSet;

use super::choice::Choice;
use super::prose::Prose;
use super::target::Target;

/// What kind of question a Crux is. The kind says whether work may go on with a default until the person answers,
/// and whether the question may be left unspecified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CruxKind {
    /// The text can be read more than one way. Work goes on with a default.
    Ambiguity,
    /// Two promises cannot both hold. There is no default: the person decides.
    Conflict,
    /// The words of the model are in question (one word for two things, two words for one). There is no default,
    /// and it cannot be left unspecified: it is about words, not behaviour.
    Model,
    /// A test and the promise it checks read differently. Work goes on with a default.
    Disagreement,
}

impl CruxKind {
    /// Whether work may go on with a default until the person answers.
    pub fn allows_default(self) -> bool {
        matches!(self, CruxKind::Ambiguity | CruxKind::Disagreement)
    }

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

/// One answer a Crux offers: the choice, the change it makes to the definitions (in words, and as the definitions
/// that must change; none when it leaves them as they are), and what else it affects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CruxOption {
    choice: Choice,
    change: Option<Prose>,
    changes: BTreeSet<Target>,
    consequence: Option<Prose>,
}

impl CruxOption {
    /// An option. `changes` are the definitions that must change when it is chosen, apart from the lines of the
    /// change request, which change whatever is chosen. The default option normally has none.
    pub fn new(
        choice: Choice,
        change: Option<Prose>,
        changes: BTreeSet<Target>,
        consequence: Option<Prose>,
    ) -> CruxOption {
        CruxOption {
            choice,
            change,
            changes,
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

    /// The definitions that must change when it is chosen, in order; empty: it asks no more than the lines.
    pub fn changes(&self) -> &BTreeSet<Target> {
        &self.changes
    }

    /// What else it affects.
    pub fn consequence(&self) -> Option<&Prose> {
        self.consequence.as_ref()
    }
}

/// A choice with the reason it was taken (a default, a recommendation).
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

/// What the person decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// One of the options, and why.
    Chosen(Reasoned),
    /// None of the options: the behaviour in the person's own words, and why.
    Worded {
        /// The behaviour.
        wording: Prose,
        /// Why.
        reason: Option<Prose>,
    },
    /// The behaviour is left free, as long as these still hold (none: the defaults of the Project).
    Unspecified {
        /// What must still hold.
        keep: Vec<Prose>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_ambiguity_or_a_disagreement_has_a_default_and_a_model_question_is_never_unspecified()
    {
        assert!(CruxKind::Ambiguity.allows_default() && CruxKind::Disagreement.allows_default());
        assert!(!CruxKind::Conflict.allows_default() && !CruxKind::Model.allows_default());
        assert!(CruxKind::Conflict.allows_unspecified() && !CruxKind::Model.allows_unspecified());
    }
}
