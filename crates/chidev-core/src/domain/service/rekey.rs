//! Which declaration a provider found is which Item an Interface already has. The rule reads no language: only the
//! lines each side points at.

use crate::domain::vo::line_range::LineRange;

/// Whether two ranges have a line in common.
fn overlap(a: LineRange, b: LineRange) -> bool {
    a.first() <= b.last() && b.first() <= a.last()
}

/// For each of the `kept` ranges (the lines of the Items an Interface has), the index of the range of `found` (what a
/// provider finds in the same code) it is paired with, or `None`.
///
/// A pair is made when the two ranges overlap **and each is the only one on its side that overlaps the other**.
/// Anything else is not paired and is left to the caller to report: a kept range that overlaps several found ones,
/// several kept ranges that overlap one found one (two declarations on one line come to either), and a kept range
/// that no found one overlaps. Pairing on a doubt would hand an Item the key of another declaration, and its id
/// would follow the wrong one.
pub fn pair(kept: &[LineRange], found: &[LineRange]) -> Vec<Option<usize>> {
    kept.iter()
        .map(|k| {
            let mut over = found
                .iter()
                .enumerate()
                .filter(|(_, f)| overlap(*k, **f))
                .map(|(n, _)| n);
            let only = over.next().filter(|_| over.next().is_none())?;
            let others = kept.iter().filter(|o| overlap(**o, found[only])).count();
            (others == 1).then_some(only)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(first: usize, last: usize) -> LineRange {
        LineRange::new(first, last).unwrap()
    }

    #[test]
    fn ranges_that_overlap_alone_on_both_sides_are_paired() {
        // the provider's range may be longer or shorter than the one kept, and starts or ends elsewhere
        let kept = [at(1, 1), at(3, 4), at(6, 6)];
        let found = [at(1, 2), at(4, 5), at(6, 6)];
        assert_eq!(pair(&kept, &found), [Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn a_kept_range_that_nothing_found_overlaps_is_not_paired() {
        assert_eq!(pair(&[at(1, 1), at(5, 5)], &[at(1, 1)]), [Some(0), None]);
        assert_eq!(pair(&[at(1, 1)], &[]), [None]);
        assert_eq!(pair(&[], &[at(1, 1)]), Vec::<Option<usize>>::new());
    }

    #[test]
    fn one_kept_range_over_several_found_ones_is_not_paired() {
        // `fn a(); fn b();` was one Item and is now two declarations on its line
        assert_eq!(pair(&[at(1, 2)], &[at(1, 1), at(2, 2)]), [None]);
    }

    #[test]
    fn one_found_range_over_several_kept_ones_pairs_none_of_them() {
        // two Items were kept on lines the provider finds as one declaration
        assert_eq!(pair(&[at(1, 1), at(2, 2)], &[at(1, 2)]), [None, None]);
        // and the ones beside them are paired as they were
        assert_eq!(
            pair(&[at(1, 1), at(2, 2), at(4, 4)], &[at(1, 2), at(4, 4)]),
            [None, None, Some(1)]
        );
    }

    #[test]
    fn two_declarations_on_one_line_pair_with_neither() {
        // `struct A; struct B;` found as two ranges on line 1, kept as one Item on it
        assert_eq!(pair(&[at(1, 1)], &[at(1, 1), at(1, 1)]), [None]);
        // kept as two Items on it, found as two: each overlaps both
        assert_eq!(
            pair(&[at(1, 1), at(1, 1)], &[at(1, 1), at(1, 1)]),
            [None, None]
        );
    }

    #[test]
    fn a_range_touching_only_at_its_end_overlaps_there() {
        assert_eq!(pair(&[at(1, 3)], &[at(3, 5)]), [Some(0)]);
        assert_eq!(pair(&[at(1, 2)], &[at(3, 5)]), [None]);
    }
}
