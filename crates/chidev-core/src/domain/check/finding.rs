//! What a check found.

use super::descriptor::Descriptor;
use super::level::Level;
use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestState};
use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
use crate::domain::vo::id::{ContractId, HarnessId, InterfaceId};

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

/// One thing a check found: the rule and what it is about (the [`FindingRef`], which is also how a change request
/// points at it), how serious it is, and what is wrong in words. The words carry no names the store keeps (those
/// are not the domain's); the reader of the finding has the subject to look them up by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    at: FindingRef,
    level: Level,
    message: String,
    mentions: Vec<Mention>,
}

impl Finding {
    /// A finding of the check `d` about `subject`, at the level `d` has by default. The runner sets the level the
    /// policy gives.
    pub fn new(d: &Descriptor, subject: Option<Subject>, message: impl Into<String>) -> Finding {
        Finding {
            at: FindingRef::new(d.id.clone(), subject),
            level: d.default_level,
            message: message.into(),
            mentions: Vec::new(),
        }
    }

    /// The same finding, whose message goes on to name `mentions`.
    pub fn mentioning(mut self, mentions: Vec<Mention>) -> Finding {
        self.mentions = mentions;
        self
    }

    /// What the message goes on to name, in order; empty for most findings.
    pub fn mentions(&self) -> &[Mention] {
        &self.mentions
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
            if cr.state() != ChangeRequestState::Resolved {
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
    use crate::domain::check::level::Hook;
    use crate::domain::entity::change_request::{ChangeRequestDraft, Route};
    use crate::domain::vo::id::{ChangeRequestId, ClauseId, test_uuid};
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::title::Title;
    use std::collections::BTreeMap;

    static RULE: Descriptor = Descriptor {
        id: RuleId::from_static("t.rule"),
        summary: Cow::Borrowed("s"),
        help: Cow::Borrowed("h"),
        hooks: Cow::Borrowed(&[Hook::Check]),
        default_level: Level::Note,
        mandatory: false,
        params: &[],
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
                lines: BTreeMap::new(),
                route: Route::Machine,
                addresses,
            },
        );
        if resolved {
            c.resolve().unwrap();
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
