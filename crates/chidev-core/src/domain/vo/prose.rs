//! Free text a person writes to explain something: why a question came up, why an answer was chosen.

use std::fmt;

use thiserror::Error;

/// Why a text is not prose.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProseError {
    /// It is empty or only white space.
    #[error("the text is empty")]
    Empty,
}

/// Text that explains, one line or many, without the white space around it. The domain keeps it and does not read
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Prose(String);

impl Prose {
    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Prose {
    type Error = ProseError;

    fn try_from(text: &str) -> Result<Prose, ProseError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ProseError::Empty);
        }
        Ok(Prose(text.to_string()))
    }
}

impl fmt::Display for Prose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_is_kept_without_the_space_around_it_and_not_blank() {
        assert_eq!(
            Prose::try_from("\n because\nof this \n").unwrap().as_str(),
            "because\nof this"
        );
        assert_eq!(Prose::try_from(" \n "), Err(ProseError::Empty));
    }
}
