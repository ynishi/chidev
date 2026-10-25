//! The Interface: what a Context shows to the others, as code, and the Items declared in it.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::vo::id::{InterfaceId, ItemId};
use crate::domain::vo::item_key::ItemKey;
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::signature::{Signature, SignatureError};
use crate::domain::vo::source::Source;

/// One declaration of an Interface. It is known by its id, so a citation of it survives a rename, and is matched by
/// its key when the code is put again; its revision says whether the declaration changed since a citation was made.
/// The declaration (its signature) is derived by the Interface from the code and the Item's lines, never given: it
/// cannot differ from what the code says. Where it is in the code (its lines) is shown but is not its content:
/// moving it changes no revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    id: ItemId,
    key: ItemKey,
    revision: Revision,
    signature: Signature,
    lines: LineRange,
}

impl Item {
    /// Its id.
    pub fn id(&self) -> ItemId {
        self.id
    }

    /// Its key: the name it is matched by, among the Items of its Interface.
    pub fn key(&self) -> &ItemKey {
        &self.key
    }

    /// How many times its declaration has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Its declaration, as the code says it on its lines (see [`Signature::of_lines`]).
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
    /// Two Items have one key. `id` is the Item that came second.
    #[error("the item key `{key}` is given twice")]
    KeyTaken {
        /// The key.
        key: ItemKey,
        /// The Item that has a key another Item has.
        id: ItemId,
    },
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
    /// The lines of an Item are blank, so it declares nothing.
    #[error("the item {id} is on lines with nothing in them")]
    BlankItem {
        /// The Item.
        id: ItemId,
    },
}

/// What a Context shows. The code is the content (it is what compiles); the Items are the declarations in it,
/// found by the caller, who knows the language: a key and the lines each is on. The domain reads no language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    id: InterfaceId,
    revision: Revision,
    source: Source,
    items: Vec<Item>,
}

/// An Item as the caller found it: its id, its key and its lines.
pub type ItemFound = (ItemId, ItemKey, LineRange);

/// An Item once its declaration is derived: id, key, signature and lines.
type Derived = (ItemId, ItemKey, Signature, LineRange);

/// The keys of `items` are each given once, then the ids are each given once, and every Item's lines are inside
/// `source` and not blank; the signature of each is derived from them. The keys are checked first: two Items
/// that are given one key have been matched to one id, and it is the key the caller has to mend.
fn derive(source: &Source, items: &[ItemFound]) -> Result<Vec<Derived>, InterfaceError> {
    for (k, (id, key, _)) in items.iter().enumerate() {
        if items[..k].iter().any(|(_, other, _)| other == key) {
            return Err(InterfaceError::KeyTaken {
                key: key.clone(),
                id: *id,
            });
        }
    }
    let mut out = Vec::with_capacity(items.len());
    for (k, (id, key, lines)) in items.iter().enumerate() {
        if items[..k].iter().any(|(i, _, _)| i == id) {
            return Err(InterfaceError::DuplicateId(*id));
        }
        let signature = Signature::of_lines(source, *lines).map_err(|e| match e {
            SignatureError::Outside => InterfaceError::RangeOutside {
                id: *id,
                first: lines.first(),
                last: lines.last(),
                total: source.line_count(),
            },
            SignatureError::Blank => InterfaceError::BlankItem { id: *id },
        })?;
        out.push((*id, key.clone(), signature, *lines));
    }
    Ok(out)
}

impl Interface {
    /// A new Interface at its first revision, with its Items in the order given, each at its first revision.
    /// The ids are the caller's: the domain makes none.
    pub fn new(
        id: InterfaceId,
        source: Source,
        items: Vec<ItemFound>,
    ) -> Result<Interface, InterfaceError> {
        let derived = derive(&source, &items)?;
        Ok(Interface {
            id,
            revision: Revision::first(),
            source,
            items: derived
                .into_iter()
                .map(|(id, key, signature, lines)| Item {
                    id,
                    key,
                    revision: Revision::first(),
                    signature,
                    lines,
                })
                .collect(),
        })
    }

    /// An Interface as a store kept it, revisions included (the signatures are not kept: they are derived again).
    /// The rules of [`Interface::new`] hold on it.
    pub(crate) fn restore(
        id: InterfaceId,
        revision: Revision,
        source: Source,
        items: Vec<(ItemId, Revision, ItemKey, LineRange)>,
    ) -> Result<Interface, InterfaceError> {
        let found: Vec<ItemFound> = items
            .iter()
            .map(|(i, _, k, l)| (*i, k.clone(), *l))
            .collect();
        let derived = derive(&source, &found)?;
        Ok(Interface {
            id,
            revision,
            source,
            items: derived
                .into_iter()
                .zip(items)
                .map(|((id, key, signature, lines), (_, revision, _, _))| Item {
                    id,
                    key,
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

    /// The Item with this key.
    pub fn item_by_key(&self, key: &ItemKey) -> Option<&Item> {
        self.items.iter().find(|i| i.key == *key)
    }

    /// Gives the Interface new code and Items. An Item whose id was there keeps its revision when its declaration
    /// (the signature derived from its lines in the new code) is the same, wherever it now is, and goes to the next
    /// one when it differs; a new id starts at the first; an id left out is gone. The Interface goes to its next
    /// revision when the code or its Items (which ones, in what order, declaring what) differ, and stays when
    /// nothing does. The key is the identity, not the content: an Item given another key (a rename) keeps its
    /// revision, and the Interface keeps its, when the code and the declarations are the same, so that a citation
    /// of it, which is by id, stays. Content that breaks a rule changes nothing.
    pub fn revise(&mut self, source: Source, items: Vec<ItemFound>) -> Result<(), InterfaceError> {
        let derived = derive(&source, &items)?;
        let revised: Vec<Item> = derived
            .into_iter()
            .map(|(id, key, signature, lines)| {
                let revision = match self.item(id) {
                    None => Revision::first(),
                    Some(old) if old.signature == signature => old.revision,
                    Some(old) => old.revision.next(),
                };
                Item {
                    id,
                    key,
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

    /// Gives some of its Items another key and other lines, in the code it has: the Items of `map` (by id) take the
    /// key and the lines given, and the others stay as they are. An id the Interface does not have is not used.
    /// The revisions follow the rules of [`Interface::revise`]: an Item whose key alone changes keeps its revision
    /// and so does the Interface, and an Item whose new lines declare something else (its derived signature
    /// differs) goes to the next revision, and so does the Interface. A key another Item has is refused
    /// ([`InterfaceError::KeyTaken`]), as are lines outside the code or blank, and nothing is changed.
    pub fn rekey(
        &mut self,
        map: &BTreeMap<ItemId, (ItemKey, LineRange)>,
    ) -> Result<(), InterfaceError> {
        let items = self
            .items
            .iter()
            .map(|it| match map.get(&it.id) {
                Some((key, lines)) => (it.id, key.clone(), *lines),
                None => (it.id, it.key.clone(), it.lines),
            })
            .collect();
        self.revise(self.source.clone(), items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;

    fn iid(n: u128) -> ItemId {
        ItemId::from_uuid(test_uuid(n))
    }

    fn key(s: &str) -> ItemKey {
        ItemKey::try_from(s).unwrap()
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
                (iid(1), key("push"), at(1, 1)),
                (iid(2), key("pop"), at(2, 2)),
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
        assert_eq!(i.item(iid(2)).unwrap().key().as_str(), "pop");
        assert_eq!(
            i.item(iid(2)).unwrap().signature().as_str(),
            "pub fn pop() -> u8"
        );
        assert_eq!(i.item_by_key(&key("push")).unwrap().id(), iid(1));
        assert!(i.item(iid(9)).is_none());
        assert!(i.item_by_key(&key("peek")).is_none());
    }

    #[test]
    fn the_signature_is_derived_from_the_lines_of_the_code() {
        let i = Interface::new(
            InterfaceId::from_uuid(test_uuid(100)),
            src("/// Adds.\npub fn add(\n    a: u8,  \n) -> u8 {\n    a\n}\n"),
            vec![
                (iid(1), key("add"), at(2, 4)),
                (iid(2), key("add with doc"), at(1, 4)),
            ],
        )
        .unwrap();
        assert_eq!(
            i.item(iid(1)).unwrap().signature().as_str(),
            "pub fn add(\n    a: u8,\n) -> u8 {"
        );
        assert_eq!(
            i.item(iid(2)).unwrap().signature().as_str(),
            "/// Adds.\npub fn add(\n    a: u8,\n) -> u8 {"
        );
    }

    #[test]
    fn two_items_may_declare_the_same_name_but_not_have_one_key_or_one_id() {
        let id = InterfaceId::from_uuid(test_uuid(100));
        let code = src("fn a()\nfn a()\n");
        let made = Interface::new(
            id,
            code.clone(),
            vec![
                (iid(1), key("a"), at(1, 1)),
                (iid(2), key("a #2"), at(2, 2)),
            ],
        )
        .unwrap();
        assert_eq!(
            made.item(iid(1)).unwrap().signature(),
            made.item(iid(2)).unwrap().signature()
        );
        assert_eq!(
            Interface::new(
                id,
                code.clone(),
                vec![(iid(1), key("a"), at(1, 1)), (iid(2), key("a"), at(2, 2))]
            ),
            Err(InterfaceError::KeyTaken {
                key: key("a"),
                id: iid(2)
            })
        );
        assert_eq!(
            Interface::new(
                id,
                code,
                vec![(iid(1), key("a"), at(1, 1)), (iid(1), key("b"), at(2, 2))]
            ),
            Err(InterfaceError::DuplicateId(iid(1)))
        );
    }

    #[test]
    fn a_key_given_twice_is_refused_before_an_id_given_twice() {
        // the same existing key twice in one put: both are matched to one id, and it is the key that is named
        let id = InterfaceId::from_uuid(test_uuid(100));
        let twice = vec![
            (iid(1), key("push"), at(1, 1)),
            (iid(1), key("push"), at(2, 2)),
        ];
        let expected = InterfaceError::KeyTaken {
            key: key("push"),
            id: iid(1),
        };
        assert_eq!(
            Interface::new(id, src(CODE), twice.clone()),
            Err(expected.clone())
        );
        let mut i = stack();
        assert_eq!(i.revise(src(CODE), twice), Err(expected));
        // an id given twice under keys that differ is still an id given twice, and a key given twice wins over it
        // wherever it stands in the list
        assert_eq!(
            Interface::new(
                id,
                src(CODE),
                vec![
                    (iid(1), key("push"), at(1, 1)),
                    (iid(1), key("pop"), at(2, 2)),
                    (iid(2), key("pop"), at(2, 2)),
                ]
            ),
            Err(InterfaceError::KeyTaken {
                key: key("pop"),
                id: iid(2)
            })
        );
    }

    #[test]
    fn an_item_past_the_last_line_is_refused() {
        assert_eq!(
            Interface::new(
                InterfaceId::from_uuid(test_uuid(100)),
                src("fn a()\n"),
                vec![(iid(1), key("a"), at(1, 2))]
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
    fn an_item_on_blank_lines_is_refused_whether_it_is_new_revised_or_restored() {
        let id = InterfaceId::from_uuid(test_uuid(100));
        let blank = || vec![(iid(1), key("a"), at(2, 3))];
        let code = "fn a()\n  \n\t\n";
        assert_eq!(
            Interface::new(id, src(code), blank()),
            Err(InterfaceError::BlankItem { id: iid(1) })
        );
        let mut i = stack();
        let before = i.clone();
        assert_eq!(
            i.revise(src(code), blank()),
            Err(InterfaceError::BlankItem { id: iid(1) })
        );
        assert_eq!(i, before);
        assert_eq!(
            Interface::restore(
                id,
                Revision::first(),
                src(code),
                vec![(iid(1), Revision::first(), key("a"), at(2, 3))]
            ),
            Err(InterfaceError::BlankItem { id: iid(1) })
        );
    }

    #[test]
    fn a_restored_interface_derives_the_signatures_and_keeps_the_revisions() {
        let mut i = stack();
        i.revise(
            src("pub fn push(v: u16)\npub fn pop() -> u8\n"),
            vec![
                (iid(1), key("push"), at(1, 1)),
                (iid(2), key("pop"), at(2, 2)),
            ],
        )
        .unwrap();
        let back = Interface::restore(
            i.id(),
            i.revision(),
            i.source().clone(),
            i.items()
                .iter()
                .map(|it| (it.id(), it.revision(), it.key().clone(), it.lines()))
                .collect(),
        )
        .unwrap();
        assert_eq!(back, i);
        assert_eq!(revisions(&back), [2, 1]);
    }

    #[test]
    fn the_same_code_and_items_again_change_no_revision() {
        let mut i = stack();
        let before = i.clone();
        i.revise(
            src(CODE),
            vec![
                (iid(1), key("push"), at(1, 1)),
                (iid(2), key("pop"), at(2, 2)),
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
                (iid(1), key("push"), at(2, 2)),
                (iid(2), key("pop"), at(3, 3)),
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
                (iid(1), key("push"), at(1, 1)),
                (iid(2), key("pop"), at(2, 2)),
            ],
        )
        .unwrap();
        assert_eq!(i.revision().number(), 2);
        assert_eq!(revisions(&i), [2, 1]);
    }

    #[test]
    fn a_rename_alone_changes_no_revision_of_the_item_or_the_interface() {
        let mut i = stack();
        i.revise(
            src(CODE),
            vec![
                (iid(1), key("Stack::push"), at(1, 1)),
                (iid(2), key("Stack::pop"), at(2, 2)),
            ],
        )
        .unwrap();
        assert_eq!(i.revision(), Revision::first());
        assert_eq!(revisions(&i), [1, 1]);
        assert_eq!(i.item(iid(1)).unwrap().key().as_str(), "Stack::push");
        assert!(i.item_by_key(&key("push")).is_none());
        assert_eq!(i.item_by_key(&key("Stack::pop")).unwrap().id(), iid(2));
    }

    #[test]
    fn a_rename_with_a_changed_declaration_goes_to_the_next_revision() {
        let mut i = stack();
        i.revise(
            src("pub fn push(v: u16)\npub fn pop() -> u8\n"),
            vec![
                (iid(1), key("Stack::push"), at(1, 1)),
                (iid(2), key("pop"), at(2, 2)),
            ],
        )
        .unwrap();
        assert_eq!(i.revision().number(), 2);
        assert_eq!(revisions(&i), [2, 1]);
    }

    fn rekeyed(entries: &[(u128, &str, usize, usize)]) -> BTreeMap<ItemId, (ItemKey, LineRange)> {
        entries
            .iter()
            .map(|(n, k, first, last)| (iid(*n), (key(k), at(*first, *last))))
            .collect()
    }

    #[test]
    fn a_rekey_of_the_key_alone_moves_no_revision_of_the_item_or_the_interface() {
        let mut i = stack();
        i.rekey(&rekeyed(&[(1, "Stack::push", 1, 1)])).unwrap();
        assert_eq!(i.revision(), Revision::first());
        assert_eq!(revisions(&i), [1, 1]);
        assert_eq!(i.item(iid(1)).unwrap().key().as_str(), "Stack::push");
        // the Item that is not in the map is as it was
        assert_eq!(i.item(iid(2)).unwrap().key().as_str(), "pop");
        assert_eq!(i.source().as_str(), CODE);
    }

    #[test]
    fn a_rekey_to_lines_that_declare_something_else_goes_to_the_next_revision() {
        let mut i = Interface::new(
            InterfaceId::from_uuid(test_uuid(100)),
            src("fn push(v: u8)\nfn pop() -> u8\nfn peek()\n"),
            vec![
                (iid(1), key("fn push(v: u8)"), at(1, 1)),
                (iid(2), key("fn pop() -> u8"), at(2, 2)),
                (iid(3), key("fn peek()"), at(3, 3)),
            ],
        )
        .unwrap();
        // the first now spans a line more (its signature differs); the second keeps the line it was on; the
        // third is on the same line with another key
        i.rekey(&rekeyed(&[
            (1, "push", 1, 2),
            (2, "pop", 2, 2),
            (3, "peek", 3, 3),
        ]))
        .unwrap();
        assert_eq!(i.revision().number(), 2);
        assert_eq!(revisions(&i), [2, 1, 1]);
        assert_eq!(i.item(iid(1)).unwrap().lines(), at(1, 2));
        assert_eq!(
            i.item(iid(1)).unwrap().signature().as_str(),
            "fn push(v: u8)\nfn pop() -> u8"
        );
    }

    #[test]
    fn a_rekey_that_gives_a_key_another_item_has_is_refused_and_changes_nothing() {
        let mut i = stack();
        let before = i.clone();
        assert_eq!(
            i.rekey(&rekeyed(&[(1, "pop", 1, 1)])),
            Err(InterfaceError::KeyTaken {
                key: key("pop"),
                id: iid(2)
            })
        );
        assert_eq!(i, before);
        // two Items that take one new key
        assert_eq!(
            i.rekey(&rekeyed(&[(1, "top", 1, 1), (2, "top", 2, 2)])),
            Err(InterfaceError::KeyTaken {
                key: key("top"),
                id: iid(2)
            })
        );
        assert_eq!(i, before);
    }

    #[test]
    fn a_rekey_may_swap_two_keys_and_ignores_an_id_the_interface_does_not_have() {
        let mut i = stack();
        i.rekey(&rekeyed(&[
            (1, "pop", 1, 1),
            (2, "push", 2, 2),
            (9, "other", 1, 1),
        ]))
        .unwrap();
        assert_eq!(i.item(iid(1)).unwrap().key().as_str(), "pop");
        assert_eq!(i.item(iid(2)).unwrap().key().as_str(), "push");
        assert!(i.item(iid(9)).is_none());
        assert_eq!(i.items().len(), 2);
    }

    #[test]
    fn a_rekey_to_lines_outside_the_code_or_blank_is_refused_and_changes_nothing() {
        let mut i = stack();
        let before = i.clone();
        assert_eq!(
            i.rekey(&rekeyed(&[(1, "push", 1, 3)])),
            Err(InterfaceError::RangeOutside {
                id: iid(1),
                first: 1,
                last: 3,
                total: 2
            })
        );
        assert_eq!(i, before);
        let mut blank = Interface::new(
            InterfaceId::from_uuid(test_uuid(100)),
            src("fn a()\n\n"),
            vec![(iid(1), key("a"), at(1, 1))],
        )
        .unwrap();
        let before = blank.clone();
        assert_eq!(
            blank.rekey(&rekeyed(&[(1, "a", 2, 2)])),
            Err(InterfaceError::BlankItem { id: iid(1) })
        );
        assert_eq!(blank, before);
    }

    #[test]
    fn content_that_breaks_a_rule_changes_nothing() {
        let mut i = stack();
        let before = i.clone();
        assert!(
            i.revise(src("fn a()\n"), vec![(iid(1), key("a"), at(1, 3))])
                .is_err()
        );
        assert_eq!(i, before);
    }
}
