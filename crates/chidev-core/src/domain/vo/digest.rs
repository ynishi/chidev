//! The fingerprint of the code a Pin accepts. The code is a cache kept outside the domain, so a Pin can only
//! record what its bytes were; the assets, whose content the domain holds, count their changes in a `Revision`.

use std::fmt;

use sha2::{Digest as _, Sha256};

/// The SHA-256 of some bytes. The bytes are not normalised (no trimming, no line-ending change): a digest that
/// forgave white space would say "unchanged" about a text that a person did change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

impl Digest {
    /// The digest of `bytes` as they are.
    pub fn of(bytes: &[u8]) -> Digest {
        Digest(Sha256::digest(bytes).into())
    }

    /// The digest written as [`Digest`]'s `Display` writes it (64 lower-case hex digits), as a store kept it;
    /// `None` for any other text.
    pub(crate) fn restore(hex: &str) -> Option<Digest> {
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let mut raw = [0u8; 32];
        for (k, byte) in raw.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).ok()?;
        }
        Some(Digest(raw))
    }
}

/// Lower-case hex, 64 digits, as `sha256sum` prints it.
impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_is_the_sha256_in_hex() {
        assert_eq!(
            Digest::of(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            Digest::of(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_digest_is_restored_from_its_hex_and_nothing_else() {
        let d = Digest::of(b"abc");
        assert_eq!(Digest::restore(&d.to_string()), Some(d));
        assert_eq!(Digest::restore("abc"), None);
        assert_eq!(Digest::restore(&d.to_string().to_uppercase()), None);
    }

    #[test]
    fn the_same_bytes_give_equal_digests_and_a_changed_byte_does_not() {
        assert_eq!(Digest::of(b"a b"), Digest::of(b"a b"));
        assert_ne!(Digest::of(b"a b"), Digest::of(b"a  b"));
        assert_ne!(Digest::of(b"a\n"), Digest::of(b"a\r\n"));
    }
}
