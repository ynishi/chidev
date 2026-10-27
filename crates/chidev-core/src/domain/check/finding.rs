//! What a check found.

use super::descriptor::Descriptor;
use super::level::Level;
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::service::pin::StaleReason;
use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
use crate::domain::vo::id::{ContractId, HarnessId, InterfaceId};
use crate::domain::vo::policy::PolicyLineId;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

/// Whether a finding is being dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FindingState {
    /// No change request addresses it.
    Open,
    /// A change request that is not resolved addresses it.
    Addressed,
    /// Only change requests that are resolved address it: it was decided, and the check still finds it.
    Decided,
}

impl FindingState {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            FindingState::Open => "open",
            FindingState::Addressed => "addressed",
            FindingState::Decided => "decided",
        }
    }
}

/// A thing a finding's message is about besides its subject, by id. The message of a finding has no names (the
/// store keeps them, not the domain), so a finding that has to say which of several things moved gives the things
/// and the caller who has the names writes them into the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mention {
    /// An Interface.
    Interface(InterfaceId),
    /// A Harness.
    Harness(HarnessId),
    /// A Contract.
    Contract(ContractId),
}

/// Whether a policy line a decision rests on is gone, or still there with another text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineState {
    /// The Policy no longer has the line.
    Gone,
    /// The line has another text than when it was applied. Holds the revision of the Policy at which the line got
    /// the text it has now (the line's `changed_at`, not the revision of the Policy as a whole, which goes up when
    /// any line changes).
    Reworded(Revision),
}

/// The mark of one fact that makes a finding true. Two runs of a check that find the same rule about the same
/// subject (the same [`FindingRef`]) are told apart by their grounds: a put that leaves every ground of a finding
/// as it was has introduced nothing, and one that adds a ground, or moves the mark of one, has introduced that
/// part of it. A ground is the mark of a fact, never a copy of the message: the wording can change and the fact
/// not.
///
/// A check about the state of the whole model, not about what a change did to it, gives no grounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ground {
    /// A citation of a Case whose target is at revision `now`, past the one the Case recorded.
    Cite {
        /// What the Case cites.
        target: Target,
        /// The revision the target has.
        now: Revision,
    },
    /// Something a Pin stands on moved.
    Pin {
        /// What moved.
        reason: StaleReason,
        /// The revision of what moved now; `None` when it is gone.
        now: Option<Revision>,
    },
    /// A target a change request keeps has moved.
    Moved {
        /// The target.
        target: Target,
        /// The revision it has now; `None` when it is gone.
        now: Option<Revision>,
    },
    /// The work of a change request moved a target outside its frame, in the change set kept at the store
    /// revision `at`. A footprint is a fact about the past that never changes, so another change set moving the
    /// same target is another ground.
    Work {
        /// The target it moved.
        target: Target,
        /// The revision of the store the change set was kept at.
        at: u64,
    },
    /// A policy line a decision rests on.
    PolicyLine {
        /// The line.
        line: PolicyLineId,
        /// Whether it is gone or reworded.
        state: LineState,
    },
}

/// One thing a check found: the rule and what it is about (the [`FindingRef`], which is also how a change request
/// points at it), how serious it is, and what is wrong in words. The words carry no names the store keeps (those
/// are not the domain's); the reader of the finding has the subject to look them up by. The grounds are the marks
/// of the facts it stands on (see [`Ground`]); they are not part of what makes it the same finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    at: FindingRef,
    level: Level,
    message: String,
    grounds: Vec<Ground>,
}

impl Finding {
    /// A finding of the check `d` about `subject`, at the level `d` has by default. The runner sets the level the
    /// policy gives.
    pub fn new(d: &Descriptor, subject: Option<Subject>, message: impl Into<String>) -> Finding {
        Finding {
            at: FindingRef::new(d.id.clone(), subject),
            level: d.default_level,
            message: message.into(),
            grounds: Vec::new(),
        }
    }

    /// The same finding, standing on `grounds`.
    pub fn grounded(mut self, grounds: Vec<Ground>) -> Finding {
        self.grounds = grounds;
        self
    }

    /// The marks of the facts it stands on, in order; empty for a check that gives none.
    pub fn grounds(&self) -> &[Ground] {
        &self.grounds
    }

    /// What the message goes on to name, in order; empty for most findings. Derived from the grounds, so that
    /// what a Pin moved on is held once: a `Pin` ground names the thing that moved (the pinned Interface is the
    /// subject of the finding).
    pub fn mentions(&self) -> Vec<Mention> {
        self.grounds
            .iter()
            .filter_map(|g| match g {
                Ground::Pin { reason, .. } => match reason {
                    StaleReason::Interface => match self.subject() {
                        Some(Subject::Interface(id)) => Some(Mention::Interface(id)),
                        _ => None,
                    },
                    StaleReason::Harness(id) => Some(Mention::Harness(*id)),
                    StaleReason::Contract(id) => Some(Mention::Contract(*id)),
                    StaleReason::UsedInterface(id) => Some(Mention::Interface(*id)),
                },
                _ => None,
            })
            .collect()
    }

    /// The same finding at `level`.
    pub fn at_level(mut self, level: Level) -> Finding {
        self.level = level;
        self
    }

    /// The rule and the subject.
    pub fn at(&self) -> &FindingRef {
        &self.at
    }

    /// The rule.
    pub fn rule(&self) -> &RuleId {
        self.at.rule()
    }

    /// What it is about.
    pub fn subject(&self) -> Option<Subject> {
        self.at.subject()
    }

    /// How serious it is.
    pub fn level(&self) -> Level {
        self.level
    }

    /// What is wrong, in words.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether `change_requests` deal with it. A change request not yet resolved makes it addressed, even when
    /// another one, resolved, addressed it before: a new one is being worked on. Only resolved ones make it decided.
    pub fn state_in(&self, change_requests: &[ChangeRequest]) -> FindingState {
        let mut state = FindingState::Open;
        for cr in change_requests
            .iter()
            .filter(|c| c.addresses().contains(&self.at))
        {
            if !cr.is_resolved() {
                return FindingState::Addressed;
            }
            state = FindingState::Decided;
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::domain::check::descriptor::Executor;
    use crate::domain::check::level::Moment;
    use crate::domain::entity::change_request::ChangeRequestDraft;
    use crate::domain::vo::answer::DecisionMaker;
    use crate::domain::vo::id::{ChangeRequestId, ClauseId, test_uuid};
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::resolution::{Resolution, ResolutionKind};
    use crate::domain::vo::timestamp::Timestamp;
    use crate::domain::vo::title::Title;
    use std::collections::BTreeMap;

    static RULE: Descriptor = Descriptor {
        id: RuleId::from_static("t.rule"),
        summary: Cow::Borrowed("s"),
        help: Cow::Borrowed("h"),
        moments: Cow::Borrowed(&[Moment::Check]),
        default_level: Level::Note,
        mandatory: false,
        params: std::borrow::Cow::Borrowed(&[]),
        executor: Executor::Builtin,
    };

    fn finding(n: u128) -> Finding {
        Finding::new(
            &RULE,
            Some(Subject::Clause(ClauseId::from_uuid(test_uuid(n)))),
            "m",
        )
    }

    fn cr(n: u128, addresses: Vec<FindingRef>, resolved: bool) -> ChangeRequest {
        let mut c = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(n)),
            ChangeRequestDraft {
                title: Title::try_from(format!("cr {n}").as_str()).unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: BTreeMap::new(),
                assumptions: Vec::new(),
                addresses,
            },
        );
        if resolved {
            c.resolve(Resolution::new(
                ResolutionKind::Done,
                Prose::try_from("settled").unwrap(),
                DecisionMaker::person("alice").unwrap(),
                Timestamp::from_millis(1),
            ))
            .unwrap();
        }
        c
    }

    #[test]
    fn a_new_finding_has_its_rules_default_level_and_the_runner_may_change_it() {
        assert_eq!(finding(1).level(), Level::Note);
        assert_eq!(finding(1).at_level(Level::Error).level(), Level::Error);
        assert_eq!(finding(1).rule().as_str(), "t.rule");
    }

    #[test]
    fn a_finding_is_open_addressed_or_decided_by_the_change_requests_that_name_it() {
        let f = finding(1);
        let named = vec![f.at().clone()];
        let other = vec![finding(2).at().clone()];
        assert_eq!(f.state_in(&[]), FindingState::Open);
        assert_eq!(
            f.state_in(&[cr(1, other.clone(), false)]),
            FindingState::Open
        );
        assert_eq!(
            f.state_in(&[cr(1, named.clone(), false)]),
            FindingState::Addressed
        );
        assert_eq!(
            f.state_in(&[cr(1, named.clone(), true)]),
            FindingState::Decided
        );
        // A resolved one and one still being worked: it is being worked.
        assert_eq!(
            f.state_in(&[cr(1, named.clone(), true), cr(2, named, false)]),
            FindingState::Addressed
        );
    }
}
