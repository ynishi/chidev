//! The port of the language providers that find the Items of an Interface: what the use case hands over (the
//! language and the code) and what comes back (a key and the lines of each declaration). How a provider is found,
//! started and spoken to is the adapter's; what the use case does with the answer (the signature of an Item, its id,
//! its revision) is not.

use std::fmt;

use thiserror::Error;

use super::measure::BoxFuture;

/// One declaration a provider found: its key and the first and last line of it, from 1. The adapter has checked
/// that the key is a key and that the lines are in the code; it has not checked that the declarations are apart
/// (one line may hold two).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundItem {
    /// The name the Item is matched by when the code is put again.
    pub key: String,
    /// The first and last line of the declaration.
    pub lines: (usize, usize),
}

/// Why the Items of an Interface could not be found. Nothing was kept.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ItemsError {
    /// The configuration lays the Interface out with no language (or does not lay it out at all), so there is no
    /// provider to ask.
    #[error("the interface has no language to find its items by")]
    NoLanguage,
    /// No provider is configured: for the language, or at all (the project has no `[measure]` table).
    #[error("no provider is configured to find items")]
    NoProvider,
    /// The provider answered, and the answer is not a reply of the protocol or does not agree with the code.
    #[error("{0}")]
    Protocol(String),
    /// The provider could not be run to its end: not found, not started, a failed exit, a deadline passed.
    #[error("{0}")]
    Failed(String),
}

/// Consumer-owned port to find the Items of an Interface from its code. An implementation runs no more than the
/// provider it is configured with, changes no file of the project, and bounds the run by a deadline.
pub trait ItemSource: fmt::Debug + Send + Sync {
    /// The language of the Interface `interface` of `context`, as the project's configuration lays it out.
    fn language(&self, context: &str, interface: &str) -> Result<String, ItemsError>;

    /// The Items of `source`, the code of an Interface of `lang`, in the order the provider gave them.
    fn items<'a>(
        &'a self,
        lang: &'a str,
        source: &'a str,
    ) -> BoxFuture<'a, Result<Vec<FoundItem>, ItemsError>>;
}

/// The item source of an application that has no `[measure]` configured: it knows no language and no provider.
#[derive(Debug)]
pub struct NoItems;

impl ItemSource for NoItems {
    fn language(&self, _: &str, _: &str) -> Result<String, ItemsError> {
        Err(ItemsError::NoProvider)
    }

    fn items<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Vec<FoundItem>, ItemsError>> {
        Box::pin(async { Err(ItemsError::NoProvider) })
    }
}
