//! What a change introduced: the findings of the state after it that the state before it did not have.

use super::finding::Finding;

/// The findings of `after` that the change introduced, in their order. A finding is introduced when `before` has
/// no finding of the same rule about the same subject (the same [`FindingRef`](crate::domain::vo::finding::FindingRef)),
/// or has one that stands on fewer grounds: `after` has a ground that the finding of `before` lacks.
///
/// The message is never compared: wording is not a fact, and a message that counts things changes without the
/// change having made anything worse. A finding that lost grounds has not been introduced; what is left of it was
/// there before.
pub fn introduced(before: &[Finding], after: &[Finding]) -> Vec<Finding> {
    after
        .iter()
        .filter(|f| match before.iter().find(|b| b.at() == f.at()) {
            None => true,
            Some(b) => f.grounds().iter().any(|g| !b.grounds().contains(g)),
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::domain::check::descriptor::{Descriptor, Executor};
    use crate::domain::check::finding::Ground;
    use crate::domain::check::level::{Hook, Level};
    use crate::domain::vo::finding::{RuleId, Subject};
    use crate::domain::vo::id::{ClauseId, test_uuid};
    use crate::domain::vo::target::Target;

    static RULE: Descriptor = Descriptor {
        id: RuleId::from_static("t.rule"),
        summary: Cow::Borrowed("s"),
        help: Cow::Borrowed("h"),
        hooks: Cow::Borrowed(&[Hook::Put]),
        default_level: Level::Warning,
        mandatory: false,
        params: Cow::Borrowed(&[]),
        executor: Executor::Builtin,
    };

    static OTHER: Descriptor = Descriptor {
        id: RuleId::from_static("t.other"),
        summary: Cow::Borrowed("s"),
        help: Cow::Borrowed("h"),
        hooks: Cow::Borrowed(&[Hook::Put]),
        default_level: Level::Warning,
        mandatory: false,
        params: Cow::Borrowed(&[]),
        executor: Executor::Builtin,
    };

    fn clause(n: u128) -> Subject {
        Subject::Clause(ClauseId::from_uuid(test_uuid(n)))
    }

    fn found(d: &Descriptor, n: u128, message: &str, grounds: Vec<Ground>) -> Finding {
        Finding::new(d, Some(clause(n)), message).grounded(grounds)
    }

    fn moved(at: u64) -> Ground {
        Ground::Work {
            target: Target::Clause(ClauseId::from_uuid(test_uuid(1))),
            at,
        }
    }

    #[test]
    fn a_finding_the_state_before_had_is_not_introduced() {
        let f = found(&RULE, 1, "m", vec![moved(1)]);
        assert!(introduced(std::slice::from_ref(&f), std::slice::from_ref(&f)).is_empty());
        // Nothing before: everything is new.
        assert_eq!(introduced(&[], std::slice::from_ref(&f)), [f]);
    }

    #[test]
    fn a_finding_of_another_rule_or_another_subject_is_introduced() {
        let before = [found(&RULE, 1, "m", vec![])];
        let after = [
            found(&RULE, 2, "m", vec![]),
            found(&OTHER, 1, "m", vec![]),
            found(&RULE, 1, "m", vec![]),
        ];
        let new = introduced(&before, &after);
        assert_eq!(
            new.iter()
                .map(|f| (f.rule().as_str(), f.subject()))
                .collect::<Vec<_>>(),
            [("t.rule", Some(clause(2))), ("t.other", Some(clause(1)))]
        );
    }

    #[test]
    fn a_ground_the_finding_before_lacks_makes_it_introduced() {
        let before = [found(&RULE, 1, "m", vec![moved(1)])];
        // A new ground beside the old one; the same ground moved to another mark.
        assert_eq!(
            introduced(&before, &[found(&RULE, 1, "m", vec![moved(1), moved(2)])]).len(),
            1
        );
        assert_eq!(
            introduced(&before, &[found(&RULE, 1, "m", vec![moved(2)])]).len(),
            1
        );
        // Fewer grounds than before: what is left was there.
        let before = [found(&RULE, 1, "m", vec![moved(1), moved(2)])];
        assert!(introduced(&before, &[found(&RULE, 1, "m", vec![moved(2)])]).is_empty());
    }

    #[test]
    fn the_message_is_never_compared() {
        let before = [found(&RULE, 1, "1 target(s) moved", vec![moved(1)])];
        let after = [found(&RULE, 1, "2 target(s) moved", vec![moved(1)])];
        assert!(introduced(&before, &after).is_empty());
    }

    #[test]
    fn a_finding_with_no_grounds_is_introduced_only_when_it_is_new() {
        let f = found(&RULE, 1, "m", vec![]);
        assert!(introduced(std::slice::from_ref(&f), std::slice::from_ref(&f)).is_empty());
        assert_eq!(introduced(&[], std::slice::from_ref(&f)), [f]);
    }
}
