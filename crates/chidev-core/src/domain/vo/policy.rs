//! The decision Policy of a Project: the numbered lines that say how a choice is settled without asking anyone.

use std::fmt;

use thiserror::Error;

use super::answer::Basis;
use super::prose::Prose;
use super::revision::Revision;

/// Why a Policy cannot be made or cited.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    /// The text is not `P` and a positive integer.
    #[error("`{0}` is not a policy line id; it is P and a positive number, as P1")]
    BadLineId(String),
    /// Two lines have one id.
    #[error("the policy line {0} is given twice")]
    DuplicateLine(PolicyLineId),
    /// A line is said to be last changed after the Policy's own revision.
    #[error(
        "the policy line {line} changed at {changed_at}, after the policy's revision {revision}"
    )]
    LineFromTheFuture {
        /// The line.
        line: PolicyLineId,
        /// The revision recorded for it.
        changed_at: Revision,
        /// The revision of the Policy.
        revision: Revision,
    },
    /// The Policy has no line with this id.
    #[error("the policy has no line {0}")]
    UnknownLine(PolicyLineId),
}

/// The id of a line of the Policy: `P` and a positive integer, written without leading zeros (`P1`, `P12`). A
/// line keeps its id when its text is revised, so a decision can name the line it applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PolicyLineId(u32);

impl PolicyLineId {
    /// The number after the `P`, from 1.
    pub fn number(self) -> u32 {
        self.0
    }
}

impl TryFrom<&str> for PolicyLineId {
    type Error = PolicyError;

    fn try_from(text: &str) -> Result<PolicyLineId, PolicyError> {
        let bad = || PolicyError::BadLineId(text.to_string());
        let digits = text.strip_prefix('P').ok_or_else(bad)?;
        if digits.is_empty()
            || digits.starts_with('0')
            || !digits.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(bad());
        }
        digits.parse::<u32>().map(PolicyLineId).map_err(|_| bad())
    }
}

impl fmt::Display for PolicyLineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P{}", self.0)
    }
}

/// One line of the Policy: its id, its text, and the revision of the Policy at which the line got the text it has
/// now (when it was added, or last reworded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyLine {
    id: PolicyLineId,
    text: Prose,
    changed_at: Revision,
}

impl PolicyLine {
    /// A line as a store kept it.
    pub(crate) fn restore(id: PolicyLineId, text: Prose, changed_at: Revision) -> PolicyLine {
        PolicyLine {
            id,
            text,
            changed_at,
        }
    }

    /// Its id.
    pub fn id(&self) -> PolicyLineId {
        self.id
    }

    /// What it says.
    pub fn text(&self) -> &Prose {
        &self.text
    }

    /// The revision of the Policy at which the line got its present text. A decision that applied the line at an
    /// earlier revision applied an older text.
    pub fn changed_at(&self) -> Revision {
        self.changed_at
    }
}

/// The Policy of a Project: how choices are settled without asking a decision maker (for instance "P1: take the
/// recommendation when its confidence is 80 or more"). It is a value; a Project replaces it whole. Its revision is
/// 1 when empty and goes one up each time the lines change, and each line records the revision at which it got
/// its text, so a decision that cites a line can later be told stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    revision: Revision,
    lines: Vec<PolicyLine>,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy::new()
    }
}

impl Policy {
    /// A Policy with no lines, at its first revision.
    pub fn new() -> Policy {
        Policy {
            revision: Revision::first(),
            lines: Vec::new(),
        }
    }

    /// A Policy as a store kept it. A line is once, and none is from after the Policy's revision.
    pub(crate) fn restore(
        revision: Revision,
        lines: Vec<PolicyLine>,
    ) -> Result<Policy, PolicyError> {
        for (k, l) in lines.iter().enumerate() {
            if lines[..k].iter().any(|m| m.id == l.id) {
                return Err(PolicyError::DuplicateLine(l.id));
            }
            if l.changed_at > revision {
                return Err(PolicyError::LineFromTheFuture {
                    line: l.id,
                    changed_at: l.changed_at,
                    revision,
                });
            }
        }
        Ok(Policy { revision, lines })
    }

    /// How many times its lines have changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The lines, in the order given.
    pub fn lines(&self) -> &[PolicyLine] {
        &self.lines
    }

    /// The line with this id.
    pub fn line(&self, id: PolicyLineId) -> Option<&PolicyLine> {
        self.lines.iter().find(|l| l.id == id)
    }

    /// The Policy that replaces this one with `lines`. When the lines are the ones it has, in the same order, it is
    /// this one and the revision does not move; otherwise the revision goes one up, a line whose text is the same
    /// keeps the revision it was changed at, and any other line is changed at the new revision.
    pub fn put(&self, lines: Vec<(PolicyLineId, Prose)>) -> Result<Policy, PolicyError> {
        for (k, (id, _)) in lines.iter().enumerate() {
            if lines[..k].iter().any(|(other, _)| other == id) {
                return Err(PolicyError::DuplicateLine(*id));
            }
        }
        let same = lines.len() == self.lines.len()
            && lines
                .iter()
                .zip(&self.lines)
                .all(|((id, text), l)| *id == l.id && *text == l.text);
        if same {
            return Ok(self.clone());
        }
        let revision = self.revision.next();
        let lines = lines
            .into_iter()
            .map(|(id, text)| {
                let changed_at = match self.line(id) {
                    Some(old) if old.text == text => old.changed_at,
                    _ => revision,
                };
                PolicyLine {
                    id,
                    text,
                    changed_at,
                }
            })
            .collect();
        Ok(Policy { revision, lines })
    }

    /// The basis of a decision that applies the line `id`: the line with the revision of the Policy now.
    pub fn basis_on(&self, id: PolicyLineId) -> Result<Basis, PolicyError> {
        self.line(id)
            .map(|_| Basis::Policy {
                line: id,
                policy_revision: self.revision,
            })
            .ok_or(PolicyError::UnknownLine(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> PolicyLineId {
        PolicyLineId::try_from(text).unwrap()
    }

    fn line(n: &str, text: &str) -> (PolicyLineId, Prose) {
        (id(n), Prose::try_from(text).unwrap())
    }

    #[test]
    fn a_line_id_is_p_and_a_positive_number_without_leading_zeros() {
        assert_eq!(id("P1").number(), 1);
        assert_eq!(id("P12").to_string(), "P12");
        for bad in [
            "",
            "P",
            "P0",
            "P01",
            "p1",
            "1",
            "P-1",
            "P1a",
            "P 1",
            "P+1",
            "P99999999999",
        ] {
            assert_eq!(
                PolicyLineId::try_from(bad),
                Err(PolicyError::BadLineId(bad.to_string())),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_policy_starts_empty_and_its_revision_moves_only_when_its_lines_change() {
        let empty = Policy::new();
        assert_eq!(empty.revision(), Revision::first());
        assert!(empty.lines().is_empty());
        // Putting no lines again changes nothing.
        assert_eq!(empty.put(Vec::new()).unwrap(), empty);
        let one = empty
            .put(vec![line("P1", "take the recommendation from 80")])
            .unwrap();
        assert_eq!(one.revision(), Revision::first().next());
        assert_eq!(one.line(id("P1")).unwrap().changed_at(), one.revision());
        // The same lines again: the same Policy.
        assert_eq!(
            one.put(vec![line("P1", "take the recommendation from 80")])
                .unwrap(),
            one
        );
    }

    #[test]
    fn a_line_keeps_the_revision_it_was_changed_at_until_its_text_changes_or_it_is_removed() {
        let first = Policy::new()
            .put(vec![line("P1", "a"), line("P2", "b")])
            .unwrap();
        let second = first.put(vec![line("P1", "a"), line("P2", "b2")]).unwrap();
        assert_eq!(second.revision().number(), 3);
        assert_eq!(
            second.line(id("P1")).unwrap().changed_at(),
            first.revision()
        );
        assert_eq!(
            second.line(id("P2")).unwrap().changed_at(),
            second.revision()
        );
        // A line removed and written again is a new text.
        let third = second.put(vec![line("P2", "b2")]).unwrap();
        let fourth = third.put(vec![line("P1", "a"), line("P2", "b2")]).unwrap();
        assert_eq!(third.line(id("P1")), None);
        assert_eq!(
            fourth.line(id("P1")).unwrap().changed_at(),
            fourth.revision()
        );
        assert_eq!(
            fourth.line(id("P2")).unwrap().changed_at(),
            second.revision()
        );
    }

    #[test]
    fn a_line_is_given_once() {
        assert_eq!(
            Policy::new().put(vec![line("P1", "a"), line("P1", "b")]),
            Err(PolicyError::DuplicateLine(id("P1")))
        );
    }

    #[test]
    fn a_decision_can_cite_only_a_line_the_policy_has() {
        let p = Policy::new().put(vec![line("P1", "a")]).unwrap();
        assert_eq!(
            p.basis_on(id("P1")),
            Ok(Basis::Policy {
                line: id("P1"),
                policy_revision: p.revision()
            })
        );
        assert_eq!(
            p.basis_on(id("P2")),
            Err(PolicyError::UnknownLine(id("P2")))
        );
    }

    #[test]
    fn a_stored_policy_has_each_line_once_and_none_from_after_its_revision() {
        let l =
            |n: &str, at: Revision| PolicyLine::restore(id(n), Prose::try_from("t").unwrap(), at);
        let two = Revision::first().next();
        assert!(Policy::restore(two, vec![l("P1", Revision::first()), l("P2", two)]).is_ok());
        assert_eq!(
            Policy::restore(two, vec![l("P1", two), l("P1", two)]),
            Err(PolicyError::DuplicateLine(id("P1")))
        );
        assert_eq!(
            Policy::restore(Revision::first(), vec![l("P1", two)]),
            Err(PolicyError::LineFromTheFuture {
                line: id("P1"),
                changed_at: two,
                revision: Revision::first()
            })
        );
    }
}
