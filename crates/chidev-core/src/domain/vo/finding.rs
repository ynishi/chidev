//! A reference to a finding of a check: which rule, about what.

use std::borrow::Cow;
use std::fmt;

use thiserror::Error;

use super::id::{CaseId, ChangeRequestId, ClauseId, ContextId, InterfaceId, ItemId};
use super::target::Target;

/// Why a text is not a rule id.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleIdError {
    /// It is empty.
    #[error("a rule id is empty")]
    Empty,
    /// It has a character other than a lower-case letter, a digit, `-`, `_` or `.`.
    #[error("the rule id `{id}` has `{ch}`; only a-z, 0-9, `-`, `_` and `.` are allowed")]
    BadChar {
        /// The text refused.
        id: String,
        /// The first character not allowed.
        ch: char,
    },
}

/// The id of a rule a check applies (`uncited-clause`). The rules themselves come with the checks; the id is what
/// a finding, and a change request raised from one, is filed under.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleId(Cow<'static, str>);

/// The byte of `text` that a rule id may not have, if any. One definition for the runtime check and the
/// compile-time one; a byte outside ASCII is never allowed, so a position here is the start of a character.
const fn first_bad_byte(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut k = 0;
    while k < bytes.len() {
        let b = bytes[k];
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')) {
            return Some(k);
        }
        k += 1;
    }
    None
}

impl RuleId {
    /// The id of a rule that comes with the code, for a `static` descriptor. A bad id is a compile error (the
    /// check is evaluated where the static is), never a failure at run time.
    pub const fn from_static(text: &'static str) -> RuleId {
        assert!(
            !text.is_empty() && first_bad_byte(text).is_none(),
            "a rule id is a-z, 0-9, `-`, `_` and `.`, and not empty"
        );
        RuleId(Cow::Borrowed(text))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this id starts with `prefix` (a family of rules: `cite.`).
    pub fn in_family(&self, prefix: &str) -> bool {
        self.0.starts_with(prefix)
    }
}

impl TryFrom<&str> for RuleId {
    type Error = RuleIdError;

    fn try_from(text: &str) -> Result<RuleId, RuleIdError> {
        if text.is_empty() {
            return Err(RuleIdError::Empty);
        }
        if let Some(at) = first_bad_byte(text)
            && let Some(ch) = text[at..].chars().next()
        {
            return Err(RuleIdError::BadChar {
                id: text.to_string(),
                ch,
            });
        }
        Ok(RuleId(Cow::Owned(text.to_string())))
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a finding is about, by id. A citation or a change request line points at an Item or a Clause ([`Target`]);
/// a finding can also be about a thing that is neither (an Interface with no Pin, a Context with no Harness), so
/// the subject has more kinds than a [`Target`], which stays what a line may point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Subject {
    /// A Context.
    Context(ContextId),
    /// An Interface.
    Interface(InterfaceId),
    /// An Item of an Interface.
    Item(ItemId),
    /// A Clause of a Contract.
    Clause(ClauseId),
    /// A Case of a Harness.
    Case(CaseId),
    /// A change request.
    ChangeRequest(ChangeRequestId),
}

impl From<Target> for Subject {
    fn from(t: Target) -> Subject {
        match t {
            Target::Item(i) => Subject::Item(i),
            Target::Clause(c) => Subject::Clause(c),
        }
    }
}

/// A finding a change request was raised from: the rule that found it and the thing it is about (none when it is
/// about the Project as a whole). The same rule about the same thing is the same finding, so a change request can
/// say that it deals with it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FindingRef {
    rule: RuleId,
    subject: Option<Subject>,
}

impl FindingRef {
    /// The finding of `rule` about `subject`.
    pub fn new(rule: RuleId, subject: Option<Subject>) -> FindingRef {
        FindingRef { rule, subject }
    }

    /// The rule.
    pub fn rule(&self) -> &RuleId {
        &self.rule
    }

    /// The thing it is about.
    pub fn subject(&self) -> Option<Subject> {
        self.subject
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_id_is_lower_case_digits_and_dash_underscore_dot() {
        assert_eq!(
            RuleId::try_from("uncited-clause.v1").unwrap().as_str(),
            "uncited-clause.v1"
        );
        assert_eq!(RuleId::try_from(""), Err(RuleIdError::Empty));
        assert!(matches!(
            RuleId::try_from("Uncited"),
            Err(RuleIdError::BadChar { ch: 'U', .. })
        ));
    }

    #[test]
    fn a_static_rule_id_is_the_same_as_one_parsed() {
        const ID: RuleId = RuleId::from_static("cite.stale");
        assert_eq!(ID, RuleId::try_from("cite.stale").unwrap());
        assert!(ID.in_family("cite."));
        assert!(!ID.in_family("pin."));
    }

    #[test]
    fn a_finding_is_the_same_for_the_same_rule_and_subject() {
        let c = Subject::Clause(ClauseId::from_uuid(super::super::id::test_uuid(1)));
        let a = FindingRef::new(RuleId::from_static("x"), Some(c));
        assert_eq!(a, FindingRef::new(RuleId::try_from("x").unwrap(), Some(c)));
        assert_ne!(a, FindingRef::new(RuleId::from_static("x"), None));
    }
}
