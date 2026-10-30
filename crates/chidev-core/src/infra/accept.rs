//! The runner of acceptance examples, `chidev accept`.
//!
//! An example is a name `<name>` for which a directory holds `<name>.args`, `<name>.stdin`, `<name>.expected` and,
//! optionally, `<name>.stderr`; [`crate::domain::vo::example`] says what each file holds and how an answer is
//! judged. This module reads the directory, runs the program once per example, and gives the verdicts as lines for a
//! person ([`lines`]).
//!
//! The program runs through [`crate::infra::process`]: without a shell, in a process group of its own that is killed
//! when the run ends (so a descendant is not left running), under a deadline, with a cap on what it may write to each
//! pipe. A program that gave no answer (it could not start, it did not end in time, it wrote too much, a signal ended
//! it) is a failure of the example with one reason, never an answer to compare.
//!
//! The examples a Harness of kind `examples` holds in the model are run by `measure`, which runs each by
//! [`run_resolved`], the code this module runs a directory with (the model holds them as text; the files are only how
//! a person writes them, and [`load`] reads them for a put).

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use thiserror::Error;

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

/// Why the examples could not be run at all: the failures of usage, as opposed to a failure of one example, which is
/// its [`Verdict`].
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

#[cfg(test)]
mod tests {
    use super::*;

    fn name(name: &str) -> ExampleName {
        ExampleName::new(name).unwrap()
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

    #[test]
    fn an_example_the_model_holds_is_run_as_the_files_would_give_it() {
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
