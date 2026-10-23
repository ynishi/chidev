//! The Pin: the record that the code of an Interface was accepted, and of what it stood on then.

use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{InterfaceId, PinId};
use crate::domain::vo::pin_basis::PinBasis;

/// The code of one Interface, accepted. The code is a cache kept outside the domain, so the Pin keeps only its
/// digest, and the revisions of the assets it was accepted against; whether it still holds is read by comparing
/// those with the Project as it is now (`service::pin::staleness`). Pinning an Interface again makes a new Pin
/// that replaces the old one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    id: PinId,
    interface: InterfaceId,
    code: Digest,
    basis: PinBasis,
}

impl Pin {
    /// A Pin of the code of `interface`, with what it stood on.
    pub fn new(id: PinId, interface: InterfaceId, code: Digest, basis: PinBasis) -> Pin {
        Pin {
            id,
            interface,
            code,
            basis,
        }
    }

    /// Its id.
    pub fn id(&self) -> PinId {
        self.id
    }

    /// The Interface whose code it is.
    pub fn interface(&self) -> InterfaceId {
        self.interface
    }

    /// The digest of the code.
    pub fn code(&self) -> Digest {
        self.code
    }

    /// What the code stood on when it was pinned.
    pub fn basis(&self) -> &PinBasis {
        &self.basis
    }
}
