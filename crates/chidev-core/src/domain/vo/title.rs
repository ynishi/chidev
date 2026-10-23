//! The one line a person reads first about something.

use std::fmt;

use thiserror::Error;

/// Why a text is not a title.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TitleError {
    /// It is empty or only white space.
    #[error("a title is empty")]
    Empty,
    /// It has a line break; a title is one line.
    #[error("a title is one line")]
    MultiLine,
}

/// A title: one line, not blank, without the white space around it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Title(String);

impl Title {
    /// The title as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Title {
    type Error = TitleError;

    fn try_from(text: &str) -> Result<Title, TitleError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(TitleError::Empty);
        }
        if text.contains(['\n', '\r']) {
            return Err(TitleError::MultiLine);
        }
        Ok(Title(text.to_string()))
    }
}

impl fmt::Display for Title {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_kept_without_the_space_around_it() {
        assert_eq!(Title::try_from("  Stack ").unwrap().as_str(), "Stack");
    }

    #[test]
    fn a_blank_or_multi_line_title_is_refused() {
        assert_eq!(Title::try_from(" \t"), Err(TitleError::Empty));
        assert_eq!(Title::try_from("a\nb"), Err(TitleError::MultiLine));
    }
}
