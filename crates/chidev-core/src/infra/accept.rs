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
//! reach it (an anti-corruption layer); and it answers with the wire types of [`crate::app::ports::external`]. The
//! examples of such a check are the ones a Harness of kind `examples` holds in the model sent, run as they are held
//! (the model holds them as text; the files are only how a person writes them, and [`load`] reads them for a put).

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use crate::app::ports::external::{
    ExternalFinding, ExternalReply, ExternalSubject, REQUEST_PROTOCOL,
};
use crate::domain::vo::example::{
    self, Answer, Example as Held, ExampleError, ExampleName, Expected, Verdict, Why, judge,
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
    /// A file an example needs is not in the directory.
    #[error("missing {0}")]
    MissingFile(String),
    /// A file of an example cannot be read.
    #[error("cannot read {file}: {message}")]
    UnreadableFile {
        /// The file, by its name in the directory.
        file: String,
        /// The error of the system.
        message: String,
    },
    /// A file of an example is not text: the model holds text.
    #[error("{0} is not UTF-8")]
    FileNotUtf8(String),
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
    /// The model sent has no harnesses, so the check was not registered to read them.
    #[error(
        "the model sent has no harnesses: register the check with `reads = [\"contracts\", \"harnesses\"]`"
    )]
    NoHarnesses,
    /// The model sent has no Context of this name.
    #[error("the model sent has no context `{0}`")]
    NoContext(String),
    /// The Context has no Harness of this name.
    #[error("the context `{context}` has no harness `{harness}`")]
    NoHarness {
        /// The Context.
        context: String,
        /// The Harness asked for.
        harness: String,
    },
    /// The Harness is of a kind that holds no examples.
    #[error("the harness `{harness}` is of {kind}, not examples")]
    NotExamples {
        /// The Harness.
        harness: String,
        /// Its kind, as the model sent says.
        kind: String,
    },
    /// A Clause a Case cites is not in the model sent, so a finding could not be about it.
    #[error(
        "the clause {clause}, which the case `{case}` cites, is not in the model sent: register the check with `reads = [\"contracts\", \"harnesses\"]`"
    )]
    ClauseNotSent {
        /// The Case.
        case: String,
        /// The id of the Clause.
        clause: String,
    },
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

/// The example the model holds, named `name`, as the runner takes it: the same arguments, input and expectation as
/// the files would give, so that the run and the judgment are one code path.
impl From<(ExampleName, &Held)> for Example {
    fn from((name, held): (ExampleName, &Held)) -> Example {
        Example {
            name,
            args: held.args(),
            stdin: held.input().as_bytes().to_vec(),
            expected: Ok(held.expectation().clone()),
        }
    }
}

/// The examples of `dir`, in the byte order of their names. A regular file `<name>.args`, `<name>.stdin`,
/// `<name>.expected` or `<name>.stderr` makes `<name>` an example; anything else in the directory is ignored. A
/// directory that cannot be read, or that holds no example, is an error.
pub(crate) async fn read_dir(dir: &Path) -> Result<Vec<Example>, AcceptError> {
    let names = names_in(dir).await?;
    let mut examples = Vec::with_capacity(names.len());
    for name in names {
        examples.push(read_example(dir, name).await);
    }
    Ok(examples)
}

/// An example as its files give it, in text, for a put: the four texts as written. See [`load`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The name of the example.
    pub name: ExampleName,
    /// The text of `<name>.args`.
    pub args: String,
    /// The text of `<name>.stdin`.
    pub stdin: String,
    /// The text of `<name>.expected`.
    pub expected: String,
    /// The text of `<name>.stderr`; none when the file is not there.
    pub stderr: Option<String>,
}

/// The text of the file `kind` of the example `name` in `dir`; none when there is no such file.
async fn text_of(
    dir: &Path,
    name: &ExampleName,
    kind: FileKind,
) -> Result<Option<String>, AcceptError> {
    let file = kind.file(name);
    match tokio::fs::read(dir.join(&file)).await {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| AcceptError::FileNotUtf8(file)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AcceptError::UnreadableFile {
            file,
            message: e.to_string(),
        }),
    }
}

/// The examples of `dir` as text, to be put in the model, in the byte order of their names. The examples are the
/// ones [`read_dir`] lists, but this is strict where the runner is lenient: a run reports an example whose file is
/// missing as a failure of that example, while an example that is put must be whole, so a missing `.args`,
/// `.stdin` or `.expected`, a file that cannot be read and a file that is not UTF-8 are an error that names the file.
/// `.stderr` is the one file that may be absent.
pub async fn load(dir: &Path) -> Result<Vec<Loaded>, AcceptError> {
    let names = names_in(dir).await?;
    let mut loaded = Vec::with_capacity(names.len());
    for name in names {
        let args = text_of(dir, &name, FileKind::Args).await?;
        let stdin = text_of(dir, &name, FileKind::Stdin).await?;
        let expected = text_of(dir, &name, FileKind::Expected).await?;
        let stderr = text_of(dir, &name, FileKind::Stderr).await?;
        let required = |text: Option<String>, kind: FileKind| {
            text.ok_or_else(|| AcceptError::MissingFile(kind.file(&name)))
        };
        let args = required(args, FileKind::Args)?;
        let stdin = required(stdin, FileKind::Stdin)?;
        let expected = required(expected, FileKind::Expected)?;
        loaded.push(Loaded {
            name,
            args,
            stdin,
            expected,
            stderr,
        });
    }
    Ok(loaded)
}

/// The names of the examples of `dir`: the regular files with an extension of an example's make their stems. A
/// directory that cannot be read, or that holds no example, is an error.
async fn names_in(dir: &Path) -> Result<BTreeSet<ExampleName>, AcceptError> {
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
    Ok(names)
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
    if let Err(reasons) = &example.expected {
        return Verdict::Fail(reasons.clone());
    }
    let Some((program, fixed)) = command.split_first() else {
        return Verdict::fail(Why::Abnormal("cannot start: no command".to_string()));
    };
    // `locate`, not `resolve`: a program that is a link to another picks what to run by the name it is started as
    let executable = match process::locate(cwd, program).await {
        Ok(executable) => executable,
        Err(failure) => return Verdict::fail(abnormal(&failure, timeout)),
    };
    run_resolved(
        example,
        &Resolved {
            executable: &executable,
            fixed,
            env: &[],
            cwd,
        },
        timeout,
    )
    .await
}

/// A program that is already found: the file to start, the words that come before the example's own arguments, the
/// variables added to the environment, and the directory it runs in. What [`run`] finds from a command line and the
/// runner of a measure finds in its copy of the project, so that one example is run and judged by one code path.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Resolved<'a> {
    /// The file to start.
    pub executable: &'a Path,
    /// The arguments before those of the example.
    pub fixed: &'a [String],
    /// Variables added to the environment the runner inherits.
    pub env: &'a [(String, String)],
    /// The directory it runs in.
    pub cwd: &'a Path,
}

/// Runs the program `program` for `example` and judges what it answered, as [`run`] does once it has found the
/// program. The arguments of the example follow `program.fixed`; its standard input is the example's; `timeout`
/// bounds the one example. An example that could not be read fails without running anything.
pub(crate) async fn run_resolved(
    example: &Example,
    program: &Resolved<'_>,
    timeout: Duration,
) -> Verdict {
    let expected = match &example.expected {
        Ok(expected) => expected,
        Err(reasons) => return Verdict::Fail(reasons.clone()),
    };
    let deadline = match process::deadline_in(timeout) {
        Ok(deadline) => deadline,
        Err(failure) => return Verdict::fail(abnormal(&failure, timeout)),
    };
    let args: Vec<String> = program.fixed.iter().chain(&example.args).cloned().collect();
    let pipe = Pipe {
        limit: LIMIT,
        over: Over::Fail,
    };
    let spec = Spec {
        executable: program.executable,
        args: &args,
        cwd: program.cwd,
        env: program.env,
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
    name: String,
    /// Present only when the check reads `contracts`.
    #[serde(default)]
    contracts: Option<Vec<Contract>>,
    /// Present only when the check reads `harnesses`.
    #[serde(default)]
    harnesses: Option<Vec<HarnessIn>>,
}

#[derive(Debug, Deserialize)]
struct Contract {
    #[serde(default)]
    clauses: Vec<ClauseIn>,
}

#[derive(Debug, Deserialize)]
struct ClauseIn {
    id: String,
}

#[derive(Debug, Deserialize)]
struct HarnessIn {
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    cases: Vec<CaseIn>,
}

#[derive(Debug, Deserialize)]
struct CaseIn {
    name: String,
    #[serde(default)]
    example: Option<ExampleIn>,
    #[serde(default)]
    cites: CitesIn,
}

#[derive(Debug, Default, Deserialize)]
struct CitesIn {
    #[serde(default)]
    clauses: Vec<ClauseIn>,
}

#[derive(Debug, Deserialize)]
struct ExampleIn {
    args: String,
    stdin: String,
    expected: String,
    #[serde(default)]
    stderr: Option<String>,
}

/// One example of the Harness a check is about, as the model sent holds it, with the ids of the Clauses its Case
/// cites.
#[derive(Debug)]
struct Run {
    example: Example,
    clauses: Vec<String>,
}

/// The request of an external check as the runner needs it: the one rule it is asked for, and the examples of the
/// Harness it is about, with the Clauses each one's Case cites. It is read, and everything that could make it
/// unanswerable is found, before any example runs, so a request that cannot be answered runs nothing.
#[derive(Debug)]
pub struct Call {
    rule: String,
    runs: Vec<Run>,
}

impl Call {
    /// Reads `request`, the document the check was sent, for the Harness `harness` of the Context `context`. A
    /// request that is not JSON, is not protocol 2 or does not ask for exactly one rule is an error, and so is one
    /// whose model has no harnesses, has no such Context or Harness, has a Harness that is not of kind `examples`,
    /// or lacks a Clause one of its Cases cites.
    pub fn read(request: &[u8], context: &str, harness: &str) -> Result<Call, AcceptError> {
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
        let contexts = request.model.contexts;
        if contexts.iter().all(|c| c.harnesses.is_none()) {
            return Err(AcceptError::NoHarnesses);
        }
        let known: BTreeSet<&str> = contexts
            .iter()
            .flat_map(|c| c.contracts.iter().flatten())
            .flat_map(|contract| &contract.clauses)
            .map(|clause| clause.id.as_str())
            .collect();
        let owner = contexts
            .iter()
            .find(|c| c.name == context)
            .ok_or_else(|| AcceptError::NoContext(context.to_string()))?;
        let found = owner
            .harnesses
            .iter()
            .flatten()
            .find(|h| h.name == harness)
            .ok_or_else(|| AcceptError::NoHarness {
                context: context.to_string(),
                harness: harness.to_string(),
            })?;
        if found.kind != "examples" {
            return Err(AcceptError::NotExamples {
                harness: harness.to_string(),
                kind: found.kind.clone(),
            });
        }
        let mut runs = Vec::with_capacity(found.cases.len());
        for case in &found.cases {
            let malformed =
                |what: String| AcceptError::Request(format!("the case `{}` {what}", case.name));
            let name = ExampleName::new(&case.name).map_err(|e| malformed(e.to_string()))?;
            let Some(e) = &case.example else {
                return Err(malformed("holds no example".to_string()));
            };
            let held = Held::new(
                e.args.clone(),
                e.stdin.clone(),
                e.expected.clone(),
                e.stderr.clone(),
            )
            .map_err(|err| malformed(format!("has an expected output that {err}")))?;
            let mut clauses = Vec::with_capacity(case.cites.clauses.len());
            for clause in &case.cites.clauses {
                if !known.contains(clause.id.as_str()) {
                    return Err(AcceptError::ClauseNotSent {
                        case: case.name.clone(),
                        clause: clause.id.clone(),
                    });
                }
                clauses.push(clause.id.clone());
            }
            runs.push(Run {
                example: Example::from((name, &held)),
                clauses,
            });
        }
        Ok(Call { rule, runs })
    }

    /// Runs every example of the Harness against `command`, one after another, and gives the verdicts in the order
    /// of the Harness's Cases. Each is run as the model holds it, by the code [`run_all`] runs a directory with.
    pub async fn run(
        &self,
        command: &[String],
        cwd: &Path,
        timeout: Duration,
    ) -> Vec<(ExampleName, Verdict)> {
        let mut results = Vec::with_capacity(self.runs.len());
        for r in &self.runs {
            let verdict = run(&r.example, command, cwd, timeout).await;
            results.push((r.example.name.clone(), verdict));
        }
        results
    }

    /// The reply for `results`, the verdicts of the examples of the Harness.
    ///
    /// A failing example gives a finding for each Clause its Case cites (`<name>: <reasons>`), so a finding is about
    /// the Clauses the example is an example of, found by their ids and not by what a Clause says. A passing example
    /// gives nothing.
    pub fn reply(&self, results: &[(ExampleName, Verdict)]) -> ExternalReply {
        let mut findings = Vec::new();
        for (name, verdict) in results {
            if verdict.is_ok() {
                continue;
            }
            let message = format!("{name}: {verdict}");
            let cited: &[String] = self
                .runs
                .iter()
                .find(|r| r.example.name == *name)
                .map_or(&[], |r| r.clauses.as_slice());
            let subjects: Vec<Option<ExternalSubject>> = if cited.is_empty() {
                // a Case cites a Clause at least, but a failure is never dropped for want of one to be about
                vec![None]
            } else {
                cited
                    .iter()
                    .map(|id| {
                        Some(ExternalSubject {
                            kind: "clause".to_string(),
                            id: id.clone(),
                        })
                    })
                    .collect()
            };
            for subject in subjects {
                findings.push(ExternalFinding {
                    rule: self.rule.clone(),
                    subject,
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
    fn the_files_of_an_example_are_told_by_their_extensions() {
        assert_eq!(FileKind::listed(), ".args, .stdin, .expected or .stderr");
        for kind in FileKind::ALL {
            assert_eq!(FileKind::of_ext(kind.ext()), Some(kind));
        }
        assert_eq!(FileKind::of_ext("txt"), None);
    }

    /// A case of a Harness of examples as the view shows it: one example, citing these Clauses.
    fn case(name: &str, expected: &str, clauses: &[&str]) -> serde_json::Value {
        json!({
            "id": format!("case-{name}"), "name": name, "lines": null, "revision": 1,
            "example": {"args":"","stdin":"","expected":expected},
            "cites": {"items": [],
                      "clauses": clauses.iter().map(|id| json!({"id":id,"revision":1})).collect::<Vec<_>>()}
        })
    }

    /// A request of protocol 2 whose model has the Context `x`, with one Contract holding the Clauses `c1`, `c2` and
    /// `c3`, and a Harness `acceptance` of kind `kind` with these `cases`.
    fn request_of(kind: &str, cases: &[serde_json::Value]) -> Vec<u8> {
        let clauses: Vec<_> = ["c1", "c2", "c3"]
            .iter()
            .map(|id| json!({"id":id,"label":"C","revision":1,"text":"t"}))
            .collect();
        serde_json::to_vec(&json!({
            "protocol":2,"moment":"check","rules":["ext.accept"],"read_at":0,"params":{},
            "model":{"version":5,"contexts":[{"id":"c","name":"x","basis":"contract","uses":[],
                "contracts":[{"id":"k","name":"k","title":"t","revision":1,"clauses":clauses}],
                "harnesses":[{"id":"h","name":"acceptance","revision":1,"kind":kind,"cases":cases}]}]}
        }))
        .unwrap()
    }

    fn request(cases: &[serde_json::Value]) -> Vec<u8> {
        request_of("examples", cases)
    }

    fn call(request: &[u8]) -> Result<Call, AcceptError> {
        Call::read(request, "x", "acceptance")
    }

    fn subject(id: &str) -> Option<ExternalSubject> {
        Some(ExternalSubject {
            kind: "clause".to_string(),
            id: id.to_string(),
        })
    }

    #[test]
    fn a_failing_example_is_a_finding_about_each_clause_its_case_cites() {
        let request = request(&[
            case("a1", "#exit 0\n", &["c1", "c3"]),
            case("a2", "#exit 0\n", &["c2"]),
        ]);
        let call = call(&request).unwrap();
        let reply = call.reply(&[failing("a1")]);
        assert_eq!(reply.protocol, 2);
        assert_eq!(
            reply.findings,
            ["c1", "c3"].map(|clause| ExternalFinding {
                rule: "ext.accept".to_string(),
                subject: subject(clause),
                message: "a1: exit code: expected 0, got 3".to_string(),
                level: None,
            })
        );
    }

    #[test]
    fn findings_follow_the_results_and_a_passing_example_gives_nothing() {
        let request = request(&[
            case("a1", "#exit 0\n", &["c1"]),
            case("a2", "#exit 0\n", &["c2"]),
            case("a3", "#exit 0\n", &["c3"]),
        ]);
        let call = call(&request).unwrap();
        let reply = call.reply(&[failing("a3"), passing("a2"), failing("a1")]);
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
        let none = call.reply(&[passing("a1")]);
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"protocol":2,"findings":[]}"#
        );
    }

    #[test]
    fn a_failure_whose_case_cites_nothing_is_about_the_project_and_not_dropped() {
        let request = request(&[case("a1", "#exit 0\n", &[])]);
        let reply = call(&request).unwrap().reply(&[failing("a1")]);
        assert_eq!(reply.findings.len(), 1);
        assert_eq!(reply.findings[0].subject, None);
    }

    #[test]
    fn the_examples_to_run_are_the_ones_the_model_holds_in_the_order_of_its_cases() {
        let request = request(&[
            case("b", "#exit 0\n", &["c1"]),
            case("a", "#exit 0\n", &["c1"]),
        ]);
        let call = call(&request).unwrap();
        let names: Vec<_> = call
            .runs
            .iter()
            .map(|r| r.example.name.to_string())
            .collect();
        assert_eq!(names, ["b", "a"]);
        // the same example as the files would give: its arguments, input and expectation
        let held = Held::new(
            "-n 3".to_string(),
            "in\n".to_string(),
            "out\n#exit 2\n".to_string(),
            Some("err\n".to_string()),
        )
        .unwrap();
        let example = Example::from((name("e"), &held));
        assert_eq!(example.args, ["-n", "3"]);
        assert_eq!(example.stdin, b"in\n");
        let Ok(expected) = &example.expected else {
            panic!("an example the model holds is readable");
        };
        assert_eq!(expected.output(), b"out\n");
        assert_eq!(expected.exit(), 2);
        assert_eq!(expected.errors(), Some(&b"err\n"[..]));
    }

    #[test]
    fn a_request_that_cannot_be_answered_is_an_error() {
        let one = case("a1", "#exit 0\n", &["c1"]);
        let other = |value: serde_json::Value| serde_json::to_vec(&value).unwrap();
        assert!(matches!(
            call(&other(json!({"protocol":1,"rules":["r"]}))),
            Err(AcceptError::Protocol { got: 1, want: 2 })
        ));
        assert!(matches!(
            call(&other(json!({"protocol":2,"rules":["r","s"]}))),
            Err(AcceptError::Rules(2))
        ));
        assert!(matches!(
            call(&other(json!({"protocol":2,"rules":[]}))),
            Err(AcceptError::Rules(0))
        ));
        assert!(matches!(
            call(&other(json!({"protocol":2}))),
            Err(AcceptError::Rules(0))
        ));
        assert!(matches!(call(b"not json"), Err(AcceptError::Request(_))));
        assert!(matches!(call(b""), Err(AcceptError::Request(_))));
        assert!(matches!(
            call(&other(json!({"rules":["r"]}))),
            Err(AcceptError::Request(_))
        ));
        // a case that holds no example, or an example no program could be judged by
        let mut bare = one.clone();
        bare["example"] = serde_json::Value::Null;
        assert!(matches!(
            call(&request(&[bare])),
            Err(AcceptError::Request(_))
        ));
        assert!(matches!(
            call(&request(&[case("a1", "no exit line\n", &["c1"])])),
            Err(AcceptError::Request(_))
        ));
        assert!(matches!(
            call(&request(&[case("a/1", "#exit 0\n", &["c1"])])),
            Err(AcceptError::Request(_))
        ));
    }

    #[test]
    fn a_request_without_what_the_check_reads_is_a_usage_error_found_before_anything_runs() {
        let one = case("a1", "#exit 0\n", &["c1"]);
        // no harnesses in the model: no context has them, or there is no context at all
        for bare in [
            json!({"protocol":2,"rules":["r"],"model":{"version":5,"contexts":[
                {"id":"c","name":"x","basis":"contract","uses":[]}]}}),
            json!({"protocol":2,"rules":["r"],"model":{"version":5}}),
            json!({"protocol":2,"rules":["r"]}),
        ] {
            let bare = serde_json::to_vec(&bare).unwrap();
            assert!(matches!(call(&bare), Err(AcceptError::NoHarnesses)));
        }
        let sent = request(&[one]);
        assert!(matches!(
            Call::read(&sent, "y", "acceptance"),
            Err(AcceptError::NoContext(c)) if c == "y"
        ));
        assert!(matches!(
            Call::read(&sent, "x", "other"),
            Err(AcceptError::NoHarness { context, harness }) if context == "x" && harness == "other"
        ));
        let sim = request_of("sim", &[]);
        assert!(matches!(
            call(&sim),
            Err(AcceptError::NotExamples { harness, kind }) if harness == "acceptance" && kind == "sim"
        ));
        // a Clause the model sent lacks: the Harness is there, the Contracts were not read
        let no_contracts = serde_json::to_vec(&json!({
            "protocol":2,"rules":["r"],
            "model":{"version":5,"contexts":[{"id":"c","name":"x","basis":"contract","uses":[],
                "harnesses":[{"id":"h","name":"acceptance","revision":1,"kind":"examples",
                    "cases":[case("a1", "#exit 0\n", &["c1"])]}]}]}
        }))
        .unwrap();
        assert!(matches!(
            call(&no_contracts),
            Err(AcceptError::ClauseNotSent { case, clause }) if case == "a1" && clause == "c1"
        ));
        let elsewhere = request(&[case("a1", "#exit 0\n", &["c9"])]);
        assert!(matches!(
            call(&elsewhere),
            Err(AcceptError::ClauseNotSent { clause, .. }) if clause == "c9"
        ));
    }

    #[test]
    fn a_clause_of_a_context_the_harness_is_not_in_is_one_the_model_sent() {
        // the Clause a Case cites may be in a Context its own uses: the model sent has it
        let request = serde_json::to_vec(&json!({
            "protocol":2,"rules":["r"],
            "model":{"version":5,"contexts":[
                {"id":"b","name":"base","basis":"contract","uses":[],
                 "contracts":[{"id":"k","name":"k","title":"t","revision":1,
                     "clauses":[{"id":"c7","label":"C","revision":1,"text":"t"}]}],
                 "harnesses":[]},
                {"id":"c","name":"x","basis":"contract","uses":["base"],"contracts":[],
                 "harnesses":[{"id":"h","name":"acceptance","revision":1,"kind":"examples",
                    "cases":[case("a1", "#exit 0\n", &["c7"])]}]}]}
        }))
        .unwrap();
        assert!(call(&request).is_ok());
    }

    mod files {
        use super::*;

        fn temp(tag: &str) -> std::path::PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "chidev-accept-load-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn write(dir: &Path, file: &str, text: &[u8]) {
            std::fs::write(dir.join(file), text).unwrap();
        }

        fn whole(dir: &Path, name: &str) {
            write(dir, &format!("{name}.args"), b"-n 3\n");
            write(dir, &format!("{name}.stdin"), b"in\n");
            write(dir, &format!("{name}.expected"), b"out\n#exit 0\n");
        }

        #[tokio::test]
        async fn the_examples_of_a_directory_are_loaded_as_text_in_the_order_of_their_names() {
            let dir = temp("whole");
            whole(&dir, "b");
            whole(&dir, "a");
            write(&dir, "a.stderr", b"warn\n");
            write(&dir, "README.md", b"not an example");
            let loaded = load(&dir).await.unwrap();
            assert_eq!(
                loaded,
                [
                    Loaded {
                        name: name("a"),
                        args: "-n 3\n".to_string(),
                        stdin: "in\n".to_string(),
                        expected: "out\n#exit 0\n".to_string(),
                        stderr: Some("warn\n".to_string()),
                    },
                    Loaded {
                        name: name("b"),
                        args: "-n 3\n".to_string(),
                        stdin: "in\n".to_string(),
                        expected: "out\n#exit 0\n".to_string(),
                        stderr: None,
                    },
                ]
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }

        #[tokio::test]
        async fn a_missing_required_file_or_one_that_is_not_text_is_an_error_naming_the_file() {
            let dir = temp("broken");
            whole(&dir, "a");
            std::fs::remove_file(dir.join("a.stdin")).unwrap();
            let err = load(&dir).await.unwrap_err();
            assert!(
                matches!(&err, AcceptError::MissingFile(f) if f == "a.stdin"),
                "{err}"
            );
            write(&dir, "a.stdin", b"in\n");
            write(&dir, "a.expected", &[0xff, 0xfe]);
            let err = load(&dir).await.unwrap_err();
            assert!(
                matches!(&err, AcceptError::FileNotUtf8(f) if f == "a.expected"),
                "{err}"
            );
            // a file that is a directory cannot be read, and is told with the system's words
            std::fs::remove_file(dir.join("a.expected")).unwrap();
            std::fs::create_dir(dir.join("a.expected")).unwrap();
            let err = load(&dir).await.unwrap_err();
            assert!(
                matches!(&err, AcceptError::UnreadableFile { file, .. } if file == "a.expected"),
                "{err}"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }

        #[tokio::test]
        async fn a_directory_with_no_example_or_none_at_all_is_an_error() {
            let dir = temp("empty");
            assert!(matches!(load(&dir).await, Err(AcceptError::NoExamples(_))));
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(matches!(
                load(&dir).await,
                Err(AcceptError::Directory { .. })
            ));
        }
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
