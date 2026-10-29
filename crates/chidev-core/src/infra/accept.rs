//! The runner of acceptance examples, `chidev accept`.
//!
//! An example is a name `<name>` for which a directory holds `<name>.args`, `<name>.stdin`, `<name>.expected` and,
//! optionally, `<name>.stderr`; [`crate::domain::vo::example`] says what each file holds and how an answer is
//! judged. This module reads the directory, runs the program once per example, and gives the verdicts in two faces:
//! lines for a person ([`lines`]) and the reply of an external check ([`reply`]).
//!
//! The program runs through [`crate::infra::process`]: without a shell, in a process group of its own that is killed
//! when the run ends (so a descendant is not left running), under a deadline, with a cap on what it may write to each
//! pipe. A program that gave no answer (it could not start, it did not end in time, it wrote too much, a signal ended
//! it) is a failure of the example with one reason, never an answer to compare.
//!
//! This is the program on the far side of an external check (README, "External checks"). It reads the request with
//! types of its own, which hold only what it needs and ignore the rest, so a field added to the request does not
//! reach it (an anti-corruption layer); and it answers with the wire types of [`crate::app::ports::external`].

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use crate::app::ports::external::{
    ExternalFinding, ExternalReply, ExternalSubject, REQUEST_PROTOCOL,
};
use crate::domain::vo::example::{
    self, Answer, ExampleError, ExampleName, Expected, Verdict, Why, judge,
};
use crate::infra::process::{self, Capture, FailureKind, Over, Pipe, Spec};

/// The most a program may write to each of its output pipes, in MiB; more is a failure of the example.
const LIMIT_MIB: usize = 4;

/// [`LIMIT_MIB`] in bytes.
const LIMIT: usize = LIMIT_MIB * 1024 * 1024;

/// The files of an example, each told by its extension: the one place the extensions are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    /// `<name>.args`, the arguments.
    Args,
    /// `<name>.stdin`, the standard input.
    Stdin,
    /// `<name>.expected`, the expected standard output and the `#exit` line.
    Expected,
    /// `<name>.stderr`, the expected standard error (optional).
    Stderr,
}

impl FileKind {
    /// Every file of an example.
    const ALL: [FileKind; 4] = [
        FileKind::Args,
        FileKind::Stdin,
        FileKind::Expected,
        FileKind::Stderr,
    ];

    /// The extension of the file.
    fn ext(self) -> &'static str {
        match self {
            FileKind::Args => "args",
            FileKind::Stdin => "stdin",
            FileKind::Expected => "expected",
            FileKind::Stderr => "stderr",
        }
    }

    /// The kind an extension tells; `None` for any other.
    fn of_ext(ext: &str) -> Option<FileKind> {
        Self::ALL.into_iter().find(|kind| kind.ext() == ext)
    }

    /// The file name of this file of the example `name`.
    fn file(self, name: &ExampleName) -> String {
        format!("{name}.{}", self.ext())
    }

    /// The extensions, as a list for a message: `.args, .stdin, .expected or .stderr`.
    fn listed() -> String {
        let exts: Vec<String> = Self::ALL.iter().map(|k| format!(".{}", k.ext())).collect();
        match exts.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            _ => exts.join(""),
        }
    }
}

/// Why the examples could not be run at all, or the request of an external check could not be answered: the
/// failures of usage, as opposed to a failure of one example, which is its [`Verdict`].
#[derive(Debug, Error)]
pub enum AcceptError {
    /// The directory of the examples cannot be read.
    #[error("cannot read the directory {dir}: {message}")]
    Directory {
        /// The directory as given.
        dir: String,
        /// The error of the system.
        message: String,
    },
    /// The directory holds no example.
    #[error(
        "no example in {0}: an example is a name with a {kinds} file",
        kinds = FileKind::listed()
    )]
    NoExamples(String),
    /// The file name of an example is not UTF-8, so it cannot be named in a report.
    #[error("the file name {0:?} is not UTF-8")]
    NotUtf8(String),
    /// The name part of a file name is not the name of an example.
    #[error("{file}: {source}")]
    BadName {
        /// The file.
        file: String,
        /// What is wrong with its name.
        source: ExampleError,
    },
    /// The request is not a JSON document of the shape of an external check's request.
    #[error("the request is not an external-check request: {0}")]
    Request(String),
    /// The request is of a protocol this runner does not read.
    #[error("the request is of protocol {got}, not {want}")]
    Protocol {
        /// The protocol the request says.
        got: u64,
        /// The protocol this runner reads.
        want: u32,
    },
    /// The request asks for a number of rules other than one.
    #[error("the request has {0} rules, not exactly one")]
    Rules(usize),
}

/// One example as its files give it. A file that is missing, cannot be read or is malformed does not stop the
/// others: the example carries the reasons and fails when it is run.
#[derive(Debug)]
pub(crate) struct Example {
    name: ExampleName,
    args: Vec<String>,
    stdin: Vec<u8>,
    expected: Result<Expected, Vec<Why>>,
}

/// The result of reading one file of an example.
enum Part {
    Found(Vec<u8>),
    Missing,
    Broken(String),
}

async fn part(dir: &Path, name: &ExampleName, kind: FileKind) -> Part {
    let file = kind.file(name);
    match tokio::fs::read(dir.join(&file)).await {
        Ok(bytes) => Part::Found(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Part::Missing,
        // never taken for an absent file: a permission or an I/O error is told, with the system's words
        Err(e) => Part::Broken(format!("cannot read {file}: {e}")),
    }
}

async fn read_example(dir: &Path, name: ExampleName) -> Example {
    let args = part(dir, &name, FileKind::Args).await;
    let stdin = part(dir, &name, FileKind::Stdin).await;
    let expected = part(dir, &name, FileKind::Expected).await;
    let errors = part(dir, &name, FileKind::Stderr).await;
    let mut missing = Vec::new();
    let mut reasons = Vec::new();

    let args = match args {
        Part::Found(bytes) => match String::from_utf8(bytes) {
            Ok(text) => example::args(&text),
            Err(_) => {
                reasons.push(Why::Unreadable(format!(
                    "{} is not UTF-8",
                    FileKind::Args.file(&name)
                )));
                Vec::new()
            }
        },
        Part::Missing => {
            missing.push(FileKind::Args.file(&name));
            Vec::new()
        }
        Part::Broken(message) => {
            reasons.push(Why::Unreadable(message));
            Vec::new()
        }
    };
    let stdin = match stdin {
        Part::Found(bytes) => bytes,
        Part::Missing => {
            missing.push(FileKind::Stdin.file(&name));
            Vec::new()
        }
        Part::Broken(message) => {
            reasons.push(Why::Unreadable(message));
            Vec::new()
        }
    };
    let errors = match errors {
        Part::Found(bytes) => Some(bytes),
        // the one optional file: absent, standard error is not compared
        Part::Missing => None,
        Part::Broken(message) => {
            reasons.push(Why::Unreadable(message));
            None
        }
    };
    let expected = match expected {
        Part::Found(bytes) => match Expected::parse(&bytes, errors) {
            Ok(expected) => Some(expected),
            Err(e) => {
                reasons.push(Why::Unreadable(format!(
                    "{} {e}",
                    FileKind::Expected.file(&name)
                )));
                None
            }
        },
        Part::Missing => {
            missing.push(FileKind::Expected.file(&name));
            None
        }
        Part::Broken(message) => {
            reasons.push(Why::Unreadable(message));
            None
        }
    };
    if !missing.is_empty() {
        reasons.insert(
            0,
            Why::Unreadable(format!("missing {}", missing.join(", "))),
        );
    }
    let expected = match expected {
        Some(expected) if reasons.is_empty() => Ok(expected),
        _ => Err(reasons),
    };
    Example {
        name,
        args,
        stdin,
        expected,
    }
}

fn directory(dir: &Path, e: std::io::Error) -> AcceptError {
    AcceptError::Directory {
        dir: dir.display().to_string(),
        message: e.to_string(),
    }
}

/// The examples of `dir`, in the byte order of their names. A regular file `<name>.args`, `<name>.stdin`,
/// `<name>.expected` or `<name>.stderr` makes `<name>` an example; anything else in the directory is ignored. A
/// directory that cannot be read, or that holds no example, is an error.
pub(crate) async fn read_dir(dir: &Path) -> Result<Vec<Example>, AcceptError> {
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| directory(dir, e))?;
    let mut names = BTreeSet::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| directory(dir, e))? {
        let file = entry.file_name();
        let path = Path::new(&file);
        if !path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(FileKind::of_ext)
            .is_some()
        {
            continue;
        }
        // a link is followed, as reading it would; a link to nothing is not a file
        match tokio::fs::metadata(entry.path()).await {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(directory(dir, e)),
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            return Err(AcceptError::NotUtf8(file.to_string_lossy().into_owned()));
        };
        let name = ExampleName::new(stem).map_err(|source| AcceptError::BadName {
            file: file.to_string_lossy().into_owned(),
            source,
        })?;
        names.insert(name);
    }
    if names.is_empty() {
        return Err(AcceptError::NoExamples(dir.display().to_string()));
    }
    let mut examples = Vec::with_capacity(names.len());
    for name in names {
        examples.push(read_example(dir, name).await);
    }
    Ok(examples)
}

/// The reason a run that did not end gives, in the words of a report.
fn abnormal(failure: &process::Failure, timeout: Duration) -> Why {
    Why::Abnormal(match failure.kind {
        FailureKind::Spawn => format!("cannot start: {}", failure.message),
        FailureKind::Timeout => format!("did not end within {} s", timeout.as_secs()),
        FailureKind::Limit => {
            // the message starts with the name of the pipe
            let pipe = failure
                .message
                .split_whitespace()
                .next()
                .unwrap_or("a pipe");
            format!("output over {LIMIT_MIB} MiB ({pipe})")
        }
        FailureKind::Transport | FailureKind::Unsupported => failure.message.clone(),
    })
}

/// Runs the program `command` (its first word, then its arguments) for `example`, from `cwd`, and judges what it
/// answered. The arguments of the example follow those of `command`; its standard input is the example's; nothing is
/// added to the environment. An example that could not be read fails without running anything.
pub(crate) async fn run(
    example: &Example,
    command: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Verdict {
    let expected = match &example.expected {
        Ok(expected) => expected,
        Err(reasons) => return Verdict::Fail(reasons.clone()),
    };
    let Some((program, fixed)) = command.split_first() else {
        return Verdict::fail(Why::Abnormal("cannot start: no command".to_string()));
    };
    // `locate`, not `resolve`: a program that is a link to another picks what to run by the name it is started as
    let executable = match process::locate(cwd, program).await {
        Ok(executable) => executable,
        Err(failure) => return Verdict::fail(abnormal(&failure, timeout)),
    };
    let deadline = match process::deadline_in(timeout) {
        Ok(deadline) => deadline,
        Err(failure) => return Verdict::fail(abnormal(&failure, timeout)),
    };
    let args: Vec<String> = fixed.iter().chain(&example.args).cloned().collect();
    let pipe = Pipe {
        limit: LIMIT,
        over: Over::Fail,
    };
    let spec = Spec {
        executable: &executable,
        args: &args,
        cwd,
        env: &[],
        stdin: Some(example.stdin.as_slice()),
        deadline,
        stdout: pipe,
        stderr: pipe,
    };
    let mut capture = Capture::default();
    let status = match process::run(&spec, &mut capture).await {
        Ok(status) => status,
        Err(failure) => return Verdict::fail(abnormal(&failure, timeout)),
    };
    let Some(exit) = status.code() else {
        return Verdict::fail(Why::Abnormal("ended by a signal".to_string()));
    };
    judge(
        expected,
        &Answer {
            output: capture.stdout,
            errors: capture.stderr,
            exit,
        },
    )
}

/// Runs every example of `dir` against `command`, one after another, and gives the verdicts in the order of the
/// names.
pub async fn run_all(
    dir: &Path,
    command: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<Vec<(ExampleName, Verdict)>, AcceptError> {
    let examples = read_dir(dir).await?;
    let mut results = Vec::with_capacity(examples.len());
    for example in &examples {
        let verdict = run(example, command, cwd, timeout).await;
        results.push((example.name.clone(), verdict));
    }
    Ok(results)
}

/// The report for a person: a line `ok <name>` or `fail <name>: <reasons>` for each example, then
/// `<p> passed, <f> failed`. Every line ends with a line feed.
pub fn lines(results: &[(ExampleName, Verdict)]) -> String {
    let mut text = String::new();
    for (name, verdict) in results {
        match verdict {
            Verdict::Ok => text.push_str(&format!("ok {name}\n")),
            Verdict::Fail(_) => text.push_str(&format!("fail {name}: {verdict}\n")),
        }
    }
    let passed = results
        .iter()
        .filter(|(_, verdict)| verdict.is_ok())
        .count();
    text.push_str(&format!(
        "{passed} passed, {} failed\n",
        results.len() - passed
    ));
    text
}

/// What the runner reads of the request of an external check; the rest is ignored.
#[derive(Debug, Deserialize)]
struct Request {
    protocol: u64,
    #[serde(default)]
    rules: Vec<String>,
    #[serde(default)]
    model: Model,
}

#[derive(Debug, Default, Deserialize)]
struct Model {
    #[serde(default)]
    contexts: Vec<Context>,
}

#[derive(Debug, Deserialize)]
struct Context {
    #[serde(default)]
    contracts: Vec<Contract>,
}

#[derive(Debug, Deserialize)]
struct Contract {
    #[serde(default)]
    clauses: Vec<Clause>,
}

#[derive(Debug, Deserialize)]
struct Clause {
    id: String,
    text: String,
}

/// `dir` as it is written in a path of an example: without a leading `./` (repeated) and a trailing `/`, and nothing
/// at all for the current directory.
fn prefix(dir: &str) -> &str {
    let mut dir = dir;
    while let Some(rest) = dir.strip_prefix("./") {
        dir = rest;
    }
    let dir = dir.trim_end_matches('/');
    if dir == "." { "" } else { dir }
}

/// The path of the file `kind` of the example `name` in the directory written `prefix`.
fn path_of(prefix: &str, name: &ExampleName, kind: FileKind) -> String {
    if prefix.is_empty() {
        kind.file(name)
    } else {
        format!("{prefix}/{}", kind.file(name))
    }
}

/// Whether `text` names the file `path`: `path` occurs in it as a whole, that is with no character before it that
/// would make it the tail of a longer path or name (alphanumeric, `_`, `-`, `.`, `/`) and none after it that would
/// make it the head of a longer name (alphanumeric, `_`, `-`). A `.` after it is the end of a sentence.
fn names_file(text: &str, path: &str) -> bool {
    let mut from = 0;
    while let Some(found) = text[from..].find(path) {
        let start = from + found;
        let end = start + path.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')));
        let after_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-')));
        if before_ok && after_ok {
            return true;
        }
        // an occurrence may begin inside the one just seen
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// The request of an external check as the runner needs it: the one rule it is asked for and the Clauses of the
/// model it was sent. It is read before any example runs, so a request that cannot be answered runs nothing.
#[derive(Debug)]
pub struct Call {
    rule: String,
    clauses: Vec<Clause>,
}

impl Call {
    /// Reads `request`, the document the check was sent. A request that is not JSON, is not protocol 2 or does not
    /// ask for exactly one rule is an error.
    pub fn read(request: &[u8]) -> Result<Call, AcceptError> {
        let request: Request =
            serde_json::from_slice(request).map_err(|e| AcceptError::Request(e.to_string()))?;
        if request.protocol != u64::from(REQUEST_PROTOCOL) {
            return Err(AcceptError::Protocol {
                got: request.protocol,
                want: REQUEST_PROTOCOL,
            });
        }
        let [rule] = request.rules.as_slice() else {
            return Err(AcceptError::Rules(request.rules.len()));
        };
        let rule = rule.clone();
        let clauses = request
            .model
            .contexts
            .into_iter()
            .flat_map(|context| context.contracts)
            .flat_map(|contract| contract.clauses)
            .collect();
        Ok(Call { rule, clauses })
    }

    /// The reply for `results`, the verdicts of the examples of the directory written `dir`.
    ///
    /// A failing example gives a finding for each Clause of the model whose text names a file of the example by its
    /// path, `<dir>/<name>.<ext>`, so a finding is about the Clause that states the example. When no Clause names it,
    /// the finding is about the project and says so. A passing example gives nothing.
    pub fn reply(&self, dir: &str, results: &[(ExampleName, Verdict)]) -> ExternalReply {
        let rule = &self.rule;
        let clauses = &self.clauses;
        let prefix = prefix(dir);
        let mut findings = Vec::new();
        for (name, verdict) in results {
            if verdict.is_ok() {
                continue;
            }
            let message = format!("{name}: {verdict}");
            let paths = FileKind::ALL.map(|kind| path_of(prefix, name, kind));
            let named: Vec<&Clause> = clauses
                .iter()
                .filter(|clause| paths.iter().any(|path| names_file(&clause.text, path)))
                .collect();
            if named.is_empty() {
                findings.push(ExternalFinding {
                    rule: rule.clone(),
                    subject: None,
                    message: format!(
                        "{message} (no clause names {})",
                        path_of(prefix, name, FileKind::Expected)
                    ),
                    level: None,
                });
            }
            for clause in named {
                findings.push(ExternalFinding {
                    rule: rule.clone(),
                    subject: Some(ExternalSubject {
                        kind: "clause".to_string(),
                        id: clause.id.clone(),
                    }),
                    message: message.clone(),
                    level: None,
                });
            }
        }
        ExternalReply {
            protocol: REQUEST_PROTOCOL,
            findings,
        }
    }
}

/// [`Call::read`] and [`Call::reply`] in one, for the tests of both.
#[cfg(test)]
fn reply(
    request: &[u8],
    dir: &str,
    results: &[(ExampleName, Verdict)],
) -> Result<ExternalReply, AcceptError> {
    Ok(Call::read(request)?.reply(dir, results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn name(name: &str) -> ExampleName {
        ExampleName::new(name).unwrap()
    }

    fn failing(name_: &str) -> (ExampleName, Verdict) {
        (
            name(name_),
            Verdict::fail(Why::Exit {
                expected: 0,
                got: 3,
            }),
        )
    }

    fn passing(name_: &str) -> (ExampleName, Verdict) {
        (name(name_), Verdict::Ok)
    }

    #[test]
    fn a_path_is_named_as_a_whole() {
        let path = "acceptance/a1.expected";
        assert!(names_file("`acceptance/a1.expected` gives", path));
        assert!(names_file(path, path));
        assert!(names_file("the output is in acceptance/a1.expected.", path));
        assert!(names_file("(acceptance/a1.expected)", path));
        assert!(!names_file("`acceptance/a10.expected` gives", path));
        assert!(!names_file("`xacceptance/a1.expected` gives", path));
        assert!(!names_file("`./acceptance/a1.expected` gives", path));
        assert!(!names_file("`/acceptance/a1.expected` gives", path));
        assert!(!names_file("`my-acceptance/a1.expected` gives", path));
        assert!(!names_file("`acceptance/a1.expected2` gives", path));
        assert!(!names_file("`acceptance/a1.expected_x` gives", path));
        assert!(!names_file("`acceptance/a1.expected-x` gives", path));
        assert!(!names_file("`acceptance/a1.expectedé` gives", path));
        assert!(!names_file("", path));
    }

    #[test]
    fn every_occurrence_is_tried() {
        let path = "ex/a1.expected";
        assert!(names_file("xex/a1.expected and then ex/a1.expected", path));
        assert!(names_file("ex/a1.expected2, ex/a1.expected;", path));
        assert!(!names_file("xex/a1.expected and ex/a1.expected2", path));
    }

    #[test]
    fn the_directory_is_written_without_a_dot_slash_or_a_trailing_slash() {
        assert_eq!(prefix("acceptance"), "acceptance");
        assert_eq!(prefix("./acceptance/"), "acceptance");
        assert_eq!(prefix("././acceptance//"), "acceptance");
        assert_eq!(prefix("a/b"), "a/b");
        assert_eq!(prefix("."), "");
        assert_eq!(prefix("./"), "");
        assert_eq!(
            path_of(prefix("./acceptance/"), &name("a1"), FileKind::Stdin),
            "acceptance/a1.stdin"
        );
        assert_eq!(
            path_of(prefix("."), &name("a1"), FileKind::Stdin),
            "a1.stdin"
        );
        assert!(names_file(
            "the input a1.stdin gives",
            &path_of(prefix("."), &name("a1"), FileKind::Stdin)
        ));
        assert_eq!(FileKind::listed(), ".args, .stdin, .expected or .stderr");
        for kind in FileKind::ALL {
            assert_eq!(FileKind::of_ext(kind.ext()), Some(kind));
        }
        assert_eq!(FileKind::of_ext("txt"), None);
    }

    /// A request of protocol 2 whose model has one Context with one Contract holding `clauses` (id, text).
    fn request(clauses: &[(&str, &str)]) -> Vec<u8> {
        let clauses: Vec<_> = clauses
            .iter()
            .map(|(id, text)| json!({"id":id,"label":"C","revision":1,"text":text}))
            .collect();
        serde_json::to_vec(&json!({
            "protocol":2,"moment":"check","rules":["ext.accept"],"read_at":0,"params":{},
            "model":{"version":4,"contexts":[{"id":"c","name":"x","basis":"contract","uses":[],
                "contracts":[{"id":"k","name":"k","title":"t","revision":1,"clauses":clauses}],
                "interfaces":[],"harnesses":[]}]}
        }))
        .unwrap()
    }

    fn subject(id: &str) -> Option<ExternalSubject> {
        Some(ExternalSubject {
            kind: "clause".to_string(),
            id: id.to_string(),
        })
    }

    #[test]
    fn a_failing_example_is_a_finding_about_the_clause_that_names_its_file() {
        let request = request(&[
            (
                "c1",
                "the input `acceptance/a1.stdin` gives `acceptance/a1.expected`",
            ),
            ("c2", "nothing about it"),
            ("c3", "see acceptance/a10.expected"),
        ]);
        let reply = reply(&request, "acceptance", &[failing("a1")]).unwrap();
        assert_eq!(reply.protocol, 2);
        assert_eq!(
            reply.findings,
            [ExternalFinding {
                rule: "ext.accept".to_string(),
                subject: subject("c1"),
                message: "a1: exit code: expected 0, got 3".to_string(),
                level: None,
            }]
        );
    }

    #[test]
    fn two_clauses_that_name_it_give_two_findings_in_model_order_and_one_clause_gives_one() {
        let request = request(&[
            ("c1", "acceptance/a1.args and acceptance/a1.expected"),
            ("c2", "other"),
            ("c3", "the file acceptance/a1.stdin."),
        ]);
        let reply = reply(&request, "./acceptance/", &[failing("a1")]).unwrap();
        let subjects: Vec<_> = reply.findings.iter().map(|f| f.subject.clone()).collect();
        assert_eq!(subjects, [subject("c1"), subject("c3")]);
    }

    #[test]
    fn examples_are_taken_in_the_order_given_and_a_passing_one_gives_nothing() {
        let request = request(&[
            ("c1", "acceptance/a1.expected"),
            ("c2", "acceptance/a2.expected"),
            ("c3", "acceptance/a3.expected"),
        ]);
        let results = [failing("a3"), passing("a2"), failing("a1")];
        let reply = reply(&request, "acceptance", &results).unwrap();
        let found: Vec<_> = reply
            .findings
            .iter()
            .map(|f| (f.subject.clone(), f.message.clone()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    subject("c3"),
                    "a3: exit code: expected 0, got 3".to_string()
                ),
                (
                    subject("c1"),
                    "a1: exit code: expected 0, got 3".to_string()
                ),
            ]
        );
        assert!(
            super::reply(&request, "acceptance", &[passing("a1")])
                .unwrap()
                .findings
                .is_empty()
        );
    }

    #[test]
    fn an_example_no_clause_names_is_a_finding_about_the_project() {
        let request = request(&[("c1", "acceptance/b1.expected")]);
        let reply = reply(&request, "./acceptance/", &[failing("a1")]).unwrap();
        assert_eq!(
            reply.findings,
            [ExternalFinding {
                rule: "ext.accept".to_string(),
                subject: None,
                message:
                    "a1: exit code: expected 0, got 3 (no clause names acceptance/a1.expected)"
                        .to_string(),
                level: None,
            }]
        );
        let here = super::reply(&request, ".", &[failing("a1")]).unwrap();
        assert_eq!(
            here.findings[0].message,
            "a1: exit code: expected 0, got 3 (no clause names a1.expected)"
        );
    }

    #[test]
    fn a_request_without_contracts_gives_project_findings() {
        let request = serde_json::to_vec(&json!({
            "protocol":2,"moment":"check","rules":["ext.accept"],"read_at":0,"params":{},
            "model":{"version":4,"contexts":[{"id":"c","name":"x","basis":"contract","uses":[]}]}
        }))
        .unwrap();
        let reply = reply(&request, "acceptance", &[failing("a1")]).unwrap();
        assert_eq!(reply.findings.len(), 1);
        assert_eq!(reply.findings[0].subject, None);
        // no contexts at all, and no model
        for bare in [
            json!({"protocol":2,"rules":["r"],"model":{"version":4}}),
            json!({"protocol":2,"rules":["r"]}),
        ] {
            let bare = serde_json::to_vec(&bare).unwrap();
            let reply = super::reply(&bare, "acceptance", &[failing("a1")]).unwrap();
            assert_eq!(reply.findings[0].rule, "r");
            assert_eq!(reply.findings[0].subject, None);
        }
    }

    #[test]
    fn a_reply_with_no_failing_example_has_no_findings_and_is_protocol_two() {
        let reply = reply(&request(&[]), "acceptance", &[passing("a1")]).unwrap();
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"protocol":2,"findings":[]}"#
        );
    }

    #[test]
    fn a_request_that_cannot_be_answered_is_an_error() {
        let results = [failing("a1")];
        let other = |value: serde_json::Value| serde_json::to_vec(&value).unwrap();
        assert!(matches!(
            reply(&other(json!({"protocol":1,"rules":["r"]})), "d", &results),
            Err(AcceptError::Protocol { got: 1, want: 2 })
        ));
        assert!(matches!(
            reply(
                &other(json!({"protocol":2,"rules":["r","s"]})),
                "d",
                &results
            ),
            Err(AcceptError::Rules(2))
        ));
        assert!(matches!(
            reply(&other(json!({"protocol":2,"rules":[]})), "d", &results),
            Err(AcceptError::Rules(0))
        ));
        assert!(matches!(
            reply(&other(json!({"protocol":2})), "d", &results),
            Err(AcceptError::Rules(0))
        ));
        assert!(matches!(
            reply(b"not json", "d", &results),
            Err(AcceptError::Request(_))
        ));
        assert!(matches!(
            reply(b"", "d", &results),
            Err(AcceptError::Request(_))
        ));
        assert!(matches!(
            reply(&other(json!({"rules":["r"]})), "d", &results),
            Err(AcceptError::Request(_))
        ));
    }

    #[test]
    fn the_report_for_a_person_has_a_line_each_and_a_summary() {
        let results = [
            passing("a1"),
            (
                name("a2"),
                Verdict::Fail(vec![
                    Why::Abnormal("did not end within 1 s".to_string()),
                    Why::Unreadable("x".to_string()),
                ]),
            ),
        ];
        assert_eq!(
            lines(&results),
            "ok a1\nfail a2: did not end within 1 s; x\n1 passed, 1 failed\n"
        );
    }

    #[cfg(unix)]
    mod running {
        use super::*;

        fn example(stdin: &str, expected: &str, args: &[&str]) -> Example {
            Example {
                name: name("e"),
                args: args.iter().map(|a| a.to_string()).collect(),
                stdin: stdin.as_bytes().to_vec(),
                expected: Ok(Expected::parse(expected.as_bytes(), None).unwrap()),
            }
        }

        async fn run_in_temp(example: &Example, command: &[&str], secs: u64) -> Verdict {
            let command: Vec<String> = command.iter().map(|c| c.to_string()).collect();
            run(
                example,
                &command,
                &std::env::temp_dir(),
                Duration::from_secs(secs),
            )
            .await
        }

        #[tokio::test]
        async fn a_program_that_answers_as_expected_passes() {
            let example = example("hi\nthere\n", "hi\nthere\n#exit 0\n", &[]);
            assert_eq!(run_in_temp(&example, &["/bin/cat"], 10).await, Verdict::Ok);
        }

        #[tokio::test]
        async fn a_different_output_fails_with_the_line() {
            let example = example("hi\n", "bye\n#exit 0\n", &[]);
            let verdict = run_in_temp(&example, &["/bin/cat"], 10).await;
            assert_eq!(
                verdict.to_string(),
                r#"standard output differs at line 1: expected "bye", got "hi""#
            );
        }

        #[tokio::test]
        async fn the_arguments_of_the_example_follow_those_of_the_command() {
            let example = example("", "a-b\n#exit 0\n", &["a", "b"]);
            let verdict = run_in_temp(&example, &["printf", "%s-%s\\n"], 10).await;
            assert_eq!(verdict, Verdict::Ok, "{verdict}");
        }

        #[tokio::test]
        async fn a_program_that_does_not_exist_cannot_start() {
            let example = example("", "#exit 0\n", &[]);
            let verdict = run_in_temp(&example, &["/nonexistent/chidev-program"], 10).await;
            let Verdict::Fail(reasons) = verdict else {
                panic!("passed");
            };
            let [Why::Abnormal(text)] = reasons.as_slice() else {
                panic!("{reasons:?}");
            };
            assert!(text.starts_with("cannot start: "), "{text}");
        }

        #[tokio::test]
        async fn a_program_that_does_not_end_in_time_fails_and_is_stopped() {
            let example = example("", "#exit 0\n", &["30"]);
            let started = std::time::Instant::now();
            let verdict = run_in_temp(&example, &["sleep"], 1).await;
            assert_eq!(
                verdict,
                Verdict::fail(Why::Abnormal("did not end within 1 s".to_string()))
            );
            assert!(started.elapsed() < Duration::from_secs(20));
        }

        #[tokio::test]
        async fn an_example_that_could_not_be_read_fails_without_running() {
            let mut broken = example("", "#exit 0\n", &[]);
            broken.expected = Err(vec![Why::Unreadable("missing e.stdin".to_string())]);
            let verdict = run_in_temp(&broken, &["/nonexistent/chidev-program"], 10).await;
            assert_eq!(verdict.to_string(), "missing e.stdin");
        }
    }
}
