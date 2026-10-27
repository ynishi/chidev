//! The Contract: what a Context promises, as a title and a list of Clauses a Harness case can cite.

use thiserror::Error;

use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::id::{ClauseId, ContractId};
use crate::domain::vo::revision::Revision;
use crate::domain::vo::title::Title;

/// One promise of a Contract. It is known by its id, so a citation of it survives a new label or a reworded
/// sentence. Its label is the name a person uses for it, as an Item's key is, and is not part of what it
/// promises: its revision says whether the sentence changed since a citation was made, and a new label alone
/// does not move it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    id: ClauseId,
    label: ClauseLabel,
    revision: Revision,
    text: ClauseText,
}

impl Clause {
    /// Its id.
    pub fn id(&self) -> ClauseId {
        self.id
    }

    /// The name it is known by.
    pub fn label(&self) -> ClauseLabel {
        self.label
    }

    /// How many times its sentence has changed, from 1. A new label does not count.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The sentence it promises.
    pub fn text(&self) -> &ClauseText {
        &self.text
    }
}

/// Why a list of Clauses is not the content of a Contract. The Contract is not made or changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContractError {
    /// Two Clauses have one id.
    #[error("the clause id {0} is given twice")]
    DuplicateId(ClauseId),
    /// Two Clauses have one label, so a person could not tell which one a citation means.
    #[error("the clause label {0} is given twice")]
    DuplicateLabel(ClauseLabel),
}

/// What a Context promises. The Contract is its content (a title and Clauses in order), not a text in some form:
/// the same content read from Markdown or from JSON is the same Contract. Its revision counts changes of the
/// title, of the sentence of a Clause, of the order and of the set of Clauses; a new label alone is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    id: ContractId,
    revision: Revision,
    title: Title,
    clauses: Vec<Clause>,
}

/// The ids and labels of `clauses` are each given once.
fn check(clauses: &[(ClauseId, ClauseContent)]) -> Result<(), ContractError> {
    for (k, (id, content)) in clauses.iter().enumerate() {
        let before = &clauses[..k];
        if before.iter().any(|(i, _)| i == id) {
            return Err(ContractError::DuplicateId(*id));
        }
        if before.iter().any(|(_, c)| c.label() == content.label()) {
            return Err(ContractError::DuplicateLabel(content.label()));
        }
    }
    Ok(())
}

impl Contract {
    /// A new Contract at its first revision, with its Clauses in the order given, each at its first revision.
    /// The ids are the caller's: the domain makes none.
    pub fn new(
        id: ContractId,
        title: Title,
        clauses: Vec<(ClauseId, ClauseContent)>,
    ) -> Result<Contract, ContractError> {
        check(&clauses)?;
        Ok(Contract {
            id,
            revision: Revision::first(),
            title,
            clauses: clauses
                .into_iter()
                .map(|(id, content)| {
                    let (label, text) = content.into_parts();
                    Clause {
                        id,
                        label,
                        revision: Revision::first(),
                        text,
                    }
                })
                .collect(),
        })
    }

    /// A Contract as a store kept it, revisions included. The rules of [`Contract::new`] hold on it.
    pub(crate) fn restore(
        id: ContractId,
        revision: Revision,
        title: Title,
        clauses: Vec<(ClauseId, Revision, ClauseContent)>,
    ) -> Result<Contract, ContractError> {
        let given: Vec<(ClauseId, ClauseContent)> =
            clauses.iter().map(|(i, _, c)| (*i, c.clone())).collect();
        check(&given)?;
        Ok(Contract {
            id,
            revision,
            title,
            clauses: clauses
                .into_iter()
                .map(|(id, revision, content)| {
                    let (label, text) = content.into_parts();
                    Clause {
                        id,
                        label,
                        revision,
                        text,
                    }
                })
                .collect(),
        })
    }

    /// Its id.
    pub fn id(&self) -> ContractId {
        self.id
    }

    /// How many times its content has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Its title.
    pub fn title(&self) -> &Title {
        &self.title
    }

    /// Its Clauses, in order.
    pub fn clauses(&self) -> &[Clause] {
        &self.clauses
    }

    /// The Clause with this id.
    pub fn clause(&self, id: ClauseId) -> Option<&Clause> {
        self.clauses.iter().find(|c| c.id == id)
    }

    /// The Clause with this label.
    pub fn clause_by_label(&self, label: ClauseLabel) -> Option<&Clause> {
        self.clauses.iter().find(|c| c.label == label)
    }

    /// Gives the Contract new content. A Clause whose id was there keeps its revision when its sentence is the
    /// same and goes to the next one when it differs; a new id starts at the first; an id left out is gone. A
    /// Clause takes the label it is given, and a new label alone moves no revision, as a new key moves none of
    /// an Item. The Contract goes to its next revision when anything else differs (the title, the sentence of a
    /// Clause, the order, a Clause added or gone) and stays when nothing does. Content that breaks a rule
    /// changes nothing.
    pub fn revise(
        &mut self,
        title: Title,
        clauses: Vec<(ClauseId, ClauseContent)>,
    ) -> Result<(), ContractError> {
        check(&clauses)?;
        let revised: Vec<Clause> = clauses
            .into_iter()
            .map(|(id, content)| {
                let (label, text) = content.into_parts();
                let revision = match self.clause(id) {
                    None => Revision::first(),
                    Some(old) if old.text == text => old.revision,
                    Some(old) => old.revision.next(),
                };
                Clause {
                    id,
                    label,
                    revision,
                    text,
                }
            })
            .collect();
        let same = title == self.title
            && revised.len() == self.clauses.len()
            && revised
                .iter()
                .zip(&self.clauses)
                .all(|(new, old)| new.id == old.id && new.text == old.text);
        if !same {
            self.revision = self.revision.next();
        }
        self.title = title;
        self.clauses = revised;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::clause::ClauseText;
    use crate::domain::vo::id::test_uuid;

    fn cid(n: u128) -> ClauseId {
        ClauseId::from_uuid(test_uuid(n))
    }

    fn content(label: &str, text: &str) -> ClauseContent {
        ClauseContent::new(
            ClauseLabel::try_from(label).unwrap(),
            ClauseText::try_from(text).unwrap(),
        )
    }

    fn title(t: &str) -> Title {
        Title::try_from(t).unwrap()
    }

    fn stack() -> Contract {
        Contract::new(
            ContractId::from_uuid(test_uuid(100)),
            title("Stack"),
            vec![
                (cid(1), content("C1", "pop gives back what push put")),
                (cid(2), content("C2", "an empty stack has no top")),
            ],
        )
        .unwrap()
    }

    fn revisions(c: &Contract) -> Vec<u64> {
        c.clauses().iter().map(|c| c.revision().number()).collect()
    }

    #[test]
    fn a_new_contract_and_its_clauses_start_at_the_first_revision() {
        let c = stack();
        assert_eq!(c.revision(), Revision::first());
        assert_eq!(c.title().as_str(), "Stack");
        assert_eq!(revisions(&c), [1, 1]);
        assert_eq!(
            c.clause(cid(2)).unwrap().text().as_str(),
            "an empty stack has no top"
        );
        let c1 = ClauseLabel::try_from("C1").unwrap();
        assert_eq!(c.clause_by_label(c1).unwrap().id(), cid(1));
        assert!(c.clause(cid(9)).is_none());
    }

    #[test]
    fn a_duplicate_id_or_label_is_refused() {
        let id = ContractId::from_uuid(test_uuid(100));
        assert_eq!(
            Contract::new(
                id,
                title("T"),
                vec![(cid(1), content("C1", "a")), (cid(1), content("C2", "b"))]
            ),
            Err(ContractError::DuplicateId(cid(1)))
        );
        assert_eq!(
            Contract::new(
                id,
                title("T"),
                vec![(cid(1), content("C1", "a")), (cid(2), content("C1", "b"))]
            ),
            Err(ContractError::DuplicateLabel(
                ClauseLabel::try_from("C1").unwrap()
            ))
        );
    }

    #[test]
    fn the_same_content_again_changes_no_revision() {
        let mut c = stack();
        let before = c.clone();
        c.revise(
            title("Stack"),
            vec![
                (cid(1), content("C1", "pop gives back what push put")),
                (cid(2), content("C2", "an empty stack has no top")),
            ],
        )
        .unwrap();
        assert_eq!(c, before);
    }

    #[test]
    fn a_changed_clause_and_its_contract_go_to_the_next_revision_and_the_others_stay() {
        let mut c = stack();
        c.revise(
            title("Stack"),
            vec![
                (cid(1), content("C1", "pop gives back the last push")),
                (cid(2), content("C2", "an empty stack has no top")),
            ],
        )
        .unwrap();
        assert_eq!(c.revision().number(), 2);
        assert_eq!(revisions(&c), [2, 1]);
    }

    #[test]
    fn a_new_label_alone_moves_no_revision_and_keeps_the_id() {
        let mut c = stack();
        c.revise(
            title("Stack"),
            vec![
                (cid(1), content("C3", "pop gives back what push put")),
                (cid(2), content("C2", "an empty stack has no top")),
            ],
        )
        .unwrap();
        assert_eq!(c.clause(cid(1)).unwrap().label().number(), 3);
        assert_eq!(c.revision(), Revision::first());
        assert_eq!(revisions(&c), [1, 1]);
        let c3 = ClauseLabel::try_from("C3").unwrap();
        assert_eq!(c.clause_by_label(c3).unwrap().id(), cid(1));
        let c1 = ClauseLabel::try_from("C1").unwrap();
        assert!(c.clause_by_label(c1).is_none());
    }

    #[test]
    fn a_new_label_with_a_new_sentence_moves_the_clause_and_the_contract() {
        let mut c = stack();
        c.revise(
            title("Stack"),
            vec![
                (cid(1), content("C3", "pop gives back the last push")),
                (cid(2), content("C2", "an empty stack has no top")),
            ],
        )
        .unwrap();
        assert_eq!(c.clause(cid(1)).unwrap().label().number(), 3);
        assert_eq!(c.revision().number(), 2);
        assert_eq!(revisions(&c), [2, 1]);
    }

    #[test]
    fn a_new_title_an_added_a_removed_or_a_moved_clause_changes_the_contract() {
        let mut c = stack();
        c.revise(
            title("Stack v2"),
            vec![
                (cid(1), content("C1", "pop gives back what push put")),
                (cid(2), content("C2", "an empty stack has no top")),
            ],
        )
        .unwrap();
        assert_eq!(c.revision().number(), 2);
        c.revise(
            title("Stack v2"),
            vec![
                (cid(1), content("C1", "pop gives back what push put")),
                (cid(3), content("C3", "push grows the stack by one")),
            ],
        )
        .unwrap();
        assert_eq!(c.revision().number(), 3);
        assert_eq!(revisions(&c), [1, 1]);
        assert!(c.clause(cid(2)).is_none());
        c.revise(
            title("Stack v2"),
            vec![
                (cid(3), content("C3", "push grows the stack by one")),
                (cid(1), content("C1", "pop gives back what push put")),
            ],
        )
        .unwrap();
        assert_eq!(c.revision().number(), 4);
        assert_eq!(revisions(&c), [1, 1]);
    }

    #[test]
    fn content_that_breaks_a_rule_changes_nothing() {
        let mut c = stack();
        let before = c.clone();
        assert_eq!(
            c.revise(
                title("X"),
                vec![(cid(1), content("C1", "a")), (cid(3), content("C1", "b"))]
            ),
            Err(ContractError::DuplicateLabel(
                ClauseLabel::try_from("C1").unwrap()
            ))
        );
        assert_eq!(c, before);
    }
}
