//! The names a person gives: a Context's, and an asset's (a Contract's, an Interface's, a Harness's).

use std::fmt;

use thiserror::Error;

/// Which kind of name a text was meant to be, for the message that refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameOf {
    /// A Context's name.
    Context,
    /// An asset's name.
    Asset,
}

impl fmt::Display for NameOf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NameOf::Context => "a context name",
            NameOf::Asset => "an asset name",
        })
    }
}

/// Why a text is not a name. A Context's name and an asset's follow one rule, and say which they are.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NameError {
    /// There is nothing in it.
    #[error("{of} is empty")]
    Empty {
        /// Which kind of name it was meant to be.
        of: NameOf,
    },
    /// It has a character other than a lower-case letter, a digit, `-` or `_`.
    #[error("{of} cannot be `{name}`: it has `{ch}`; only a-z, 0-9, `-` and `_` are allowed")]
    BadChar {
        /// Which kind of name it was meant to be.
        of: NameOf,
        /// The text that was refused.
        name: String,
        /// The first character not allowed.
        ch: char,
    },
}

/// The one rule of a name: not empty, and only ASCII lower-case letters, ASCII digits, `-` and `_`. The characters
/// are limited so that a name can never contain the `#` or `::` that a citation uses to end it, nor white space,
/// which ends a name in the report of a script.
fn check(of: NameOf, text: &str) -> Result<(), NameError> {
    if text.is_empty() {
        return Err(NameError::Empty { of });
    }
    if let Some(ch) = text
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_'))
    {
        return Err(NameError::BadChar {
            of,
            name: text.to_string(),
            ch,
        });
    }
    Ok(())
}

/// What a person calls a Context, in citations and on the command line. It is an attribute, not the identity
/// (that is the `ContextId`), so it may change. It follows the rule of every name (see [`AssetName`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContextName(String);

impl ContextName {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ContextName {
    type Error = NameError;

    fn try_from(text: &str) -> Result<ContextName, NameError> {
        check(NameOf::Context, text)?;
        Ok(ContextName(text.to_string()))
    }
}

impl TryFrom<String> for ContextName {
    type Error = NameError;

    fn try_from(text: String) -> Result<ContextName, NameError> {
        check(NameOf::Context, &text)?;
        Ok(ContextName(text))
    }
}

impl fmt::Display for ContextName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a person calls a Contract, an Interface or a Harness, in citations and on the command line: one for each
/// asset within its Context and kind. Like a Context's name it is an attribute, not the identity, and it is
/// not empty and has only ASCII lower-case letters, ASCII digits, `-` and `_`, so that it can never contain the `#`
/// or `::` that a citation uses to end it, nor white space.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetName(String);

impl AssetName {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for AssetName {
    type Error = NameError;

    fn try_from(text: &str) -> Result<AssetName, NameError> {
        check(NameOf::Asset, text)?;
        Ok(AssetName(text.to_string()))
    }
}

impl TryFrom<String> for AssetName {
    type Error = NameError;

    fn try_from(text: String) -> Result<AssetName, NameError> {
        check(NameOf::Asset, &text)?;
        Ok(AssetName(text))
    }
}

impl fmt::Display for AssetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCEPTED: [&str; 4] = ["core-2_a", "a", "0", "_-"];
    const REFUSED: [&str; 8] = ["Core", "a b", "a#b", "a:b", "a::b", "a.b", "a/b", "é"];

    #[test]
    fn a_name_of_lower_case_digits_dash_and_underscore_is_accepted() {
        for good in ACCEPTED {
            assert_eq!(ContextName::try_from(good).unwrap().as_str(), good);
            assert_eq!(AssetName::try_from(good).unwrap().as_str(), good);
            assert_eq!(
                AssetName::try_from(good.to_string()).unwrap().to_string(),
                good
            );
        }
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert_eq!(
            ContextName::try_from(""),
            Err(NameError::Empty {
                of: NameOf::Context
            })
        );
        assert_eq!(
            AssetName::try_from(""),
            Err(NameError::Empty { of: NameOf::Asset })
        );
    }

    #[test]
    fn a_name_with_another_character_is_refused_and_the_character_is_named() {
        let first_bad = |name: &str| name.chars().find(|c| !c.is_ascii_lowercase()).unwrap();
        for bad in REFUSED {
            let ch = first_bad(bad);
            assert_eq!(
                ContextName::try_from(bad),
                Err(NameError::BadChar {
                    of: NameOf::Context,
                    name: bad.to_string(),
                    ch
                }),
                "{bad}"
            );
            assert_eq!(
                AssetName::try_from(bad.to_string()),
                Err(NameError::BadChar {
                    of: NameOf::Asset,
                    name: bad.to_string(),
                    ch
                }),
                "{bad}"
            );
        }
    }

    #[test]
    fn both_kinds_of_name_follow_one_rule() {
        for text in ACCEPTED.into_iter().chain(REFUSED).chain([""]) {
            assert_eq!(
                ContextName::try_from(text).is_ok(),
                AssetName::try_from(text).is_ok(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_message_says_which_kind_of_name_it_refuses() {
        let context = ContextName::try_from("a b").unwrap_err().to_string();
        let asset = AssetName::try_from("a b").unwrap_err().to_string();
        assert!(context.starts_with("a context name"), "{context}");
        assert!(asset.starts_with("an asset name"), "{asset}");
        assert!(asset.contains("` `"), "{asset}");
        assert!(
            AssetName::try_from("")
                .unwrap_err()
                .to_string()
                .starts_with("an asset name")
        );
        assert!(
            ContextName::try_from("")
                .unwrap_err()
                .to_string()
                .starts_with("a context name")
        );
    }
}
