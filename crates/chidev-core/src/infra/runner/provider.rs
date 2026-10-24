//! Asking a provider for mutants: `chidev-mutants 1`. One JSON request on the provider's standard input, one JSON
//! reply on its standard output, stderr for diagnostics; the provider is a program started without a shell. The
//! reply is a list of edits, and each is checked against the text chidev holds before any is applied, so a provider
//! that disagrees with the source is a protocol failure and not a silently wrong mutant.
//!
//! ```json
//! {"protocol":1,"path":"src/stock.rs","source":"...","max":40,"seed":0,"operators":["rel","str"]}
//! {"protocol":1,"sites":118,"mutants":[{"key":"reserve/rel/2/9f3a","line":42,"col":13,
//!   "operator":"rel","span":[1834,1835],"from":"<","to":"<="}]}
//! ```

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::super::process::{self, Over, Pipe};
use crate::domain::vo::measure::{
    FailureKind, MeasureFailure, Mutant, MutantKey, Operator, Site, StepEvidence,
};

/// The protocol version chidev speaks; a reply of another version is refused.
pub(super) const PROTOCOL: u32 = 1;

/// The largest reply taken: a list of edits, not files.
const REPLY_LIMIT: usize = 4 * 1024 * 1024;

/// How much of what the provider wrote on standard error is kept.
const STDERR_LIMIT: usize = 64 * 1024;

#[derive(Debug, Serialize)]
struct Request<'a> {
    protocol: u32,
    path: &'a str,
    source: &'a str,
    max: usize,
    seed: u64,
    /// Left out of the request when the settings name no classes: the provider then makes all it can.
    #[serde(skip_serializing_if = "Option::is_none")]
    operators: Option<&'a [String]>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    protocol: u32,
    sites: usize,
    mutants: Vec<Edit>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    key: String,
    line: u32,
    col: u32,
    operator: String,
    span: [usize; 2],
    from: String,
    to: String,
}

/// A mutant with the byte range its edit replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Planned {
    pub mutant: Mutant,
    pub span: (usize, usize),
}

/// What the provider answered: its mutants in order, and how many sites it saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Mutants {
    pub sites: u32,
    pub planned: Vec<Planned>,
}

fn failure(kind: FailureKind, message: impl Into<String>) -> MeasureFailure {
    MeasureFailure::of(kind, message)
}

/// The end of `text`, at most `n` bytes, from a character boundary, for a message.
pub(super) fn tail(text: &[u8], n: usize) -> String {
    let text = String::from_utf8_lossy(text);
    let start = text.len().saturating_sub(n);
    let start = (start..=text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    text[start..].trim().to_string()
}

/// Asks the provider `command` for the mutants of `source`, the text of `path`. The evidence of the call is given
/// whether or not it worked.
#[allow(clippy::too_many_arguments)]
pub(super) async fn ask(
    root: &Path,
    command: &[String],
    path: &str,
    source: &str,
    max: usize,
    seed: u64,
    operators: Option<&[String]>,
    deadline: tokio::time::Instant,
) -> (Result<Mutants, MeasureFailure>, StepEvidence) {
    let started = Instant::now();
    let mut evidence = StepEvidence {
        step: "provider".to_string(),
        command: command.join(" "),
        command_digest: None,
        exit_code: None,
        duration_ms: 0,
        stderr: String::new(),
    };
    let result = run(
        root,
        command,
        path,
        source,
        max,
        seed,
        operators,
        deadline,
        &mut evidence,
    )
    .await;
    evidence.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    (result, evidence)
}

#[allow(clippy::too_many_arguments)]
async fn run(
    root: &Path,
    command: &[String],
    path: &str,
    source: &str,
    max: usize,
    seed: u64,
    operators: Option<&[String]>,
    deadline: tokio::time::Instant,
    evidence: &mut StepEvidence,
) -> Result<Mutants, MeasureFailure> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| failure(FailureKind::Provider, "the provider command is empty"))?;
    let provider = |f: process::Failure| match f.kind {
        process::FailureKind::Timeout => {
            failure(FailureKind::Timeout, "the provider did not answer in time")
        }
        _ => failure(
            FailureKind::Provider,
            format!("{}: {}", f.kind.word(), f.message),
        ),
    };
    let executable = process::locate(root, program).await.map_err(provider)?;
    evidence.command = std::iter::once(executable.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    evidence.command_digest = Some(process::file_digest(&executable).await.map_err(provider)?);
    let input = serde_json::to_vec(&Request {
        protocol: PROTOCOL,
        path,
        source,
        max,
        seed,
        operators,
    })
    .map_err(|e| failure(FailureKind::Provider, e.to_string()))?;
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
    evidence.exit_code = capture.exit_code;
    evidence.stderr = tail(&capture.stderr, 4096);
    let status = status.map_err(provider)?;
    if !status.success() {
        return Err(failure(
            FailureKind::Provider,
            format!("the provider exited with {status}: {}", evidence.stderr),
        ));
    }
    let reply: Reply = serde_json::from_slice(&capture.stdout).map_err(|e| {
        failure(
            FailureKind::Protocol,
            format!("not a chidev-mutants reply: {e}"),
        )
    })?;
    plan(source, reply)
}

/// The edits of `reply`, each checked against `source`.
fn plan(source: &str, reply: Reply) -> Result<Mutants, MeasureFailure> {
    let protocol = |message: String| failure(FailureKind::Protocol, message);
    if reply.protocol != PROTOCOL {
        return Err(protocol(format!(
            "the reply is protocol {}; chidev speaks {PROTOCOL}",
            reply.protocol
        )));
    }
    let sites = u32::try_from(reply.sites)
        .map_err(|_| protocol(format!("{} sites is more than a run can hold", reply.sites)))?;
    let mut keys = BTreeSet::new();
    let mut planned = Vec::with_capacity(reply.mutants.len());
    for e in reply.mutants {
        let [start, end] = e.span;
        if source.get(start..end) != Some(e.from.as_str()) {
            return Err(protocol(format!(
                "mutant `{}`: the source at bytes {start}..{end} is not `{}`",
                e.key, e.from
            )));
        }
        let mutant = (|| {
            Mutant::new(
                MutantKey::try_from(e.key.as_str())?,
                Site::new(e.line, e.col)?,
                Operator::try_from(e.operator.as_str())?,
                e.from.clone(),
                e.to.clone(),
            )
        })()
        .map_err(|err| protocol(format!("mutant `{}`: {err}", e.key)))?;
        if !keys.insert(e.key.clone()) {
            return Err(protocol(format!(
                "the mutant key `{}` is given twice",
                e.key
            )));
        }
        planned.push(Planned {
            mutant,
            span: (start, end),
        });
    }
    if planned.len() > reply.sites {
        return Err(protocol(format!(
            "{} mutants from {} sites",
            planned.len(),
            reply.sites
        )));
    }
    Ok(Mutants { sites, planned })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn f(a: u32) -> bool { a <= 5 }\n";

    fn reply(json: &str) -> Result<Mutants, MeasureFailure> {
        plan(SOURCE, serde_json::from_str(json).expect("a reply"))
    }

    fn good() -> String {
        let start = SOURCE.find("<=").unwrap();
        format!(
            r#"{{"protocol":1,"sites":2,"mutants":[{{"key":"f/rel/1/0000","line":1,"col":{},"operator":"rel","span":[{start},{}],"from":"<=","to":"<"}}]}}"#,
            start + 1,
            start + 2
        )
    }

    #[test]
    fn a_reply_that_agrees_with_the_source_is_planned() {
        let planned = reply(&good()).unwrap();
        assert_eq!(planned.sites, 2);
        assert_eq!(planned.planned.len(), 1);
        let p = &planned.planned[0];
        assert_eq!(p.mutant.from(), "<=");
        assert_eq!(&SOURCE[p.span.0..p.span.1], "<=");
    }

    #[test]
    fn a_span_that_does_not_hold_its_from_is_a_protocol_failure() {
        let off = good().replace("\"from\":\"<=\"", "\"from\":\">=\"");
        let error = reply(&off).unwrap_err();
        assert_eq!(error.kind(), FailureKind::Protocol);
        assert!(error.message().contains("is not `>=`"), "{error}");
        // past the end, or not on a character boundary, is the same failure and not a panic
        let beyond = Reply {
            protocol: 1,
            sites: 1,
            mutants: vec![Edit {
                key: "k".into(),
                line: 1,
                col: 1,
                operator: "rel".into(),
                span: [400, 402],
                from: "<=".into(),
                to: "<".into(),
            }],
        };
        assert_eq!(
            plan(SOURCE, beyond).unwrap_err().kind(),
            FailureKind::Protocol
        );
        let inside_char = Reply {
            protocol: 1,
            sites: 1,
            mutants: vec![Edit {
                key: "k".into(),
                line: 1,
                col: 1,
                operator: "rel".into(),
                span: [1, 2],
                from: "x".into(),
                to: "y".into(),
            }],
        };
        assert!(plan("é", inside_char).is_err());
    }

    #[test]
    fn a_repeated_key_a_bad_value_or_another_protocol_is_refused() {
        let twice = good()
            .replace("}]}","},{\"key\":\"f/rel/1/0000\",\"line\":1,\"col\":2,\"operator\":\"rel\",\"span\":[0,2],\"from\":\"fn\",\"to\":\"gn\"}]}");
        assert!(reply(&twice).unwrap_err().message().contains("twice"));
        for (from, to) in [
            ("\"line\":1", "\"line\":0"),
            ("\"operator\":\"rel\"", "\"operator\":\"Rel\""),
            ("\"to\":\"<\"", "\"to\":\"<=\""),
            ("\"protocol\":1", "\"protocol\":2"),
            ("\"sites\":2", "\"sites\":0"),
        ] {
            let error = reply(&good().replace(from, to)).unwrap_err();
            assert_eq!(error.kind(), FailureKind::Protocol, "{to}");
        }
    }

    #[test]
    fn a_reply_with_a_field_the_protocol_does_not_have_is_not_a_reply() {
        assert!(
            serde_json::from_str::<Reply>(
                &good().replace("\"sites\":2", "\"sites\":2,\"extra\":1")
            )
            .is_err()
        );
    }

    #[test]
    fn the_tail_of_a_message_is_cut_on_a_character() {
        assert_eq!(tail("aé".repeat(10).as_bytes(), 5), "é\u{0061}é");
        assert_eq!(tail(b"  short \n", 100), "short");
    }
}
