//! The answer to a Crux: what was decided, on what basis, by whom, and when. It is an external input that cannot be
//! reproduced, so it is recorded whole and never rewritten, and it has an id of its own so that a later record can
//! point at the decision.

use std::fmt;

use thiserror::Error;

use super::crux::Decision;
use super::id::{AnswerId, ChangeRequestId};
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

/// Why a list of change requests is not a `beyond` an answer can carry.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BeyondError {
    /// The list is empty.
    #[error("`beyond` names no change request")]
    Empty,
    /// A change request is in the list twice.
    #[error("the change request {0} is named twice in `beyond`")]
    Twice(ChangeRequestId),
}

/// Where what an answer adds outside the change request it was given in goes. An answer may choose an option that
/// names targets the change request does not frame, and the change request does not take them (its frame is what it
/// was opened with), so the answer says what became of them. It is part of the answer: written with it, never
/// rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Beyond {
    /// Change requests opened for it, each framing the targets that are outside. Build it with
    /// [`Beyond::taken_by`], which refuses a list that is empty or names a change request twice.
    TakenBy(Vec<ChangeRequestId>),
    /// It is not done.
    Declined {
        /// Why.
        reason: Prose,
    },
}

impl Beyond {
    /// `Beyond::TakenBy` of `requests`, which are at least one and none twice.
    pub fn taken_by(requests: Vec<ChangeRequestId>) -> Result<Beyond, BeyondError> {
        if requests.is_empty() {
            return Err(BeyondError::Empty);
        }
        for (i, id) in requests.iter().enumerate() {
            if requests[..i].contains(id) {
                return Err(BeyondError::Twice(*id));
            }
        }
        Ok(Beyond::TakenBy(requests))
    }

    /// `Beyond::Declined`, with the reason it is not done.
    pub fn declined(reason: Prose) -> Beyond {
        Beyond::Declined { reason }
    }
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
///
/// The authority to decide (`CruxAnswerable`) is a concept of the model's words, and it is managed outside chidev:
/// chidev holds no identity of its users, so it neither grants nor checks it. What it keeps is this claim of who
/// decided.
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

/// The answer to a Crux: its id, the decision, the basis it rests on, who decided it, and when. An answer is
/// immutable; a different decision is a new Crux that supersedes the one answered. It may say where what it adds
/// outside its change request goes ([`Beyond`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    id: AnswerId,
    decision: Decision,
    basis: Basis,
    by: DecisionMaker,
    at: Timestamp,
    beyond: Option<Beyond>,
}

impl Answer {
    /// An answer that says nothing of what lies outside its change request.
    pub fn new(
        id: AnswerId,
        decision: Decision,
        basis: Basis,
        by: DecisionMaker,
        at: Timestamp,
    ) -> Answer {
        Answer {
            id,
            decision,
            basis,
            by,
            at,
            beyond: None,
        }
    }

    /// This answer, saying where what it adds outside its change request goes.
    pub fn with_beyond(mut self, beyond: Beyond) -> Answer {
        self.beyond = Some(beyond);
        self
    }

    /// Its id.
    pub fn id(&self) -> AnswerId {
        self.id
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

    /// Where what it adds outside its change request goes, when it says.
    pub fn beyond(&self) -> Option<&Beyond> {
        self.beyond.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;

    fn request(n: u128) -> ChangeRequestId {
        ChangeRequestId::from_uuid(test_uuid(n))
    }

    #[test]
    fn a_beyond_names_at_least_one_change_request_and_none_twice() {
        assert_eq!(Beyond::taken_by(Vec::new()), Err(BeyondError::Empty));
        assert_eq!(
            Beyond::taken_by(vec![request(1), request(2), request(1)]),
            Err(BeyondError::Twice(request(1)))
        );
        assert_eq!(
            Beyond::taken_by(vec![request(2), request(1)]),
            Ok(Beyond::TakenBy(vec![request(2), request(1)]))
        );
    }

    #[test]
    fn an_answer_says_nothing_beyond_until_it_is_given_a_beyond() {
        let answer = Answer::new(
            AnswerId::from_uuid(test_uuid(900)),
            Decision::Withdrawn,
            Basis::Arbitrary {
                reason: Prose::try_from("settled").unwrap(),
            },
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(5),
        );
        assert_eq!(answer.beyond(), None);
        let declined = Beyond::declined(Prose::try_from("not needed").unwrap());
        assert_eq!(
            answer.with_beyond(declined.clone()).beyond(),
            Some(&declined)
        );
    }

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
