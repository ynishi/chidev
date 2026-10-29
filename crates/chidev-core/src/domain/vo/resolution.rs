//! How a change request ended, and the transitions it went through. A resolution is a decision somebody made, with
//! the reason and the time; it is never inferred from what the definitions did. Reopening is a transition of its
//! own, so the history of a request only grows, and so is a correction of a text in it.

use std::fmt;

use super::answer::DecisionMaker;
use super::correction::Correction;
use super::id::ChangeRequestId;
use super::prose::Prose;
use super::timestamp::Timestamp;
use crate::domain::entity::change_request::ChangeRequestField;

/// The conclusion of a resolution: how the request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionKind {
    /// What was asked is done.
    Done,
    /// It will not be done.
    WontDo,
    /// It is the same as another change request, which carries it.
    Duplicate {
        /// The change request it duplicates.
        of: ChangeRequestId,
    },
    /// Another change request replaces it.
    Superseded {
        /// The change request that replaces it.
        by: ChangeRequestId,
    },
    /// It was never a valid request (a mistake, or nothing to ask).
    Invalid,
}

impl ResolutionKind {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            ResolutionKind::Done => "done",
            ResolutionKind::WontDo => "wont_do",
            ResolutionKind::Duplicate { .. } => "duplicate",
            ResolutionKind::Superseded { .. } => "superseded",
            ResolutionKind::Invalid => "invalid",
        }
    }

    /// The change request it points at, when it points at one.
    pub fn reference(self) -> Option<ChangeRequestId> {
        match self {
            ResolutionKind::Duplicate { of: id } | ResolutionKind::Superseded { by: id } => {
                Some(id)
            }
            _ => None,
        }
    }

    /// Whether the request is closed as done.
    pub fn is_done(self) -> bool {
        self == ResolutionKind::Done
    }
}

/// The record of a resolution: how it ended, why, who said so, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    kind: ResolutionKind,
    reason: Prose,
    by: DecisionMaker,
    at: Timestamp,
}

impl Resolution {
    /// A resolution.
    pub fn new(
        kind: ResolutionKind,
        reason: Prose,
        by: DecisionMaker,
        at: Timestamp,
    ) -> Resolution {
        Resolution {
            kind,
            reason,
            by,
            at,
        }
    }

    /// How it ended.
    pub fn kind(&self) -> ResolutionKind {
        self.kind
    }

    /// Why.
    pub fn reason(&self) -> &Prose {
        &self.reason
    }

    /// Who resolved it.
    pub fn by(&self) -> &DecisionMaker {
        &self.by
    }

    /// When.
    pub fn at(&self) -> Timestamp {
        self.at
    }
}

/// The record of a reopening: why, who, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reopening {
    reason: Prose,
    by: DecisionMaker,
    at: Timestamp,
}

impl Reopening {
    /// A reopening.
    pub fn new(reason: Prose, by: DecisionMaker, at: Timestamp) -> Reopening {
        Reopening { reason, by, at }
    }

    /// Why.
    pub fn reason(&self) -> &Prose {
        &self.reason
    }

    /// Who reopened it.
    pub fn by(&self) -> &DecisionMaker {
        &self.by
    }

    /// When.
    pub fn at(&self) -> Timestamp {
        self.at
    }
}

/// One step in the history of a change request: it was resolved, it was reopened, or a text in it was corrected. The
/// history is a log that is only appended to. Resolutions and reopenings alternate, starting with a resolution, and
/// the request is resolved when the last of them is a resolution; a correction may stand anywhere among them and
/// changes no state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// The request was resolved.
    Resolved(Resolution),
    /// The request was opened again.
    Reopened(Reopening),
    /// A text in the request was replaced by a mark. The step keeps the fields, why, who and when, and no text.
    Corrected(Correction<ChangeRequestField>),
}

impl Transition {
    /// Whether the step is a resolution or a reopening, the steps that decide the state.
    pub fn is_state(&self) -> bool {
        !matches!(self, Transition::Corrected(_))
    }
}

impl fmt::Display for ResolutionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;

    #[test]
    fn a_kind_names_the_request_it_points_at() {
        let other = ChangeRequestId::from_uuid(test_uuid(2));
        assert_eq!(
            ResolutionKind::Duplicate { of: other }.reference(),
            Some(other)
        );
        assert_eq!(
            ResolutionKind::Superseded { by: other }.reference(),
            Some(other)
        );
        for kind in [
            ResolutionKind::Done,
            ResolutionKind::WontDo,
            ResolutionKind::Invalid,
        ] {
            assert_eq!(kind.reference(), None);
        }
        assert!(ResolutionKind::Done.is_done());
        assert!(!ResolutionKind::WontDo.is_done());
        assert_eq!(ResolutionKind::Duplicate { of: other }.word(), "duplicate");
    }
}
