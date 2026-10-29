//! `chidev accept`, end to end through the binary: the examples of a directory run against a small program, in plain
//! mode (a line for each, a count, the exit codes) and as an external check (a reply naming the Clause whose text names
//! the files of a failing example), and registered in a project and run by `chidev exec`. The program is a shell
//! script written by the test; it does what its first argument says.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

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
    // no program, a time limit of 0
    full(&dir, "h1", "echo", "", "#exit 0\n");
    for args in [
        &["accept", "ex"][..],
        &["accept", "ex", "--"][..],
        &["accept", "--timeout", "0", "ex", "--", "./prog.sh"][..],
        &["accept", "--", "./prog.sh"][..],
    ] {
        let output = chidev(&dir, args, "");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(out(&output), "", "{args:?}");
    }
}

/// A request of an external check, protocol 2, whose one Context has one Contract with the Clause `id` saying `text`.
fn request(id: &str, text: &str) -> String {
    json!({
        "protocol": 2, "moment": "pin", "rules": ["ext.accept"], "read_at": 0, "params": {},
        "model": {"version": 4, "contexts": [{
            "id": "c", "name": "x", "basis": "contract", "uses": [],
            "contracts": [{"id": "k", "name": "k", "title": "t", "revision": 1, "clauses": [
                {"id": id, "label": "C1", "revision": 1, "text": text}
            ]}],
            "interfaces": [], "harnesses": []
        }]}
    })
    .to_string()
}

const CLAUSE: &str = "0190d8a0-0000-7000-8000-000000000001";

#[test]
fn as_an_external_check_a_finding_is_about_the_clause_that_names_the_files() {
    let dir = fresh_dir("accept-external");
    full(&dir, "a1", "echo", "ok\n", "ok\n#exit 0\n");
    full(&dir, "a2", "exit3", "", "#exit 0\n");
    full(&dir, "a3", "exit3", "", "#exit 0\n");
    let request = request(CLAUSE, "the input `ex/a2.stdin` gives `ex/a2.expected`");
    let output = accept(&dir, &["--external"], &request);
    assert_eq!(output.status.code(), Some(0), "{}", err(&output));
    let text = out(&output);
    assert_eq!(text.lines().count(), 1, "{text}");
    let reply: Value = serde_json::from_str(&text).expect("one JSON document");
    assert_eq!(reply["protocol"], 2);
    let findings = reply["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 2, "{reply:#}");
    assert_eq!(findings[0]["rule"], "ext.accept");
    assert_eq!(
        findings[0]["subject"],
        json!({"kind": "clause", "id": CLAUSE})
    );
    assert_eq!(findings[0]["message"], "a2: exit code: expected 0, got 3");
    assert!(findings[0]["level"].is_null());
    // a3 is named by no Clause: the finding is about the project
    assert_eq!(findings[1]["rule"], "ext.accept");
    assert!(findings[1]["subject"].is_null());
    assert_eq!(
        findings[1]["message"],
        "a3: exit code: expected 0, got 3 (no clause names ex/a3.expected)"
    );
}

#[test]
fn as_an_external_check_passing_examples_give_no_findings() {
    let dir = fresh_dir("accept-external-ok");
    full(&dir, "a1", "echo", "ok\n", "ok\n#exit 0\n");
    let request = request(CLAUSE, "the input `ex/a1.stdin` gives `ex/a1.expected`");
    let output = accept(&dir, &["--external"], &request);
    assert_eq!(output.status.code(), Some(0), "{}", err(&output));
    assert_eq!(out(&output), "{\"protocol\":2,\"findings\":[]}\n");
}

#[test]
fn as_an_external_check_a_request_that_cannot_be_answered_is_exit_code_2() {
    let dir = fresh_dir("accept-external-bad");
    full(&dir, "a1", "echo", "ok\n", "ok\n#exit 0\n");
    // an example that would take the whole limit: a request refused before the examples run comes back at once
    full(&dir, "a2", "sleep", "", "#exit 0\n");
    let two_rules = request(CLAUSE, "x").replace("[\"ext.accept\"]", "[\"a\",\"b\"]");
    let protocol_one = request(CLAUSE, "x").replace("\"protocol\":2", "\"protocol\":1");
    for bad in ["this is not JSON", "", "{}", &two_rules, &protocol_one] {
        let started = std::time::Instant::now();
        let output = accept(&dir, &["--external", "--timeout", "8"], bad);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "no example runs for {bad}"
        );
        assert_eq!(output.status.code(), Some(2), "{bad}");
        assert_eq!(out(&output), "", "{bad}");
        assert!(
            err(&output).starts_with("chidev accept: "),
            "{}",
            err(&output)
        );
    }
}

#[test]
fn registered_in_a_project_it_checks_the_examples_for_chidev_exec() {
    let dir = fresh_dir("accept-registered");
    full(&dir, "a1", "echo", "ok\n", "ok\n#exit 0\n");
    full(&dir, "a2", "exit3", "", "#exit 0\n");
    std::fs::write(
        dir.join("chidev.toml"),
        format!(
            "[external.\"ext.accept\"]\ncommand = '{}'\nargs = [\"accept\", \"--external\", \"ex\", \"--\", \"./prog.sh\"]\n\
             moments = [\"check\"]\nreads = [\"contracts\"]\ngate = false\n",
            env!("CARGO_BIN_EXE_chidev")
        ),
    )
    .expect("a file");
    let ops = [
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"contract_put","context":"core","name":"examples","title":"Examples","clauses":[
            {"label":"C1","text":"the input `ex/a2.stdin` gives `ex/a2.expected`"}]}),
        json!({"op":"check_run"}),
    ];
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(dir.join("ops.jsonl"), text).expect("a file");
    let output = chidev(&dir, &["exec", "ops.jsonl", "--store", "chidev.db"], "");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        out(&output),
        err(&output)
    );
    let results: Vec<Value> = out(&output)
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    assert_eq!(results.len(), 4, "{results:#?}");
    let report = &results[3]["external"][0];
    assert_eq!(report["rule"], "ext.accept", "{results:#?}");
    assert!(report["failure"].is_null(), "{results:#?}");
    let findings = report["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{results:#?}");
    assert_eq!(findings[0]["subject"]["kind"], "clause");
    assert_eq!(findings[0]["message"], "a2: exit code: expected 0, got 3");
}
