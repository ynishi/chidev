//! The Interface: what a Context shows to the others, as code, and the Items declared in it.

use thiserror::Error;

use crate::domain::vo::id::{InterfaceId, ItemId};
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::signature::Signature;
use crate::domain::vo::source::Source;

/// One declaration of an Interface. It is known by its id, so a citation of it survives a rename; its revision
/// says whether the declaration changed since a citation was made. Where it is in the code (its lines) is shown
/// but is not its content: moving it changes no revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    id: ItemId,
    revision: Revision,
    signature: Signature,
    lines: LineRange,
}

impl Item {
    /// Its id.
    pub fn id(&self) -> ItemId {
        self.id
    }

    /// How many times its declaration has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Its declaration.
    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    /// The lines of the code it is declared on.
    pub fn lines(&self) -> LineRange {
        self.lines
    }
}

/// Why code and a list of Items are not an Interface. The Interface is not made or changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InterfaceError {
    /// Two Items have one id.
    #[error("the item id {0} is given twice")]
    DuplicateId(ItemId),
    /// The lines of an Item are not inside the code.
    #[error("the item {id} is on lines {first}..={last}, past the {total} lines of the code")]
    RangeOutside {
        /// The Item.
        id: ItemId,
        /// The first line it was given.
        first: usize,
        /// The last line it was given.
        last: usize,
        /// How many lines the code has.
        total: usize,
    },
}

/// What a Context shows. The code is the content (it is what compiles); the Items are the declarations in it,
/// found by the caller, who knows the language. The domain reads no language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    id: InterfaceId,
    revision: Revision,
    source: Source,
    items: Vec<Item>,
}

/// An Item as the caller found it: its id, its declaration and its lines.
pub type ItemFound = (ItemId, Signature, LineRange);

/// The ids of `items` are each given once, and every Item's lines are inside `source`.
fn check(source: &Source, items: &[ItemFound]) -> Result<(), InterfaceError> {
    for (k, (id, _, lines)) in items.iter().enumerate() {
        if items[..k].iter().any(|(i, _, _)| i == id) {
            return Err(InterfaceError::DuplicateId(*id));
        }
        if source.lines(*lines).is_none() {
            return Err(InterfaceError::RangeOutside {
                id: *id,
                first: lines.first(),
                last: lines.last(),
                total: source.line_count(),
            });
        }
    }
    Ok(())
}

impl Interface {
    /// A new Interface at its first revision, with its Items in the order given, each at its first revision.
    /// The ids are the caller's: the domain makes none.
    pub fn new(
        id: InterfaceId,
        source: Source,
        items: Vec<ItemFound>,
    ) -> Result<Interface, InterfaceError> {
        check(&source, &items)?;
        Ok(Interface {
            id,
            revision: Revision::first(),
            source,
            items: items
                .into_iter()
                .map(|(id, signature, lines)| Item {
                    id,
                    revision: Revision::first(),
                    signature,
                    lines,
                })
                .collect(),
        })
    }

    /// An Interface as a store kept it, revisions included. The rules of [`Interface::new`] hold on it.
    pub(crate) fn restore(
        id: InterfaceId,
        revision: Revision,
        source: Source,
        items: Vec<(ItemId, Revision, Signature, LineRange)>,
    ) -> Result<Interface, InterfaceError> {
        let found: Vec<ItemFound> = items
            .iter()
            .map(|(i, _, s, l)| (*i, s.clone(), *l))
            .collect();
        check(&source, &found)?;
        Ok(Interface {
            id,
            revision,
            source,
            items: items
                .into_iter()
                .map(|(id, revision, signature, lines)| Item {
                    id,
                    revision,
                    signature,
                    lines,
                })
                .collect(),
        })
    }

    /// Its id.
    pub fn id(&self) -> InterfaceId {
        self.id
    }

    /// How many times its content has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Its code.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Its Items, in order.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The Item with this id.
    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Gives the Interface new code and Items. An Item whose id was there keeps its revision when its declaration
    /// is the same (wherever it now is) and goes to the next one when it differs; a new id starts at the first; an
    /// id left out is gone. The Interface goes to its next revision when the code or its Items (which ones, in
    /// what order, declaring what) differ, and stays when nothing does. Content that breaks a rule changes
    /// nothing.
    pub fn revise(&mut self, source: Source, items: Vec<ItemFound>) -> Result<(), InterfaceError> {
        check(&source, &items)?;
        let revised: Vec<Item> = items
            .into_iter()
            .map(|(id, signature, lines)| {
                let revision = match self.item(id) {
                    None => Revision::first(),
                    Some(old) if old.signature == signature => old.revision,
                    Some(old) => old.revision.next(),
                };
                Item {
                    id,
                    revision,
                    signature,
                    lines,
                }
            })
            .collect();
        let same = source == self.source
            && revised.len() == self.items.len()
            && revised
                .iter()
                .zip(&self.items)
                .all(|(new, old)| new.id == old.id && new.signature == old.signature);
        if !same {
            self.revision = self.revision.next();
        }
        self.source = source;
        self.items = revised;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;

    fn iid(n: u128) -> ItemId {
        ItemId::from_uuid(test_uuid(n))
    }

    fn sig(s: &str) -> Signature {
        Signature::try_from(s).unwrap()
    }

    fn at(first: usize, last: usize) -> LineRange {
        LineRange::new(first, last).unwrap()
    }

    fn src(s: &str) -> Source {
        Source::new(s.to_string())
    }

    const CODE: &str = "pub fn push(v: u8)\npub fn pop() -> u8\n";

    fn stack() -> Interface {
        Interface::new(
            InterfaceId::from_uuid(test_uuid(100)),
            src(CODE),
            vec![
                (iid(1), sig("fn push(v: u8)"), at(1, 1)),
                (iid(2), sig("fn pop() -> u8"), at(2, 2)),
            ],
        )
        .unwrap()
    }

    fn revisions(i: &Interface) -> Vec<u64> {
        i.items().iter().map(|i| i.revision().number()).collect()
    }

    #[test]
    fn a_new_interface_and_its_items_start_at_the_first_revision() {
        let i = stack();
        assert_eq!(i.revision(), Revision::first());
        assert_eq!(i.source().as_str(), CODE);
        assert_eq!(revisions(&i), [1, 1]);
        assert_eq!(
            i.item(iid(2)).unwrap().signature().as_str(),
            "fn pop() -> u8"
        );
        assert!(i.item(iid(9)).is_none());
    }

    #[test]
    fn two_items_may_declare_the_same_name_but_not_have_one_id() {
        let id = InterfaceId::from_uuid(test_uuid(100));
        let code = src("fn a()\nfn a()\n");
        assert!(
            Interface::new(
                id,
                code.clone(),
                vec![
                    (iid(1), sig("fn a()"), at(1, 1)),
                    (iid(2), sig("fn a()"), at(2, 2))
                ]
            )
            .is_ok()
        );
        assert_eq!(
            Interface::new(
                id,
                code,
                vec![
                    (iid(1), sig("fn a()"), at(1, 1)),
                    (iid(1), sig("fn a()"), at(2, 2))
                ]
            ),
            Err(InterfaceError::DuplicateId(iid(1)))
        );
    }

    #[test]
    fn an_item_past_the_last_line_is_refused() {
        assert_eq!(
            Interface::new(
                InterfaceId::from_uuid(test_uuid(100)),
                src("fn a()\n"),
                vec![(iid(1), sig("fn a()"), at(1, 2))]
            ),
            Err(InterfaceError::RangeOutside {
                id: iid(1),
                first: 1,
                last: 2,
                total: 1
            })
        );
    }

    #[test]
    fn the_same_code_and_items_again_change_no_revision() {
        let mut i = stack();
        let before = i.clone();
        i.revise(
            src(CODE),
            vec![
                (iid(1), sig("fn push(v: u8)"), at(1, 1)),
                (iid(2), sig("fn pop() -> u8"), at(2, 2)),
            ],
        )
        .unwrap();
        assert_eq!(i, before);
    }

    #[test]
    fn a_moved_item_keeps_its_revision_and_the_interface_goes_to_the_next() {
        let mut i = stack();
        i.revise(
            src("// stack\npub fn push(v: u8)\npub fn pop() -> u8\n"),
            vec![
                (iid(1), sig("fn push(v: u8)"), at(2, 2)),
                (iid(2), sig("fn pop() -> u8"), at(3, 3)),
            ],
        )
        .unwrap();
        assert_eq!(i.revision().number(), 2);
        assert_eq!(revisions(&i), [1, 1]);
        assert_eq!(i.item(iid(1)).unwrap().lines(), at(2, 2));
    }

    #[test]
    fn a_changed_signature_goes_to_the_next_revision_and_the_others_stay() {
        let mut i = stack();
        i.revise(
            src("pub fn push(v: u16)\npub fn pop() -> u8\n"),
            vec![
                (iid(1), sig("fn push(v: u16)"), at(1, 1)),
                (iid(2), sig("fn pop() -> u8"), at(2, 2)),
            ],
        )
        .unwrap();
        assert_eq!(i.revision().number(), 2);
        assert_eq!(revisions(&i), [2, 1]);
    }

    #[test]
    fn content_that_breaks_a_rule_changes_nothing() {
        let mut i = stack();
        let before = i.clone();
        assert!(
            i.revise(src("fn a()\n"), vec![(iid(1), sig("fn a()"), at(1, 3))])
                .is_err()
        );
        assert_eq!(i, before);
    }
}
