//! One of the answers a Crux offers.

use std::fmt;

use thiserror::Error;

/// Why a text is not a choice.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChoiceError {
    /// It is empty or only white space.
    #[error("a choice is empty")]
    Empty,
}

/// One answer a Crux offers, without the white space around it. An answer is always one of the choices, so it
/// never has to be interpreted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Choice(String);

impl Choice {
    /// The choice as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Choice {
    type Error = ChoiceError;

    fn try_from(text: &str) -> Result<Choice, ChoiceError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ChoiceError::Empty);
        }
        Ok(Choice(text.to_string()))
    }
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_is_kept_without_the_space_around_it_and_not_blank() {
        assert_eq!(
            Choice::try_from(" round half up ").unwrap().as_str(),
            "round half up"
        );
        assert_eq!(Choice::try_from("  "), Err(ChoiceError::Empty));
    }
}
