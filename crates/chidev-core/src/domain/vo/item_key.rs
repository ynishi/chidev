//! The name an Item is known by in its Interface.

use std::fmt;

use thiserror::Error;

/// Why a text is not an Item key.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ItemKeyError {
    /// There is nothing in it.
    #[error("an item key is empty")]
    Empty,
    /// It has a line break; a key is one line.
    #[error("the item key `{0}` has a line break; a key is one line")]
    LineBreak(String),
}

/// What names an Item among the Items of its Interface (`price`, `Stack::push`): the caller's, or its language
/// provider's. It is the identity the Item is matched by when the code is put again, not its content (that is the
/// declaration, derived from the code): a key that changes is a rename. The domain does not read it, so it holds no
/// rule about what a name or a path in it looks like; it only has to be told apart from the keys beside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemKey(String);

impl ItemKey {
    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ItemKey {
    type Error = ItemKeyError;

    fn try_from(text: &str) -> Result<ItemKey, ItemKeyError> {
        if text.is_empty() {
            return Err(ItemKeyError::Empty);
        }
        if text.contains(['\n', '\r']) {
            return Err(ItemKeyError::LineBreak(text.to_string()));
        }
        Ok(ItemKey(text.to_string()))
    }
}

impl fmt::Display for ItemKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_kept_as_written_and_its_content_is_not_read() {
        for text in [
            "price",
            "Stack<u8>::push",
            "fn price(total: u64) -> u64",
            " a b ",
        ] {
            assert_eq!(ItemKey::try_from(text).unwrap().as_str(), text);
        }
    }

    #[test]
    fn an_empty_key_is_refused() {
        assert_eq!(ItemKey::try_from(""), Err(ItemKeyError::Empty));
    }

    #[test]
    fn a_key_with_a_line_break_is_refused() {
        for bad in ["a\nb", "a\r\nb", "a\rb", "\n"] {
            assert_eq!(
                ItemKey::try_from(bad),
                Err(ItemKeyError::LineBreak(bad.to_string()))
            );
        }
    }
}
