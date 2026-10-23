//! Where a definition is in the text of its asset.

use thiserror::Error;

/// Why two line numbers are not a range.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LineRangeError {
    /// The first line is 0; lines are counted from 1.
    #[error("lines are counted from 1; the range starts at 0")]
    StartsAtZero,
    /// The last line is before the first.
    #[error("the range {first}..={last} ends before it starts")]
    EndsBeforeStart {
        /// The first line given.
        first: usize,
        /// The last line given.
        last: usize,
    },
}

/// Lines `first..=last` of a text, counted from 1. It is where a definition is shown, not what it is: a
/// definition keeps its id when it moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineRange {
    first: usize,
    last: usize,
}

impl LineRange {
    /// The range from `first` to `last`, both included.
    pub fn new(first: usize, last: usize) -> Result<LineRange, LineRangeError> {
        if first == 0 {
            return Err(LineRangeError::StartsAtZero);
        }
        if last < first {
            return Err(LineRangeError::EndsBeforeStart { first, last });
        }
        Ok(LineRange { first, last })
    }

    /// The first line.
    pub fn first(&self) -> usize {
        self.first
    }

    /// The last line.
    pub fn last(&self) -> usize {
        self.last
    }

    /// How many lines it covers; at least one.
    pub fn line_count(&self) -> usize {
        self.last - self.first + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_from_one_line_on_is_accepted() {
        let r = LineRange::new(2, 4).unwrap();
        assert_eq!((r.first(), r.last(), r.line_count()), (2, 4, 3));
        assert_eq!(LineRange::new(1, 1).unwrap().line_count(), 1);
    }

    #[test]
    fn a_range_from_zero_or_backwards_is_refused() {
        assert_eq!(LineRange::new(0, 1), Err(LineRangeError::StartsAtZero));
        assert_eq!(
            LineRange::new(3, 2),
            Err(LineRangeError::EndsBeforeStart { first: 3, last: 2 })
        );
    }
}
