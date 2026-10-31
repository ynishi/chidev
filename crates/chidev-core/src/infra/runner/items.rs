//! Asking a provider for the Items of an Interface: `chidev-items 2`. One JSON request on the provider's standard
//! input, one JSON reply on its standard output, stderr for diagnostics; the provider is a program started without a
//! shell, as the provider of mutants is.
//!
//! A **caller** of a source is code outside that source, built as a caller builds it (not as part of the source's
//! own tests). An Item is a declaration such a caller can name. The provider answers the Items, and in `left_out`
//! how many declarations it found that are not. `left_out` is required and written even when it is 0; a reply
//! without it, or of another protocol, is refused (chidev reads `chidev-items 2` only).
//!
//! The reply is a key and the lines of each Item, and each is checked against the code chidev holds, so a provider
//! that disagrees with it is a protocol failure and not an Item that points at nothing. Whether the declarations are
//! apart is not checked: one line may hold two.
//!
//! ```json
//! {"protocol":2,"source":"pub fn price(total: u64) -> u64;\n"}
//! {"protocol":2,"items":[{"key":"price","lines":[1,1]}],"left_out":0}
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::process::{self, Over, Pipe};
use super::provider::tail;
use crate::app::ports::items::{FoundItem, FoundItems, ItemsError};
use crate::domain::vo::item_key::ItemKey;
use crate::domain::vo::line_range::LineRange;

/// The protocol version chidev speaks; a reply of another version is refused.
pub(super) const PROTOCOL: u32 = 2;

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
    /// How many declarations the provider found that a caller cannot name; required, 0 included.
    left_out: usize,
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
) -> Result<FoundItems, ItemsError> {
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
    read(source, &capture.stdout)
}

/// The Items in what the provider wrote on its standard output, `reply`, checked against `source`.
fn read(source: &str, reply: &[u8]) -> Result<FoundItems, ItemsError> {
    let reply: Reply = serde_json::from_slice(reply).map_err(|e| {
        protocol(format!(
            "not a chidev-items reply (chidev reads chidev-items {PROTOCOL}): {e}"
        ))
    })?;
    plan(source, reply)
}

/// The Items of `reply`, each checked against `source`: a key is a key, and the lines are lines of the code.
fn plan(source: &str, reply: Reply) -> Result<FoundItems, ItemsError> {
    if reply.protocol != PROTOCOL {
        return Err(protocol(format!(
            "the reply is protocol {}; chidev reads chidev-items {PROTOCOL}",
            reply.protocol
        )));
    }
    let total = source.lines().count();
    let left_out = reply.left_out;
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
    Ok(FoundItems {
        items: found,
        left_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn a();\nfn b(\n    x: u8,\n);\n";

    fn reply(json: &str) -> Result<FoundItems, ItemsError> {
        read(SOURCE, json.as_bytes())
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
            r#"{"protocol":2,"items":[{"key":"b","lines":[2,4]},{"key":"a","lines":[1,1]}],"left_out":3}"#,
        )
        .unwrap();
        assert_eq!(
            found,
            FoundItems {
                items: vec![
                    FoundItem {
                        key: "b".into(),
                        lines: (2, 4)
                    },
                    FoundItem {
                        key: "a".into(),
                        lines: (1, 1)
                    },
                ],
                left_out: 3,
            }
        );
        assert_eq!(
            reply(r#"{"protocol":2,"items":[],"left_out":0}"#).unwrap(),
            FoundItems {
                items: Vec::new(),
                left_out: 0
            }
        );
    }

    #[test]
    fn lines_that_overlap_are_taken_as_they_are() {
        // `pub trait Clock { fn now(&self) -> u64; }`: two declarations on one line is a right answer
        let found = reply(
            r#"{"protocol":2,"items":[{"key":"Clock","lines":[1,1]},{"key":"Clock::now","lines":[1,1]}],"left_out":0}"#,
        )
        .unwrap();
        assert_eq!(found.items.len(), 2);
    }

    #[test]
    fn lines_outside_the_code_an_empty_key_or_another_protocol_are_refused() {
        for (json, expected) in [
            (
                r#"{"protocol":2,"items":[{"key":"b","lines":[4,5]}],"left_out":0}"#,
                "past the last",
            ),
            (
                r#"{"protocol":2,"items":[{"key":"b","lines":[0,1]}],"left_out":0}"#,
                "from 1",
            ),
            (
                r#"{"protocol":2,"items":[{"key":"b","lines":[3,2]}],"left_out":0}"#,
                "ends before",
            ),
            (
                r#"{"protocol":2,"items":[{"key":"","lines":[1,1]}],"left_out":0}"#,
                "is empty",
            ),
            (
                r#"{"protocol":2,"items":[{"key":"a\nb","lines":[1,1]}],"left_out":0}"#,
                "line break",
            ),
            (
                r#"{"protocol":1,"items":[],"left_out":0}"#,
                "protocol 1; chidev reads chidev-items 2",
            ),
            (
                r#"{"protocol":3,"items":[],"left_out":0}"#,
                "protocol 3; chidev reads chidev-items 2",
            ),
        ] {
            let said = message(reply(json).unwrap_err());
            assert!(said.contains(expected), "{said} vs {expected}");
        }
    }

    #[test]
    fn a_reply_without_left_out_is_refused_and_says_what_chidev_reads() {
        for json in [
            // protocol 2, but no `left_out`
            r#"{"protocol":2,"items":[]}"#,
            r#"{"protocol":2,"items":[{"key":"a","lines":[1,1]}]}"#,
            // what chidev-items 1 answered
            r#"{"protocol":1,"items":[{"key":"a","lines":[1,1]}]}"#,
        ] {
            let said = message(reply(json).unwrap_err());
            assert!(
                said.contains("not a chidev-items reply")
                    && said.contains("chidev reads chidev-items 2"),
                "{json}: {said}"
            );
        }
    }

    #[test]
    fn a_reply_with_a_field_the_protocol_does_not_have_is_not_a_reply() {
        for json in [
            r#"{"protocol":2,"items":[{"key":"a","lines":[1,1],"kind":"fn"}],"left_out":0}"#,
            r#"{"protocol":2,"items":[],"left_out":0,"extra":1}"#,
            r#"{"protocol":2,"items":[{"key":"a","lines":[1]}],"left_out":0}"#,
            r#"{"protocol":2}"#,
            r#"{"protocol":2,"items":[],"left_out":-1}"#,
            r#"{"protocol":2,"items":[],"left_out":"1"}"#,
        ] {
            assert!(serde_json::from_str::<Reply>(json).is_err(), "{json}");
            assert!(
                message(reply(json).unwrap_err()).contains("not a chidev-items reply"),
                "{json}"
            );
        }
    }
}
