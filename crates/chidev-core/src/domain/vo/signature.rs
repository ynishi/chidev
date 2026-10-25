//! The declaration of an Item of an Interface, taken from its code.

use std::fmt;

use thiserror::Error;

use super::line_range::LineRange;
use super::source::Source;

/// Why the lines of a code are not a signature.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignatureError {
    /// The lines go past the last line of the code.
    #[error("the lines are not all in the code")]
    Outside,
    /// The lines are empty or only white space.
    #[error("the lines are blank")]
    Blank,
}

/// The declaration of an Item (`fn push(&mut self, v: T)`), as the lines of the code it is on say it. It is content,
/// not the identity (that is the `ItemId`, and the key it is matched by): it is never written by a person, only
/// derived from the code by [`Signature::of_lines`], so that it cannot differ from what the code says. The domain
/// does not read it, so it holds no rule about what a name or a path in it looks like.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature(String);

impl Signature {
    /// The signature of the Item on `lines` of `source`: those lines as they are, each without the white space at
    /// its end, joined by a line break. The lines are the caller's to choose: they point at the declaration alone
    /// (from its first token, after any doc comment and attribute, to the line that ends it with `;` or the `{` of
    /// its body), and a doc comment among them is part of the signature, so that editing it revises the Item.
    pub fn of_lines(source: &Source, lines: LineRange) -> Result<Signature, SignatureError> {
        let text = source.lines(lines).ok_or(SignatureError::Outside)?;
        let text = text
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            return Err(SignatureError::Blank);
        }
        Ok(Signature(text))
    }

    /// The declaration as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(first: usize, last: usize) -> LineRange {
        LineRange::new(first, last).unwrap()
    }

    fn of(code: &str, first: usize, last: usize) -> Result<Signature, SignatureError> {
        Signature::of_lines(&Source::new(code.to_string()), range(first, last))
    }

    #[test]
    fn one_line_is_taken_as_it_is() {
        let s = of("fn a()\npub fn push(&mut self, v: T);\n", 2, 2).unwrap();
        assert_eq!(s.as_str(), "pub fn push(&mut self, v: T);");
    }

    #[test]
    fn several_lines_are_joined_by_a_line_break_and_keep_their_indentation() {
        let code = "pub fn push(\n    &mut self,\n    v: T,\n) -> bool;\n";
        assert_eq!(
            of(code, 1, 4).unwrap().as_str(),
            "pub fn push(\n    &mut self,\n    v: T,\n) -> bool;"
        );
    }

    #[test]
    fn white_space_at_the_end_of_a_line_is_dropped_and_none_at_the_start() {
        let code = "  fn a()  \t\n    -> u8;   \r\nfn b()\n";
        assert_eq!(of(code, 1, 2).unwrap().as_str(), "  fn a()\n    -> u8;");
    }

    #[test]
    fn a_line_with_the_opening_brace_of_a_body_is_kept_with_it() {
        let code = "pub fn price(total: u64) -> u64 {\n    total\n}\n";
        assert_eq!(
            of(code, 1, 1).unwrap().as_str(),
            "pub fn price(total: u64) -> u64 {"
        );
    }

    #[test]
    fn a_doc_comment_among_the_lines_is_part_of_the_signature() {
        let code = "/// The price.\npub fn price() -> u64;\n";
        assert_eq!(
            of(code, 1, 2).unwrap().as_str(),
            "/// The price.\npub fn price() -> u64;"
        );
        assert_ne!(of(code, 1, 2), of(code, 2, 2));
    }

    #[test]
    fn the_same_text_on_other_lines_is_the_same_signature() {
        assert_eq!(of("fn a()\n", 1, 1), of("// moved\n\nfn a()   \n", 3, 3));
    }

    #[test]
    fn lines_that_are_blank_or_past_the_code_are_refused() {
        assert_eq!(of("fn a()\n  \n", 2, 2), Err(SignatureError::Blank));
        assert_eq!(of("fn a()\n\n\n", 2, 3), Err(SignatureError::Blank));
        assert_eq!(of("fn a()\n", 1, 2), Err(SignatureError::Outside));
    }
}
