//! The ids of what is kept over time: the aggregates, the assets, and the definitions inside an asset.
//!
//! An id is what a link points at, so a rename or an edit of the text never breaks one. The domain never makes
//! an id (no random source, no clock): the caller hands it in, as git hands a hash to its object model.

use std::fmt;

use uuid::Uuid;

/// Makes an id type that wraps a `Uuid`, with the field private and `from_uuid` as the only way in.
macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            /// The id made of a uuid the caller made.
            pub fn from_uuid(uuid: Uuid) -> $name {
                $name(uuid)
            }

            /// The uuid inside, for the layer that stores it.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

uuid_id!(
    /// The identity of a Project, the root that holds the Contexts of one repository.
    ProjectId
);
uuid_id!(
    /// The identity of a Context; it stays when the Context is renamed.
    ContextId
);
uuid_id!(
    /// The identity of a Contract, an asset of a Context.
    ContractId
);
uuid_id!(
    /// The identity of an Interface, an asset of a Context.
    InterfaceId
);
uuid_id!(
    /// The identity of a Harness, an asset of a Context.
    HarnessId
);
uuid_id!(
    /// The identity of a Clause of a Contract; it stays when the Clause is relabelled or reworded.
    ClauseId
);
uuid_id!(
    /// The identity of an Item of an Interface; it stays when the Item is renamed or its signature changes.
    ItemId
);
uuid_id!(
    /// The identity of a Case of a Harness; it stays when the Case is renamed.
    CaseId
);
uuid_id!(
    /// The identity of a Pin.
    PinId
);
uuid_id!(
    /// The identity of a Crux.
    CruxId
);

/// A fixed uuid for the tests: the domain does not make ids, so a test makes them from a number.
#[cfg(test)]
pub(crate) fn test_uuid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_keeps_the_uuid_it_is_made_from() {
        let u = test_uuid(7);
        assert_eq!(ContextId::from_uuid(u).as_uuid(), u);
        assert_eq!(ClauseId::from_uuid(u).as_uuid(), u);
        assert_eq!(CruxId::from_uuid(u).to_string(), u.to_string());
    }

    #[test]
    fn ids_of_different_uuids_differ() {
        assert_ne!(
            ContextId::from_uuid(test_uuid(1)),
            ContextId::from_uuid(test_uuid(2))
        );
    }
}
