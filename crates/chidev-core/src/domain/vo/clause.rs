//! The content of a Clause: the label a person cites it by and the sentence it promises.

use std::fmt;

use thiserror::Error;

/// Why a text is not a clause label or a clause sentence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClauseContentError {
    /// The label is not `C` and a number from 1 without a leading zero.
    #[error("`{0}` is not a clause label (`C` and a number from 1)")]
    BadLabel(String),
    /// The sentence is empty or only white space.
    #[error("a clause sentence is empty")]
    EmptyText,
}

/// The label a person writes for a Clause, `C` and a number from 1 (`C1`). It is an attribute, not the identity
/// (that is the `ClauseId`). The number is kept as a number: `C01` is refused rather than read as `C1`, so two
/// spellings of one label cannot both exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClauseLabel(u32);

impl ClauseLabel {
    /// The number after `C`.
    pub fn number(self) -> u32 {
        self.0
    }
}

impl TryFrom<&str> for ClauseLabel {
    type Error = ClauseContentError;

    fn try_from(text: &str) -> Result<ClauseLabel, ClauseContentError> {
        let bad = || ClauseContentError::BadLabel(text.to_string());
        let digits = text.strip_prefix('C').ok_or_else(bad)?;
        if digits.is_empty()
            || digits.starts_with('0')
            || !digits.chars().all(|c| c.is_ascii_digit())
        {
            return Err(bad());
        }
        digits.parse().map(ClauseLabel).map_err(|_| bad())
    }
}

impl fmt::Display for ClauseLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "C{}", self.0)
    }
}

/// The sentence of a Clause, without the white space around it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClauseText(String);

impl ClauseText {
    /// The sentence as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ClauseText {
    type Error = ClauseContentError;

    fn try_from(text: &str) -> Result<ClauseText, ClauseContentError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ClauseContentError::EmptyText);
        }
        Ok(ClauseText(text.to_string()))
    }
}

/// A Clause as written: the name it is known by and the sentence it promises. This is the pair a caller gives to
/// make or change a Contract. Only the sentence is what the Clause promises; the label is its name, so a Clause
/// kept under a new label has the same promise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClauseContent {
    label: ClauseLabel,
    text: ClauseText,
}

impl ClauseContent {
    /// The content of a Clause.
    pub fn new(label: ClauseLabel, text: ClauseText) -> ClauseContent {
        ClauseContent { label, text }
    }

    /// The label.
    pub fn label(&self) -> ClauseLabel {
        self.label
    }

    /// The sentence.
    pub fn text(&self) -> &ClauseText {
        &self.text
    }

    /// The label and the sentence, taken apart.
    pub fn into_parts(self) -> (ClauseLabel, ClauseText) {
        (self.label, self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_c_and_a_number_from_one() {
        assert_eq!(ClauseLabel::try_from("C12").unwrap().number(), 12);
        assert_eq!(ClauseLabel::try_from("C1").unwrap().to_string(), "C1");
    }

    #[test]
    fn another_spelling_of_a_label_is_refused() {
        for bad in [
            "",
            "C",
            "c1",
            "C0",
            "C01",
            "C1x",
            "1",
            "C 1",
            "C99999999999",
        ] {
            assert_eq!(
                ClauseLabel::try_from(bad),
                Err(ClauseContentError::BadLabel(bad.to_string())),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_sentence_is_kept_without_the_space_around_it_and_not_blank() {
        assert_eq!(
            ClauseText::try_from(" pop gives back ").unwrap().as_str(),
            "pop gives back"
        );
        assert_eq!(
            ClauseText::try_from("  "),
            Err(ClauseContentError::EmptyText)
        );
    }
}
