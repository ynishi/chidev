//! The Contract: what a Context promises, as a title and a list of Clauses a Harness case can cite.

use thiserror::Error;

use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::clause_edit::{ClauseEdit, ClauseNameProblem, NameField, NameSide};
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
    /// A partial put of the Contract names Clauses wrongly (see [`Contract::clauses_partial`]): one problem with
    /// the names, where a put can have several. A caller that checked the names with the Contract before it put
    /// never gets this.
    #[error("the contract {contract} is put with a wrong clause name: {problem}")]
    ClauseName {
        /// The Contract.
        contract: ContractId,
        /// The problem.
        problem: ClauseNameProblem,
    },
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

/// Every problem with the names of a put of `edits` and `remove` on `old` (the Contract as it is, if it is one), in
/// the order found: for each edit a label given twice that the Contract has, then what it `replaces` (named by two
/// edits, not in the Contract, or still given); then for each label to remove one named against itself or not in
/// the Contract. A label the Contract does not have, given twice, is no problem of names: [`Contract::new`] and
/// [`Contract::revise`] refuse it as [`ContractError::DuplicateLabel`].
fn name_problems(
    old: Option<&Contract>,
    edits: &[ClauseEdit],
    remove: &[ClauseLabel],
) -> Vec<ClauseNameProblem> {
    let has = |label: ClauseLabel| old.is_some_and(|k| k.clause_by_label(label).is_some());
    let mut problems = Vec::new();
    for (n, edit) in edits.iter().enumerate() {
        let earlier = &edits[..n];
        if has(edit.label()) && earlier.iter().any(|e| e.label() == edit.label()) {
            problems.push(ClauseNameProblem::Conflict {
                label: edit.label(),
                side: NameSide::Label,
            });
        }
        let Some(was) = edit.replaces() else {
            continue;
        };
        if earlier.iter().any(|e| e.replaces() == Some(was)) {
            problems.push(ClauseNameProblem::Conflict {
                label: was,
                side: NameSide::Replaces,
            });
        } else if !has(was) {
            problems.push(ClauseNameProblem::Unknown {
                label: was,
                field: NameField::Replaces,
            });
        } else if was != edit.label() && edits.iter().any(|e| e.label() == was) {
            problems.push(ClauseNameProblem::ReplacesKept { label: was });
        }
    }
    for (n, label) in remove.iter().enumerate() {
        if remove[..n].contains(label)
            || edits
                .iter()
                .any(|e| e.label() == *label || e.replaces() == Some(*label))
        {
            problems.push(ClauseNameProblem::Conflict {
                label: *label,
                side: NameSide::Remove,
            });
        } else if !has(*label) {
            problems.push(ClauseNameProblem::Unknown {
                label: *label,
                field: NameField::Remove,
            });
        }
    }
    problems
}

/// The id of the Clause of `old` that `edit` sets: the one that has its label, or the one it replaces.
fn kept_id(old: Option<&Contract>, edit: &ClauseEdit) -> Option<ClauseId> {
    old.and_then(|k| k.clause_by_label(edit.replaces().unwrap_or(edit.label())))
        .map(Clause::id)
}

fn content_of(edit: &ClauseEdit) -> ClauseContent {
    ClauseContent::new(edit.label(), edit.text().clone())
}

impl Contract {
    /// The Clauses a Contract has after a put that gives all of them: `edits`, in the order given. A Clause of
    /// `old` (the Contract as it is, if it is one) that no edit names is gone. An edit keeps the id of the Clause of
    /// `old` that has its label, or the one it `replaces`; any other takes the id the edit carries.
    ///
    /// Every problem with the names is found, not the first: a label given twice that `old` has, a `replaces` of a
    /// label `old` does not have, named by two edits or still given. A label `old` does not have, given twice, is
    /// no problem of names: the Contract put refuses it ([`ContractError::DuplicateLabel`]).
    pub fn clauses_whole(
        old: Option<&Contract>,
        edits: &[ClauseEdit],
    ) -> Result<Vec<(ClauseId, ClauseContent)>, Vec<ClauseNameProblem>> {
        let problems = name_problems(old, edits, &[]);
        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(edits
            .iter()
            .map(|e| (kept_id(old, e).unwrap_or(e.id()), content_of(e)))
            .collect())
    }

    /// The Clauses this Contract has after a put that gives some of them: its own, in order, without the ones in
    /// `remove`; a Clause that an edit names (by its label, or as the one it `replaces`) keeps its place and id and
    /// is set to the label and sentence of the edit; any other edit is new, with the id it carries, and comes after
    /// the Clauses the Contract has, in the order given. The Clauses not named stay as they are.
    ///
    /// Every problem with the names is found, not the first: as [`Contract::clauses_whole`], and a label to
    /// remove that the Contract does not have, or that is also given, replaced or removed again.
    pub fn clauses_partial(
        &self,
        edits: &[ClauseEdit],
        remove: &[ClauseLabel],
    ) -> Result<Vec<(ClauseId, ClauseContent)>, Vec<ClauseNameProblem>> {
        let problems = name_problems(Some(self), edits, remove);
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut list: Vec<(ClauseId, ClauseContent)> = self
            .clauses
            .iter()
            .filter(|c| !remove.contains(&c.label))
            .map(|c| (c.id, ClauseContent::new(c.label, c.text.clone())))
            .collect();
        for edit in edits {
            match kept_id(Some(self), edit) {
                Some(id) => {
                    if let Some(slot) = list.iter_mut().find(|(i, _)| *i == id) {
                        slot.1 = content_of(edit);
                    }
                }
                None => list.push((edit.id(), content_of(edit))),
            }
        }
        Ok(list)
    }

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

    fn label(l: &str) -> ClauseLabel {
        ClauseLabel::try_from(l).unwrap()
    }

    /// An edit of `l` to say `text`, replacing `replaces`, with the id `new` if it makes a Clause.
    fn edit(l: &str, text: &str, replaces: Option<&str>, new: u128) -> ClauseEdit {
        ClauseEdit::new(
            label(l),
            ClauseText::try_from(text).unwrap(),
            replaces.map(label),
            cid(new),
        )
    }

    fn unknown(l: &str, field: NameField) -> ClauseNameProblem {
        ClauseNameProblem::Unknown {
            label: label(l),
            field,
        }
    }

    fn conflict(l: &str, side: NameSide) -> ClauseNameProblem {
        ClauseNameProblem::Conflict {
            label: label(l),
            side,
        }
    }

    fn ids_and_labels(list: &[(ClauseId, ClauseContent)]) -> Vec<(ClauseId, String)> {
        list.iter()
            .map(|(i, c)| (*i, format!("{} {}", c.label(), c.text().as_str())))
            .collect()
    }

    #[test]
    fn a_whole_list_is_in_the_order_given_with_the_ids_of_the_clauses_it_keeps() {
        let old = stack();
        let list = Contract::clauses_whole(
            Some(&old),
            &[
                edit("C3", "push grows the stack", None, 90),
                edit("C2", "an empty stack has no top at all", None, 91),
                edit("C4", "the same clause as C1", Some("C1"), 92),
            ],
        )
        .unwrap();
        assert_eq!(
            ids_and_labels(&list),
            [
                (cid(90), "C3 push grows the stack".to_string()),
                (cid(2), "C2 an empty stack has no top at all".to_string()),
                (cid(1), "C4 the same clause as C1".to_string()),
            ]
        );
    }

    #[test]
    fn a_whole_list_of_a_contract_that_is_not_yet_has_the_ids_of_the_edits() {
        let list =
            Contract::clauses_whole(None, &[edit("C1", "a", None, 7), edit("C2", "b", None, 8)])
                .unwrap();
        assert_eq!(
            ids_and_labels(&list),
            [(cid(7), "C1 a".to_string()), (cid(8), "C2 b".to_string())]
        );
    }

    #[test]
    fn a_label_a_contract_does_not_have_given_twice_is_left_to_the_contract_put() {
        let list =
            Contract::clauses_whole(None, &[edit("C1", "a", None, 7), edit("C1", "b", None, 8)])
                .unwrap();
        assert_eq!(
            Contract::new(ContractId::from_uuid(test_uuid(100)), title("T"), list),
            Err(ContractError::DuplicateLabel(label("C1")))
        );
    }

    #[test]
    fn a_whole_list_names_every_problem_with_the_names_at_once() {
        let old = stack();
        let problems = Contract::clauses_whole(
            Some(&old),
            &[
                edit("C1", "a", None, 7),
                edit("C1", "again", None, 8),
                edit("C5", "e", Some("C9"), 9),
                edit("C6", "f", Some("C2"), 10),
                edit("C7", "g", Some("C2"), 11),
            ],
        )
        .unwrap_err();
        assert_eq!(
            problems,
            [
                conflict("C1", NameSide::Label),
                unknown("C9", NameField::Replaces),
                conflict("C2", NameSide::Replaces),
            ]
        );
    }

    #[test]
    fn a_clause_replaced_and_still_given_under_its_old_label_is_a_problem() {
        let old = stack();
        let problems = Contract::clauses_whole(
            Some(&old),
            &[
                edit("C3", "c", Some("C1"), 7),
                edit("C1", "a new clause with the old label", None, 8),
            ],
        )
        .unwrap_err();
        assert_eq!(
            problems,
            [ClauseNameProblem::ReplacesKept { label: label("C1") }]
        );
    }

    #[test]
    fn a_partial_list_sets_the_named_in_place_keeps_the_rest_and_appends_the_new_in_order() {
        let old = stack();
        let list = old
            .clauses_partial(
                &[
                    edit("C9", "new nine", None, 90),
                    edit("C1", "pop gives back the last push", None, 91),
                    edit("C8", "new eight", None, 92),
                ],
                &[],
            )
            .unwrap();
        assert_eq!(
            ids_and_labels(&list),
            [
                (cid(1), "C1 pop gives back the last push".to_string()),
                (cid(2), "C2 an empty stack has no top".to_string()),
                (cid(90), "C9 new nine".to_string()),
                (cid(92), "C8 new eight".to_string()),
            ]
        );
    }

    #[test]
    fn a_partial_list_without_edits_or_removals_is_the_contract_as_it_is() {
        let old = stack();
        let list = old.clauses_partial(&[], &[]).unwrap();
        assert_eq!(
            ids_and_labels(&list),
            [
                (cid(1), "C1 pop gives back what push put".to_string()),
                (cid(2), "C2 an empty stack has no top".to_string()),
            ]
        );
    }

    #[test]
    fn a_partial_list_without_the_removed_and_with_a_replaced_clause_in_its_place() {
        let old = stack();
        let list = old
            .clauses_partial(&[edit("C5", "five", Some("C2"), 90)], &[label("C1")])
            .unwrap();
        assert_eq!(ids_and_labels(&list), [(cid(2), "C5 five".to_string())]);
    }

    #[test]
    fn a_partial_list_names_every_problem_with_the_names_at_once() {
        let old = stack();
        let problems = old
            .clauses_partial(
                &[
                    edit("C1", "a", None, 7),
                    edit("C1", "again", None, 8),
                    edit("C5", "e", Some("C9"), 9),
                ],
                &[label("C1"), label("C7"), label("C2"), label("C2")],
            )
            .unwrap_err();
        assert_eq!(
            problems,
            [
                conflict("C1", NameSide::Label),
                unknown("C9", NameField::Replaces),
                conflict("C1", NameSide::Remove),
                unknown("C7", NameField::Remove),
                conflict("C2", NameSide::Remove),
            ]
        );
    }

    #[test]
    fn a_label_removed_and_replaced_or_removed_twice_is_a_conflict() {
        let old = stack();
        let problems = old
            .clauses_partial(
                &[edit("C5", "five", Some("C2"), 90)],
                &[label("C2"), label("C1"), label("C1")],
            )
            .unwrap_err();
        assert_eq!(
            problems,
            [
                conflict("C2", NameSide::Remove),
                conflict("C1", NameSide::Remove)
            ]
        );
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
