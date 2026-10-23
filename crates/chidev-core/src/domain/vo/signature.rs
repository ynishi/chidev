//! The declaration of an Item of an Interface, as the code writes it.

use std::fmt;

use thiserror::Error;

/// Why a text is not a signature.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignatureError {
    /// It is empty or only white space.
    #[error("a signature is empty")]
    Empty,
}

/// The declaration of an Item (`fn push(&mut self, v: T)`), kept as the code writes it. It is content, not the
/// identity (that is the `ItemId`): the domain does not read it, so it holds no rule about what a name or a path
/// in it looks like, and two Items may declare the same name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signature(String);

impl Signature {
    /// The declaration as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Signature {
    type Error = SignatureError;

    fn try_from(text: &str) -> Result<Signature, SignatureError> {
        if text.trim().is_empty() {
            return Err(SignatureError::Empty);
        }
        Ok(Signature(text.to_string()))
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_is_kept_as_written() {
        assert_eq!(
            Signature::try_from("fn push(&mut self, v: T)")
                .unwrap()
                .as_str(),
            "fn push(&mut self, v: T)"
        );
    }

    #[test]
    fn a_blank_signature_is_refused() {
        assert_eq!(Signature::try_from(" \n"), Err(SignatureError::Empty));
    }
}
