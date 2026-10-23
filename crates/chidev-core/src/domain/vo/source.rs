//! The text of an asset as a person wrote it, which is what the asset is kept as.

use super::line_range::LineRange;

/// The whole text of an asset that is code (an Interface or a Harness): the code is what compiles, so the text is
/// the content, and the definitions in it (Items, Cases) are shown by their lines.
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
}
