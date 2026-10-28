//! A Clause as a caller writes it into a Contract put, and the problems with the names such a put can have.

use std::fmt;

use super::clause::{ClauseLabel, ClauseText};
use super::id::ClauseId;

/// One Clause of a Contract put, as written: the label it is known by, its sentence, the label of the Clause it
/// takes the place of, if it is a new name for one the Contract has, and the id it takes when the put makes it.
///
/// The `id` is the caller's: the domain draws no ids. It is the id of the Clause only when the edit makes a new
/// one; an edit that sets a Clause the Contract has (by its label, or by the one it `replaces`) keeps that
/// Clause's id, and the id given is not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseEdit {
    label: ClauseLabel,
    text: ClauseText,
    replaces: Option<ClauseLabel>,
    id: ClauseId,
}

impl ClauseEdit {
    /// An edit of the Clause `label` to say `text`. `replaces` is the label of the Clause of the Contract that
    /// this one is, under a new label. `id` is the id the Clause takes if the edit makes a new one.
    pub fn new(
        label: ClauseLabel,
        text: ClauseText,
        replaces: Option<ClauseLabel>,
        id: ClauseId,
    ) -> ClauseEdit {
        ClauseEdit {
            label,
            text,
            replaces,
            id,
        }
    }

    /// The label it is known by.
    pub fn label(&self) -> ClauseLabel {
        self.label
    }

    /// The sentence it promises.
    pub fn text(&self) -> &ClauseText {
        &self.text
    }

    /// The label of the Clause it takes the place of.
    pub fn replaces(&self) -> Option<ClauseLabel> {
        self.replaces
    }

    /// The id it takes when the edit makes a new Clause.
    pub fn id(&self) -> ClauseId {
        self.id
    }
}

/// Which list of a put a label is named in when the Contract does not have it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameField {
    /// The `replaces` of an edit.
    Replaces,
    /// The labels to remove.
    Remove,
}

impl NameField {
    /// The word for it: `replaces` or `remove`.
    pub fn word(self) -> &'static str {
        match self {
            NameField::Replaces => "replaces",
            NameField::Remove => "remove",
        }
    }
}

/// Where a label is named twice, or named against itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSide {
    /// Two edits are given the label.
    Label,
    /// Two edits replace the label.
    Replaces,
    /// The label is removed and also given, replaced or removed again.
    Remove,
}

impl NameSide {
    /// The word for it: `label`, `replaces` or `remove`.
    pub fn word(self) -> &'static str {
        match self {
            NameSide::Label => "label",
            NameSide::Replaces => "replaces",
            NameSide::Remove => "remove",
        }
    }
}

/// A way the names of a Contract put are wrong. A put can have several at once, and all of them are found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseNameProblem {
    /// A label that the Contract does not have is named where it must (`replaces` of an edit, or a label to
    /// remove).
    Unknown {
        /// The label.
        label: ClauseLabel,
        /// Where it is named.
        field: NameField,
    },
    /// A label is named twice, or against itself.
    Conflict {
        /// The label.
        label: ClauseLabel,
        /// Where it is named.
        side: NameSide,
    },
    /// An edit replaces a Clause that another edit still gives under its old label.
    ReplacesKept {
        /// The label that is replaced and still given.
        label: ClauseLabel,
    },
}

impl fmt::Display for ClauseNameProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClauseNameProblem::Unknown { label, field } => write!(
                f,
                "the contract has no clause {label} to name in `{}`",
                field.word()
            ),
            ClauseNameProblem::Conflict { label, side } => {
                write!(f, "the clause {label} is named twice in `{}`", side.word())
            }
            ClauseNameProblem::ReplacesKept { label } => {
                write!(f, "the clause {label} is replaced and still given")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_are_the_ones_a_person_reads() {
        assert_eq!(NameField::Replaces.word(), "replaces");
        assert_eq!(NameField::Remove.word(), "remove");
        assert_eq!(NameSide::Label.word(), "label");
        assert_eq!(NameSide::Replaces.word(), "replaces");
        assert_eq!(NameSide::Remove.word(), "remove");
    }

    #[test]
    fn a_problem_is_said_with_its_label() {
        let c1 = ClauseLabel::try_from("C1").unwrap();
        let said = ClauseNameProblem::Conflict {
            label: c1,
            side: NameSide::Remove,
        }
        .to_string();
        assert!(said.contains("C1") && said.contains("remove"), "{said}");
    }
}
