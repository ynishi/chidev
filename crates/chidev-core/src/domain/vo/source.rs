//! The text of an asset as a person wrote it, which is what the asset is kept as.

use super::digest::Digest;
use super::line_range::LineRange;

/// The whole text of an asset (a Contract, an Interface or a Harness). The text is the asset; what is read from
/// it (Clauses, Items, Cases) is built from it, so its digest is the digest of the asset.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Source(String);

impl Source {
    /// The text as given.
    pub fn new(text: String) -> Source {
        Source(text)
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The digest of the text, byte for byte.
    pub fn digest(&self) -> Digest {
        Digest::of(self.0.as_bytes())
    }

    /// How many lines the text has.
    pub fn line_count(&self) -> usize {
        self.0.lines().count()
    }

    /// The lines of `range`, joined by `\n`; `None` when the range goes past the last line. A range that points
    /// outside the text would make a later digest or citation point at nothing, so the caller refuses it there.
    pub fn lines(&self, range: LineRange) -> Option<String> {
        let lines: Vec<&str> = self
            .0
            .lines()
            .skip(range.first() - 1)
            .take(range.line_count())
            .collect();
        (lines.len() == range.line_count()).then(|| lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(first: usize, last: usize) -> LineRange {
        LineRange::new(first, last).unwrap()
    }

    #[test]
    fn a_range_inside_the_text_gives_its_lines() {
        let s = Source::new("a\nb\nc".to_string());
        assert_eq!(s.lines(range(2, 3)).as_deref(), Some("b\nc"));
        assert_eq!(s.lines(range(1, 1)).as_deref(), Some("a"));
        assert_eq!(s.line_count(), 3);
    }

    #[test]
    fn a_range_past_the_last_line_gives_nothing() {
        assert_eq!(Source::new("a\nb".to_string()).lines(range(2, 3)), None);
        assert_eq!(Source::new(String::new()).lines(range(1, 1)), None);
    }

    #[test]
    fn the_digest_is_the_digest_of_the_text() {
        let s = Source::new("# T\n".to_string());
        assert_eq!(s.digest(), Digest::of(b"# T\n"));
    }
}
