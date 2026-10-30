//! `chidev accept`, end to end through the binary: the examples of a directory run against a small program (a line
//! for each, a count, the exit codes). The program is a shell script written by the test; it does what its first
//! argument says.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `echo` prints its input, `exit3` exits with 3, `err` writes `oops` on standard error, `sleep` sleeps for 30 s.
const PROGRAM: &str = r#"#!/bin/sh
case "$1" in
  echo) cat ;;
  exit3) cat > /dev/null; exit 3 ;;
  err) echo oops >&2 ;;
  sleep) sleep 30 ;;
esac
"#;

/// A directory of its own under cargo's temporary one, with the program `prog.sh` and an empty directory `ex`.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ex")).expect("a temporary directory");
    std::fs::write(dir.join("prog.sh"), PROGRAM).expect("a file");
    std::fs::set_permissions(dir.join("prog.sh"), std::fs::Permissions::from_mode(0o755))
        .expect("executable");
    dir
}

/// Writes `ex/<name>.<ext>` for each of the files given.
fn example(dir: &Path, name: &str, files: &[(&str, &str)]) {
    for (ext, text) in files {
        std::fs::write(dir.join("ex").join(format!("{name}.{ext}")), text).expect("a file");
    }
}

/// A whole example: the arguments, the input, and the expected output with its exit line.
fn full(dir: &Path, name: &str, args: &str, stdin: &str, expected: &str) {
    example(
        dir,
        name,
        &[("args", args), ("stdin", stdin), ("expected", expected)],
    );
}

/// `chidev` with `args` in `dir`, standard input being `stdin`.
fn chidev(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("chidev runs");
    child
        .stdin
        .take()
        .expect("a pipe")
        .write_all(stdin.as_bytes())
        .expect("the input is written");
    child.wait_with_output().expect("chidev ends")
}

/// `chidev accept` of the directory `ex` against `./prog.sh`, with `flags` before the directory.
fn accept(dir: &Path, flags: &[&str], stdin: &str) -> Output {
    let mut args = vec!["accept"];
    args.extend_from_slice(flags);
    args.extend(["ex", "--", "./prog.sh"]);
    chidev(dir, &args, stdin)
}

fn out(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf-8")
}

fn err(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf-8")
}

#[test]
fn every_example_that_passes_is_a_line_and_the_exit_code_is_0() {
    let dir = fresh_dir("accept-ok");
    full(&dir, "a1", "echo", "hi\nthere\n", "hi\nthere\n#exit 0\n");
    full(&dir, "a2", "exit3", "", "#exit 3\n");
    example(
        &dir,
        "a3",
        &[
            ("args", "err"),
            ("stdin", ""),
            ("expected", "#exit 0"),
            ("stderr", "oops\n"),
        ],
    );
    // other files and a directory of the same kind of name are not examples
    std::fs::write(dir.join("ex/README.md"), "notes").expect("a file");
    std::fs::create_dir(dir.join("ex/a4.args")).expect("a directory");
    let output = accept(&dir, &[], "");
    assert_eq!(
        out(&output),
        "ok a1\nok a2\nok a3\n3 passed, 0 failed\n",
        "{}",
        err(&output)
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(err(&output), "");
}

#[test]
fn a_failing_example_says_why_and_the_exit_code_is_1() {
    let dir = fresh_dir("accept-fail");
    full(&dir, "b1", "echo", "hi\n", "bye\n#exit 0\n");
    full(&dir, "b2", "exit3", "", "#exit 0\n");
    full(&dir, "b3", "echo", "same\n", "same\n#exit 0\n");
    full(&dir, "b4", "echo", "x\n", "y\n#exit 1\n");
    let output = accept(&dir, &[], "");
    assert_eq!(
        out(&output),
        concat!(
            "fail b1: standard output differs at line 1: expected \"bye\", got \"hi\"\n",
            "fail b2: exit code: expected 0, got 3\n",
            "ok b3\n",
            "fail b4: standard output differs at line 1: expected \"y\", got \"x\"; exit code: expected 1, got 0\n",
            "1 passed, 3 failed\n"
        ),
        "{}",
        err(&output)
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn the_standard_error_is_compared_when_the_example_has_it() {
    let dir = fresh_dir("accept-stderr");
    example(
        &dir,
        "c1",
        &[
            ("args", "err"),
            ("stdin", ""),
            ("expected", "#exit 0"),
            ("stderr", "other\n"),
        ],
    );
    // no `.stderr`: whatever the program writes there is not compared
    full(&dir, "c2", "err", "", "#exit 0");
    let output = accept(&dir, &[], "");
    assert_eq!(
        out(&output),
        concat!(
            "fail c1: standard error differs at line 1: expected \"other\", got \"oops\"\n",
            "ok c2\n",
            "1 passed, 1 failed\n"
        )
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_missing_file_fails_the_example_naming_it() {
    let dir = fresh_dir("accept-missing");
    example(&dir, "d1", &[("args", "echo"), ("expected", "#exit 0")]);
    example(&dir, "d2", &[("stdin", "")]);
    // only a `.stderr`: it still names an example
    example(&dir, "d3", &[("stderr", "")]);
    let output = accept(&dir, &[], "");
    assert_eq!(
        out(&output),
        concat!(
            "fail d1: missing d1.stdin\n",
            "fail d2: missing d2.args, d2.expected\n",
            "fail d3: missing d3.args, d3.stdin, d3.expected\n",
            "0 passed, 3 failed\n"
        )
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn an_expected_file_without_an_exit_line_fails_the_example() {
    let dir = fresh_dir("accept-bad-expected");
    full(&dir, "e1", "echo", "x\n", "x\n");
    full(&dir, "e2", "echo", "x\n", "x\n#exit three\n");
    full(&dir, "e3", "echo", "x\n", "x\n#exit 0\n");
    let output = accept(&dir, &[], "");
    assert_eq!(
        out(&output),
        concat!(
            "fail e1: e1.expected does not end with a line #exit <code>\n",
            "fail e2: e2.expected does not end with a line #exit <code>\n",
            "ok e3\n",
            "1 passed, 2 failed\n"
        )
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_program_that_does_not_end_in_time_is_stopped_and_fails() {
    let dir = fresh_dir("accept-timeout");
    full(&dir, "f1", "sleep", "", "#exit 0\n");
    let started = Instant::now();
    let output = accept(&dir, &["--timeout", "1"], "");
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(
        out(&output),
        "fail f1: did not end within 1 s\n0 passed, 1 failed\n"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_program_that_cannot_start_fails_the_example() {
    let dir = fresh_dir("accept-no-program");
    full(&dir, "g1", "echo", "", "#exit 0\n");
    let output = chidev(&dir, &["accept", "ex", "--", "./nonexistent.sh"], "");
    let text = out(&output);
    assert!(
        text.starts_with("fail g1: cannot start: ") && text.ends_with("\n0 passed, 1 failed\n"),
        "{text}"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn what_cannot_be_run_at_all_is_exit_code_2_with_nothing_on_standard_output() {
    let dir = fresh_dir("accept-usage");
    // no example in the directory
    let output = accept(&dir, &[], "");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(out(&output), "");
    assert!(
        err(&output).starts_with("chidev accept: no example in ex"),
        "{}",
        err(&output)
    );
    // a directory that is not there
    let output = chidev(&dir, &["accept", "nowhere", "--", "./prog.sh"], "");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(out(&output), "");
    assert!(
        err(&output).starts_with("chidev accept: "),
        "{}",
        err(&output)
    );
    // no program, a time limit of 0, no directory, and the options of the external-check face that is no more
    full(&dir, "h1", "echo", "", "#exit 0\n");
    for args in [
        &["accept", "ex"][..],
        &["accept", "ex", "--"][..],
        &["accept", "--timeout", "0", "ex", "--", "./prog.sh"][..],
        &["accept", "--", "./prog.sh"][..],
        &["accept", "--external", "--", "./prog.sh"][..],
        &[
            "accept",
            "--external",
            "--context",
            "x",
            "--harness",
            "acceptance",
            "--",
            "./prog.sh",
        ][..],
        &["accept", "--context", "x", "ex", "--", "./prog.sh"][..],
    ] {
        let output = chidev(&dir, args, "");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(out(&output), "", "{args:?}");
    }
}
