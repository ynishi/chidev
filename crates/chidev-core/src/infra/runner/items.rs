//! Asking a provider for the Items of an Interface: `chidev-items 1`. One JSON request on the provider's standard
//! input, one JSON reply on its standard output, stderr for diagnostics; the provider is a program started without a
//! shell, as the provider of mutants is. The reply is a key and the lines of each declaration, and each is checked
//! against the code chidev holds, so a provider that disagrees with it is a protocol failure and not an Item that
//! points at nothing. Whether the declarations are apart is not checked: one line may hold two.
//!
//! ```json
//! {"protocol":1,"source":"fn price(total: u64) -> u64;\n"}
//! {"protocol":1,"items":[{"key":"price","lines":[1,1]}]}
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::process::{self, Over, Pipe};
use super::provider::tail;
use crate::app::ports::items::{FoundItem, ItemsError};
use crate::domain::vo::item_key::ItemKey;
use crate::domain::vo::line_range::LineRange;

/// The protocol version chidev speaks; a reply of another version is refused.
pub(super) const PROTOCOL: u32 = 1;

/// The largest reply taken: a list of keys and lines, not files.
const REPLY_LIMIT: usize = 4 * 1024 * 1024;

/// How much of what the provider wrote on standard error is kept.
const STDERR_LIMIT: usize = 64 * 1024;

#[derive(Debug, Serialize)]
struct Request<'a> {
    protocol: u32,
    source: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    protocol: u32,
    items: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    key: String,
    lines: [usize; 2],
}

fn protocol(message: impl Into<String>) -> ItemsError {
    ItemsError::Protocol(message.into())
}

fn failed(message: impl Into<String>) -> ItemsError {
    ItemsError::Failed(message.into())
}

/// Asks the provider `command` for the Items of `source`, the code of an Interface.
pub(super) async fn ask(
    root: &Path,
    command: &[String],
    source: &str,
    deadline: tokio::time::Instant,
) -> Result<Vec<FoundItem>, ItemsError> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| failed("the provider command is empty"))?;
    let from_process = |f: process::Failure| match f.kind {
        process::FailureKind::Timeout => failed("the provider did not answer in time"),
        _ => failed(format!("{}: {}", f.kind.word(), f.message)),
    };
    let executable = process::locate(root, program).await.map_err(from_process)?;
    let input = serde_json::to_vec(&Request {
        protocol: PROTOCOL,
        source,
    })
    .map_err(|e| failed(e.to_string()))?;
    let mut capture = process::Capture::default();
    let status = process::run(
        &process::Spec {
            executable: &executable,
            args,
            cwd: root,
            env: &[],
            stdin: Some(&input),
            deadline,
            stdout: Pipe {
                limit: REPLY_LIMIT,
                over: Over::Fail,
            },
            stderr: Pipe {
                limit: STDERR_LIMIT,
                over: Over::Truncate,
            },
        },
        &mut capture,
    )
    .await;
    let status = status.map_err(from_process)?;
    if !status.success() {
        return Err(failed(format!(
            "the provider exited with {status}: {}",
            tail(&capture.stderr, 4096)
        )));
    }
    let reply: Reply = serde_json::from_slice(&capture.stdout)
        .map_err(|e| protocol(format!("not a chidev-items reply: {e}")))?;
    plan(source, reply)
}

/// The Items of `reply`, each checked against `source`: a key is a key, and the lines are lines of the code.
fn plan(source: &str, reply: Reply) -> Result<Vec<FoundItem>, ItemsError> {
    if reply.protocol != PROTOCOL {
        return Err(protocol(format!(
            "the reply is protocol {}; chidev speaks {PROTOCOL}",
            reply.protocol
        )));
    }
    let total = source.lines().count();
    let mut found = Vec::with_capacity(reply.items.len());
    for e in reply.items {
        let [first, last] = e.lines;
        ItemKey::try_from(e.key.as_str())
            .map_err(|err| protocol(format!("item `{}`: {err}", e.key)))?;
        LineRange::new(first, last).map_err(|err| protocol(format!("item `{}`: {err}", e.key)))?;
        if last > total {
            return Err(protocol(format!(
                "item `{}`: lines {first}..={last} are past the last of the {total} lines of the code",
                e.key
            )));
        }
        found.push(FoundItem {
            key: e.key,
            lines: (first, last),
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn a();\nfn b(\n    x: u8,\n);\n";

    fn reply(json: &str) -> Result<Vec<FoundItem>, ItemsError> {
        plan(SOURCE, serde_json::from_str(json).expect("a reply"))
    }

    fn message(error: ItemsError) -> String {
        match error {
            ItemsError::Protocol(m) => m,
            other => panic!("not a protocol failure: {other:?}"),
        }
    }

    #[test]
    fn a_reply_that_agrees_with_the_code_is_taken_in_order() {
        let found = reply(
            r#"{"protocol":1,"items":[{"key":"b","lines":[2,4]},{"key":"a","lines":[1,1]}]}"#,
        )
        .unwrap();
        assert_eq!(
            found,
            [
                FoundItem {
                    key: "b".into(),
                    lines: (2, 4)
                },
                FoundItem {
                    key: "a".into(),
                    lines: (1, 1)
                },
            ]
        );
        assert_eq!(reply(r#"{"protocol":1,"items":[]}"#).unwrap(), []);
    }

    #[test]
    fn lines_that_overlap_are_taken_as_they_are() {
        // `pub trait Clock { fn now(&self) -> u64; }`: two declarations on one line is a right answer
        let found = reply(
            r#"{"protocol":1,"items":[{"key":"Clock","lines":[1,1]},{"key":"Clock::now","lines":[1,1]}]}"#,
        )
        .unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn lines_outside_the_code_an_empty_key_or_another_protocol_are_refused() {
        for (json, expected) in [
            (
                r#"{"protocol":1,"items":[{"key":"b","lines":[4,5]}]}"#,
                "past the last",
            ),
            (
                r#"{"protocol":1,"items":[{"key":"b","lines":[0,1]}]}"#,
                "from 1",
            ),
            (
                r#"{"protocol":1,"items":[{"key":"b","lines":[3,2]}]}"#,
                "ends before",
            ),
            (
                r#"{"protocol":1,"items":[{"key":"","lines":[1,1]}]}"#,
                "is empty",
            ),
            (
                r#"{"protocol":1,"items":[{"key":"a\nb","lines":[1,1]}]}"#,
                "line break",
            ),
            (r#"{"protocol":2,"items":[]}"#, "protocol 2"),
        ] {
            let said = message(reply(json).unwrap_err());
            assert!(said.contains(expected), "{said} vs {expected}");
        }
    }

    #[test]
    fn a_reply_with_a_field_the_protocol_does_not_have_is_not_a_reply() {
        for json in [
            r#"{"protocol":1,"items":[{"key":"a","lines":[1,1],"kind":"fn"}]}"#,
            r#"{"protocol":1,"items":[],"extra":1}"#,
            r#"{"protocol":1,"items":[{"key":"a","lines":[1]}]}"#,
            r#"{"protocol":1}"#,
        ] {
            assert!(serde_json::from_str::<Reply>(json).is_err(), "{json}");
        }
    }
}
