//! The name a person gives a Context.

use std::fmt;

use thiserror::Error;

/// Why a text is not a Context name.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContextNameError {
    /// There is nothing in it.
    #[error("a context name is empty")]
    Empty,
    /// It has a character other than a lower-case letter, a digit, `-` or `_`.
    #[error("the context name `{name}` has `{ch}`; only a-z, 0-9, `-` and `_` are allowed")]
    BadChar {
        /// The text that was refused.
        name: String,
        /// The first character not allowed.
        ch: char,
    },
}

/// What a person calls a Context, in citations and on the command line. It is an attribute, not the identity
/// (that is the `ContextId`), so it may change. The characters are limited so that a name can never contain the
/// `#` or `::` that a citation uses to end it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContextName(String);

impl ContextName {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ContextName {
    type Error = ContextNameError;

    fn try_from(text: &str) -> Result<ContextName, ContextNameError> {
        if text.is_empty() {
            return Err(ContextNameError::Empty);
        }
        if let Some(ch) = text
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_'))
        {
            return Err(ContextNameError::BadChar {
                name: text.to_string(),
                ch,
            });
        }
        Ok(ContextName(text.to_string()))
    }
}

impl fmt::Display for ContextName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_of_lower_case_digits_dash_and_underscore_is_accepted() {
        assert_eq!(
            ContextName::try_from("core-2_a").unwrap().as_str(),
            "core-2_a"
        );
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert_eq!(ContextName::try_from(""), Err(ContextNameError::Empty));
    }

    #[test]
    fn a_name_with_another_character_is_refused() {
        for bad in ["Core", "a b", "a#b", "a::b", "é"] {
            assert!(
                matches!(
                    ContextName::try_from(bad),
                    Err(ContextNameError::BadChar { .. })
                ),
                "{bad}"
            );
        }
    }
}
