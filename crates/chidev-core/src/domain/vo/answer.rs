//! The answer to a Crux: what was decided, on what basis, by whom, and when. It is an external input that cannot be
//! reproduced, so it is recorded whole and never rewritten.

use std::fmt;

use thiserror::Error;

use super::crux::Decision;
use super::policy::PolicyLineId;
use super::prose::Prose;
use super::revision::Revision;
use super::timestamp::Timestamp;

/// What a decision rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Basis {
    /// A line of the Project's decision Policy settles it. The revision of the Policy when the line was applied is
    /// kept, so that a later change of the line is seen.
    Policy {
        /// The line applied.
        line: PolicyLineId,
        /// The revision of the Policy when it was applied.
        policy_revision: Revision,
    },
    /// Nothing derives it: an intentional, arbitrary decision, and why it went this way.
    Arbitrary {
        /// Why.
        reason: Prose,
    },
}

/// Why a text is not the name of a decision maker.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecisionMakerError {
    /// It is empty or only white space.
    #[error("the name of a decision maker is empty")]
    Empty,
}

/// The name of a role or of a person, without the white space around it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MakerName(String);

impl MakerName {
    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for MakerName {
    type Error = DecisionMakerError;

    fn try_from(text: &str) -> Result<MakerName, DecisionMakerError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(DecisionMakerError::Empty);
        }
        Ok(MakerName(text.to_string()))
    }
}

impl fmt::Display for MakerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who decided: a role that holds the authority to decide that change request or finding (an orchestrator, a lead
/// of a harness), or a person. The domain checks nothing about the name; it is the record of who answered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DecisionMaker {
    /// A role with the authority to decide.
    Role(MakerName),
    /// A person.
    Person(MakerName),
}

impl DecisionMaker {
    /// A role, by name.
    pub fn role(name: &str) -> Result<DecisionMaker, DecisionMakerError> {
        MakerName::try_from(name).map(DecisionMaker::Role)
    }

    /// A person, by name.
    pub fn person(name: &str) -> Result<DecisionMaker, DecisionMakerError> {
        MakerName::try_from(name).map(DecisionMaker::Person)
    }

    /// The name.
    pub fn name(&self) -> &MakerName {
        match self {
            DecisionMaker::Role(n) | DecisionMaker::Person(n) => n,
        }
    }

    /// The word for what it is: `role` or `person`.
    pub fn word(&self) -> &'static str {
        match self {
            DecisionMaker::Role(_) => "role",
            DecisionMaker::Person(_) => "person",
        }
    }
}

impl fmt::Display for DecisionMaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.word(), self.name())
    }
}

/// The answer to a Crux: the decision, the basis it rests on, who decided it, and when. An answer is immutable;
/// a different decision is a new Crux that supersedes the one answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    decision: Decision,
    basis: Basis,
    by: DecisionMaker,
    at: Timestamp,
}

impl Answer {
    /// An answer.
    pub fn new(decision: Decision, basis: Basis, by: DecisionMaker, at: Timestamp) -> Answer {
        Answer {
            decision,
            basis,
            by,
            at,
        }
    }

    /// What was decided.
    pub fn decision(&self) -> &Decision {
        &self.decision
    }

    /// What it rests on.
    pub fn basis(&self) -> &Basis {
        &self.basis
    }

    /// Who decided.
    pub fn by(&self) -> &DecisionMaker {
        &self.by
    }

    /// When.
    pub fn at(&self) -> Timestamp {
        self.at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decision_maker_is_a_role_or_a_person_with_a_name_that_is_not_blank() {
        let role = DecisionMaker::role(" orchestrator ").unwrap();
        assert_eq!(role.name().as_str(), "orchestrator");
        assert_eq!(role.to_string(), "role orchestrator");
        assert_eq!(DecisionMaker::person("alice").unwrap().word(), "person");
        assert_eq!(DecisionMaker::role("  "), Err(DecisionMakerError::Empty));
        assert_eq!(DecisionMaker::person(""), Err(DecisionMakerError::Empty));
        // The same name as a role and as a person are two different makers.
        assert_ne!(
            DecisionMaker::role("alice").unwrap(),
            DecisionMaker::person("alice").unwrap()
        );
    }
}
