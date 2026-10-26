//! The judged rules, read from `checks/judged/*.toml` in the project's root: one file per rule, in the form of the
//! Arch Review Rules (a question, when it is a finding, when it is not, how to fix it, the words to look for).
//! A directory that is not there holds no rule. The shape of a file:
//!
//! ```toml
//! id = "judged.vague-clause"        # starts with `judged.`
//! summary = "a clause too vague for a case to check"
//! subject = "clause"                # context, interface, item, clause, case, change_request or project
//! moments = ["edit", "check"]       # at which moments an agent is asked to judge it
//! level = "warning"                 # the default level; a policy may change it
//! question = "Can a case be written from this clause alone?"
//! true = "no condition or value a case could check"
//! false = "a definition, or a clause about what is not observable (say so)"
//! fix = "reword it with a condition and an observable result"
//! when = ["should", "appropriate"]  # optional literal words that hint where to look
//! ```
//!
//! The file is parsed here, at the edge; what makes a rule a rule (the prefix, the texts, no `mandatory`) is the
//! domain's ([`JudgedRule::new`]). A file that is not a rule stops the run, as a policy that cannot be read does:
//! a rule the project believed was in force must not be quietly missing.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::domain::check::{JudgedDraft, JudgedRule, Level, Moment, renamed_key};
use crate::domain::vo::finding::{RuleId, SubjectKind};

/// The directory of the rule files, relative to the project's root.
pub const RULES_DIR: &str = "checks/judged";

/// Why the rule files cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleFileError {
    /// The directory or a file exists and cannot be read.
    #[error("{}: {message}", path.display())]
    Read {
        /// The directory or file.
        path: PathBuf,
        /// What the system said.
        message: String,
    },
    /// The file is not a judged rule.
    #[error("{}: {message}", path.display())]
    Invalid {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
}

/// What a file says, before the domain judges it. Unknown keys are refused, so a misspelt `moments` is not a rule
/// that quietly belongs nowhere.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    id: String,
    summary: String,
    subject: String,
    /// Optional only so that the old key is told apart from a missing one.
    #[serde(default)]
    moments: Option<Vec<String>>,
    /// Only to be refused: the key was `hooks` before it was `moments`.
    #[serde(default)]
    hooks: Option<toml::Value>,
    level: String,
    question: String,
    #[serde(rename = "true")]
    true_when: String,
    #[serde(rename = "false")]
    false_when: String,
    fix: String,
    #[serde(default)]
    when: Vec<String>,
    /// Only to be refused: a judged rule cannot be mandatory.
    #[serde(default)]
    mandatory: bool,
}

/// The rules in the project rooted at `root`, in the order of their file names.
pub fn load(root: &Path) -> Result<Vec<JudgedRule>, RuleFileError> {
    let dir = root.join(RULES_DIR);
    let read_error = |path: &Path, e: std::io::Error| RuleFileError::Read {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(read_error(&dir, e)),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| read_error(&dir, e))?.path();
        if path.extension().is_some_and(|x| x == "toml") && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).map_err(|e| read_error(&path, e))?;
            parse(&text).map_err(|message| RuleFileError::Invalid { path, message })
        })
        .collect()
}

/// The rule `text` is.
pub fn parse(text: &str) -> Result<JudgedRule, String> {
    let file: RuleFile = toml::from_str(text).map_err(|e| e.to_string())?;
    let id = RuleId::try_from(file.id.as_str()).map_err(|e| e.to_string())?;
    let subject = SubjectKind::from_word(&file.subject).ok_or_else(|| {
        let kinds: Vec<&str> = SubjectKind::ALL.iter().map(|k| k.word()).collect();
        format!(
            "`subject` is `{}`; it is {}",
            file.subject,
            kinds.join(", ")
        )
    })?;
    if file.hooks.is_some() {
        return Err(renamed_key("hooks", "moments"));
    }
    let moments = file
        .moments
        .as_deref()
        .ok_or("missing field `moments`")?
        .iter()
        .map(|w| Moment::parse(w).map_err(|e| format!("`moments` has `{w}`: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let level = Level::from_word(&file.level).ok_or_else(|| {
        format!(
            "`level` is `{}`; it is off, note, warning or error",
            file.level
        )
    })?;
    JudgedRule::new(JudgedDraft {
        id,
        summary: file.summary,
        question: file.question,
        true_when: file.true_when,
        false_when: file.false_when,
        fix: file.fix,
        when: file.when,
        moments,
        level,
        subject,
        mandatory: file.mandatory,
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = r#"
        id = "judged.vague-clause"
        summary = "a clause too vague for a case to check"
        subject = "clause"
        moments = ["edit", "check"]
        level = "warning"
        question = "Can a case be written from this clause alone?"
        true = "no condition or value a case could check"
        false = "a definition"
        fix = "reword it"
        when = ["should"]
    "#;

    #[test]
    fn a_file_is_read_into_a_judged_rule() {
        let r = parse(RULE).unwrap();
        assert_eq!(r.id().as_str(), "judged.vague-clause");
        assert_eq!(r.subject(), SubjectKind::Clause);
        assert_eq!(r.true_when(), "no condition or value a case could check");
        assert_eq!(r.when(), ["should"]);
        assert_eq!(r.descriptor().default_level, Level::Warning);
        assert_eq!(&*r.descriptor().moments, [Moment::Edit, Moment::Check]);
    }

    #[test]
    fn what_is_not_a_rule_is_refused_with_why() {
        let without_fix = RULE
            .lines()
            .filter(|l| !l.trim_start().starts_with("fix"))
            .collect::<Vec<_>>()
            .join("\n");
        for (text, expect) in [
            (
                RULE.replace("judged.vague", "cite.vague"),
                "starts with `judged.`",
            ),
            (
                RULE.replace("\"clause\"", "\"contract\""),
                "`subject` is `contract`",
            ),
            (RULE.replace("\"edit\"", "\"save\""), "`moments` has `save`"),
            (RULE.replace("\"edit\"", "\"put\""), "`put` is now `edit`"),
            (
                RULE.replace(
                    "moments = [\"edit\", \"check\"]",
                    "hooks = [\"put\", \"check\"]",
                ),
                "`hooks` is now `moments`",
            ),
            (
                RULE.replace(
                    "moments = [\"edit\", \"check\"]",
                    "hooks = [\"edit\", \"check\"]",
                ),
                "`hooks` is now `moments`",
            ),
            (
                RULE.replace("moments = [\"edit\", \"check\"]\n", ""),
                "missing field `moments`",
            ),
            (RULE.replace("\"warning\"", "\"loud\""), "`level` is `loud`"),
            (
                RULE.replace("moments = [\"edit\", \"check\"]", "moments = []"),
                "belongs to no moment",
            ),
            (format!("{RULE}\nmandatory = true"), "cannot be mandatory"),
            (format!("{RULE}\nseverity = 3"), "unknown field"),
            (without_fix, "missing field `fix`"),
            (RULE.replace("reword it", " "), "empty `fix`"),
            ("not toml [".to_string(), "expected"),
        ] {
            let e = parse(&text).unwrap_err();
            assert!(e.contains(expect), "{expect:?} not in {e:?}");
        }
    }

    #[test]
    fn the_directory_is_read_in_file_order_and_a_bad_file_stops_it() {
        let dir = std::env::temp_dir().join(format!("chidev-judged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), Ok(Vec::new()));
        let rules = dir.join(RULES_DIR);
        std::fs::create_dir_all(&rules).unwrap();
        std::fs::write(rules.join("b.toml"), RULE.replace("vague-clause", "b")).unwrap();
        std::fs::write(rules.join("a.toml"), RULE.replace("vague-clause", "a")).unwrap();
        std::fs::write(rules.join("notes.md"), "not a rule").unwrap();
        let loaded = load(&dir).unwrap();
        let ids: Vec<&str> = loaded.iter().map(|r| r.id().as_str()).collect();
        assert_eq!(ids, ["judged.a", "judged.b"]);
        std::fs::write(rules.join("c.toml"), "id = 1").unwrap();
        assert!(matches!(load(&dir), Err(RuleFileError::Invalid { .. })));
    }
}
