//! The port of the language providers that find the Items of an Interface: what the use case hands over (the
//! language and the code) and what comes back (a key and the lines of each declaration a caller can name, and how
//! many declarations were left out). How a provider is found, started and spoken to is the adapter's; what the use
//! case does with the answer (the signature of an Item, its id, its revision) is not.
//!
//! A **caller** of a source is code outside that source, built as a caller builds it (not as part of the source's
//! own tests). An Item is a declaration such a caller can name. The provider answers the Items, and in `left_out`
//! how many declarations it found that are not. (This is the protocol `chidev-items 2`; which declarations a caller
//! can name is the provider's to say, by the rules of its language.)

use std::fmt;

use thiserror::Error;

use super::measure::BoxFuture;
use crate::domain::vo::name::{AssetName, ContextName};

/// One declaration a provider found that a caller can name: its key and the first and last line of it, from 1. The
/// adapter has checked that the key is a key and that the lines are in the code; it has not checked that the
/// declarations are apart (one line may hold two).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundItem {
    /// The name the Item is matched by when the code is put again.
    pub key: String,
    /// The first and last line of the declaration.
    pub lines: (usize, usize),
}

/// What a provider found in the code of an Interface: the declarations a caller can name, and how many it found
/// that a caller cannot (so that leaving them out is never silent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundItems {
    /// The Items, in the order the provider gave them.
    pub items: Vec<FoundItem>,
    /// How many declarations the provider found that are not Items.
    pub left_out: usize,
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
    fn language(&self, context: &ContextName, interface: &AssetName) -> Result<String, ItemsError>;

    /// The Items of `source`, the code of an Interface of `lang`, in the order the provider gave them, and how many
    /// declarations were left out because a caller cannot name them.
    fn items<'a>(
        &'a self,
        lang: &'a str,
        source: &'a str,
    ) -> BoxFuture<'a, Result<FoundItems, ItemsError>>;
}

/// The item source of an application that has no `[measure]` configured: it knows no language and no provider.
#[derive(Debug)]
pub struct NoItems;

impl ItemSource for NoItems {
    fn language(&self, _: &ContextName, _: &AssetName) -> Result<String, ItemsError> {
        Err(ItemsError::NoProvider)
    }

    fn items<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<FoundItems, ItemsError>> {
        Box::pin(async { Err(ItemsError::NoProvider) })
    }
}
