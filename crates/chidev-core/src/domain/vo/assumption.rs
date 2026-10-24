//! An Assumption: an interpretation a role settled itself, from the repository and general knowledge, with the
//! reason and the evidence. It is reproducible, so it asks nobody; a person who disagrees opens a new change
//! request.

use std::fmt;

use thiserror::Error;

use super::line_range::LineRange;
use super::prose::Prose;

/// Why a text is not evidence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvidenceError {
    /// A repository path or a reference is empty or only white space.
    #[error("the evidence is empty")]
    Empty,
    /// A repository path is absolute.
    #[error("the repository path `{0}` is absolute; it is relative to the repository")]
    Absolute(String),
    /// A repository path leaves the repository.
    #[error("the repository path `{0}` has `..`")]
    ParentSegment(String),
}

/// A path inside the repository: relative, with `/` between its parts, and never going up with `..`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoPath(String);

impl RepoPath {
    /// The path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for RepoPath {
    type Error = EvidenceError;

    fn try_from(text: &str) -> Result<RepoPath, EvidenceError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(EvidenceError::Empty);
        }
        let drive = text.as_bytes().get(1) == Some(&b':')
            && text.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
        if text.starts_with('/') || text.starts_with('\\') || drive {
            return Err(EvidenceError::Absolute(text.to_string()));
        }
        if text.split(['/', '\\']).any(|part| part == "..") {
            return Err(EvidenceError::ParentSegment(text.to_string()));
        }
        Ok(RepoPath(text.to_string()))
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where general knowledge is written down: a standard, a document, a section, in the words of the one who cites
/// it. Not blank.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KnowledgeRef(String);

impl KnowledgeRef {
    /// The reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for KnowledgeRef {
    type Error = EvidenceError;

    fn try_from(text: &str) -> Result<KnowledgeRef, EvidenceError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(EvidenceError::Empty);
        }
        Ok(KnowledgeRef(text.to_string()))
    }
}

impl fmt::Display for KnowledgeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What an Assumption stands on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// A place in the repository, and the lines of it when they matter.
    Repo {
        /// The file.
        path: RepoPath,
        /// The lines of it.
        lines: Option<LineRange>,
    },
    /// General knowledge (a best practice, a standard).
    Knowledge {
        /// Where it is written.
        reference: KnowledgeRef,
    },
}

/// An interpretation a role settled itself: the statement, why it holds, and the evidence. It records what was
/// assumed so that a person reading later can see it and, if they disagree, open a change request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assumption {
    statement: Prose,
    reason: Prose,
    evidence: Vec<Evidence>,
}

impl Assumption {
    /// An assumption.
    pub fn new(statement: Prose, reason: Prose, evidence: Vec<Evidence>) -> Assumption {
        Assumption {
            statement,
            reason,
            evidence,
        }
    }

    /// What is assumed.
    pub fn statement(&self) -> &Prose {
        &self.statement
    }

    /// Why it holds.
    pub fn reason(&self) -> &Prose {
        &self.reason
    }

    /// What it stands on.
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repo_path_is_relative_and_stays_in_the_repository() {
        assert_eq!(
            RepoPath::try_from(" src/lib.rs ").unwrap().as_str(),
            "src/lib.rs"
        );
        assert!(RepoPath::try_from("a/b..c/d").is_ok());
        assert_eq!(RepoPath::try_from(" "), Err(EvidenceError::Empty));
        for abs in ["/etc/passwd", "\\share\\x", "C:\\x", "c:/x"] {
            assert_eq!(
                RepoPath::try_from(abs),
                Err(EvidenceError::Absolute(abs.to_string()))
            );
        }
        for up in ["../x", "a/../b", "a\\..\\b", "a/.."] {
            assert_eq!(
                RepoPath::try_from(up),
                Err(EvidenceError::ParentSegment(up.to_string()))
            );
        }
    }

    #[test]
    fn a_knowledge_reference_is_not_blank() {
        assert_eq!(
            KnowledgeRef::try_from(" RFC 3339 ").unwrap().as_str(),
            "RFC 3339"
        );
        assert_eq!(KnowledgeRef::try_from(""), Err(EvidenceError::Empty));
    }
}
