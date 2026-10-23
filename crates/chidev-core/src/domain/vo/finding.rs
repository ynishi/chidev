//! A reference to a finding of a check: which rule, about what.

use std::fmt;

use thiserror::Error;

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
/// a finding, and a Crux raised from one, is filed under.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleId(String);

impl RuleId {
    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for RuleId {
    type Error = RuleIdError;

    fn try_from(text: &str) -> Result<RuleId, RuleIdError> {
        if text.is_empty() {
            return Err(RuleIdError::Empty);
        }
        if let Some(ch) = text.chars().find(|c| {
            !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
        }) {
            return Err(RuleIdError::BadChar {
                id: text.to_string(),
                ch,
            });
        }
        Ok(RuleId(text.to_string()))
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A finding a Crux was raised from: the rule that found it and the definition it is about (none when it is about
/// the Project as a whole). The same rule about the same definition is the same finding, so a check can tell that
/// a Crux decided it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FindingRef {
    rule: RuleId,
    target: Option<Target>,
}

impl FindingRef {
    /// The finding of `rule` about `target`.
    pub fn new(rule: RuleId, target: Option<Target>) -> FindingRef {
        FindingRef { rule, target }
    }

    /// The rule.
    pub fn rule(&self) -> &RuleId {
        &self.rule
    }

    /// The definition it is about.
    pub fn target(&self) -> Option<Target> {
        self.target
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
}
