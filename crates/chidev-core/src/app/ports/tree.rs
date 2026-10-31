//! The port of the working tree: what the use case hands over to have the copies of the placed assets looked at, and
//! what comes back. Which file holds the copy of an asset is the project's layout, and how it is read is the
//! adapter's; the use case names the asset and is told what was read of its copy as a value of the domain.

use std::fmt;

use thiserror::Error;

use super::measure::BoxFuture;
use crate::domain::vo::name::{AssetName, ContextName};
use crate::domain::vo::tree::CopyRead;

/// An asset whose copy in the working tree is asked about, by the names a person knows it by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TreeTarget {
    /// The code of an Interface.
    Interface {
        /// The Context's name.
        context: ContextName,
        /// The Interface's name.
        name: AssetName,
    },
    /// The source of a Harness.
    Harness {
        /// The Context's name.
        context: ContextName,
        /// The Harness's name.
        name: AssetName,
    },
}

/// Why a file cannot be read as the text of a copy. The path is the one the layout gives, relative to the project's
/// root.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TreeError {
    /// The file could not be read.
    #[error("{path}: {message}")]
    Io {
        /// The file.
        path: String,
        /// What the system said.
        message: String,
    },
    /// The file was read and its bytes are not UTF-8 text, which every text the model holds is.
    #[error("{path}: not UTF-8 text")]
    NotText {
        /// The file.
        path: String,
    },
}

/// Consumer-owned port to look at the copies of the placed assets in the working tree. An implementation reads files
/// and nothing else: it starts no program, changes no file, and keeps nothing between calls.
pub trait WorkingTree: fmt::Debug + Send + Sync {
    /// What was read of the copy of each of `targets`, each beside the target it is of (never by position). A target
    /// with no file laid out for it is [`CopyRead::Unplaced`]; one the answer leaves out is read as unplaced too.
    /// The files are read once for the whole call, so several targets that share a file cost one read.
    fn digests_of<'a>(
        &'a self,
        targets: &'a [TreeTarget],
    ) -> BoxFuture<'a, Vec<(TreeTarget, CopyRead)>>;
}

/// The working tree of an application that has no `[measure]` configured: no layout places anything, so every copy is
/// unplaced.
#[derive(Debug)]
pub struct NoTree;

impl WorkingTree for NoTree {
    fn digests_of<'a>(
        &'a self,
        targets: &'a [TreeTarget],
    ) -> BoxFuture<'a, Vec<(TreeTarget, CopyRead)>> {
        Box::pin(async move {
            targets
                .iter()
                .map(|t| (t.clone(), CopyRead::Unplaced))
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str) -> TreeTarget {
        TreeTarget::Interface {
            context: ContextName::try_from("core").unwrap(),
            name: AssetName::try_from(name).unwrap(),
        }
    }

    #[tokio::test]
    async fn an_application_with_no_layout_places_nothing() {
        let targets = [target("api"), target("lib")];
        let said = NoTree.digests_of(&targets).await;
        assert_eq!(
            said,
            [
                (target("api"), CopyRead::Unplaced),
                (target("lib"), CopyRead::Unplaced)
            ]
        );
        assert!(NoTree.digests_of(&[]).await.is_empty());
    }

    #[test]
    fn an_error_names_the_file_by_the_path_the_layout_gives() {
        let io = TreeError::Io {
            path: "src/a.rs".into(),
            message: "Is a directory (os error 21)".into(),
        };
        assert_eq!(io.to_string(), "src/a.rs: Is a directory (os error 21)");
        let text = TreeError::NotText {
            path: "src/a.rs".into(),
        };
        assert_eq!(text.to_string(), "src/a.rs: not UTF-8 text");
    }
}
