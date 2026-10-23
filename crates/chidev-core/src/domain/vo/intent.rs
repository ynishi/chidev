//! What a change request means to do to the behaviour, and who raised it.

use std::fmt;

use thiserror::Error;

/// Whether a change request means to change what the specification promises. Where it came from (a person outside,
/// the system itself) does not matter: a request from outside may only tidy, and a refactor may need a promise to
/// move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    /// The behaviour the specification promises is to change.
    ChangeBehavior,
    /// The behaviour stays as promised: a refactor, a conflict removed, a structure tidied.
    KeepBehavior,
}

impl Intent {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Intent::ChangeBehavior => "change_behavior",
            Intent::KeepBehavior => "keep_behavior",
        }
    }
}

/// Why a text is not an issuer.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IssuerError {
    /// It is empty or only white space.
    #[error("an issuer is empty")]
    Empty,
}

/// Who raised a change request: a person or an agent, by the name they go by. It is a label for the record, not an
/// account: the domain checks nothing about it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Issuer(String);

impl Issuer {
    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Issuer {
    type Error = IssuerError;

    fn try_from(text: &str) -> Result<Issuer, IssuerError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(IssuerError::Empty);
        }
        Ok(Issuer(text.to_string()))
    }
}

impl fmt::Display for Issuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_issuer_is_a_name_not_blank() {
        assert_eq!(Issuer::try_from(" designer ").unwrap().as_str(), "designer");
        assert_eq!(Issuer::try_from(" "), Err(IssuerError::Empty));
    }
}
