//! `chidev accept`, end to end through the binary: the examples of a directory run against a small program in plain
//! mode (a line for each, a count, the exit codes), and the examples a Harness of kind `examples` holds in the model
//! an external check is sent run as an external check (a reply naming the Clauses the failing Case cites), also
//! registered in a project and run by `chidev exec`. The program is a shell script written by the test; it does what
//! its first argument says.
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

/// `chidev accept --external` for the Harness `acceptance` of the Context `x` against `./prog.sh`, with `flags`
/// before the `--`. There is no directory: the examples are the ones the request holds.
fn external(dir: &Path, flags: &[&str], stdin: &str) -> Output {
    let mut args = vec![
        "accept",
        "--external",
        "--context",
        "x",
        "--harness",
        "acceptance",
    ];
    args.extend_from_slice(flags);
    args.extend(["--", "./prog.sh"]);
    chidev(dir, &args, stdin)
}

const C1: &str = "0190d8a0-0000-7000-8000-000000000001";
const C2: &str = "0190d8a0-0000-7000-8000-000000000002";

/// A Case of a Harness of examples as the view shows it, citing the Clauses `clauses` by id.
fn case(name: &str, args: &str, stdin: &str, expected: &str, clauses: &[&str]) -> Value {
    json!({
        "id": format!("case-{name}"), "name": name, "revision": 1,
        "example": {"args": args, "stdin": stdin, "expected": expected},
        "cites": {"items": [],
                  "clauses": clauses.iter().map(|id| json!({"id": id, "revision": 1})).collect::<Vec<_>>()}
    })
}

/// A request of an external check, protocol 2, whose Context `x` has a Contract of the Clauses `C1` and `C2` and a
/// Harness `acceptance` of kind `kind` with these cases.
fn request_of(kind: &str, cases: &[Value]) -> String {
    json!({
        "protocol": 2, "moment": "pin", "rules": ["ext.accept"], "read_at": 0, "params": {},
        "model": {"version": 5, "contexts": [{
            "id": "c", "name": "x", "basis": "contract", "uses": [],
            "contracts": [{"id": "k", "name": "k", "title": "t", "revision": 1, "clauses": [
                {"id": C1, "label": "C1", "revision": 1, "text": "one"},
                {"id": C2, "label": "C2", "revision": 1, "text": "two"}
            ]}],
            "interfaces": [],
            "harnesses": [{"id": "h", "name": "acceptance", "revision": 1, "kind": kind, "cases": cases}]
        }]}
    })
    .to_string()
}

fn request(cases: &[Value]) -> String {
    request_of("examples", cases)
}

#[test]
fn as_an_external_check_a_finding_is_about_each_clause_the_failing_case_cites() {
    let dir = fresh_dir("accept-external");
    let request = request(&[
        case("a1", "echo", "ok\n", "ok\n#exit 0\n", &[C1]),
        case("a2", "exit3", "", "#exit 0\n", &[C1, C2]),
        case("a3", "exit3", "", "#exit 0\n", &[C2]),
    ]);
    // the directory `ex` holds nothing of the kind: the examples are the model's
    let output = external(&dir, &[], &request);
    assert_eq!(output.status.code(), Some(0), "{}", err(&output));
    let text = out(&output);
    assert_eq!(text.lines().count(), 1, "{text}");
    let reply: Value = serde_json::from_str(&text).expect("one JSON document");
    assert_eq!(reply["protocol"], 2);
    let findings = reply["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 3, "{reply:#}");
    let seen: Vec<(Value, &str)> = findings
        .iter()
        .map(|f| {
            (
                f["subject"].clone(),
                f["message"].as_str().expect("a message"),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (
                json!({"kind": "clause", "id": C1}),
                "a2: exit code: expected 0, got 3"
            ),
            (
                json!({"kind": "clause", "id": C2}),
                "a2: exit code: expected 0, got 3"
            ),
            (
                json!({"kind": "clause", "id": C2}),
                "a3: exit code: expected 0, got 3"
            ),
        ]
    );
    for f in findings {
        assert_eq!(f["rule"], "ext.accept");
        assert!(f["level"].is_null());
    }
}

#[test]
fn as_an_external_check_passing_examples_give_no_findings() {
    let dir = fresh_dir("accept-external-ok");
    let request = request(&[case("a1", "echo", "ok\n", "ok\n#exit 0\n", &[C1])]);
    let output = external(&dir, &[], &request);
    assert_eq!(output.status.code(), Some(0), "{}", err(&output));
    assert_eq!(out(&output), "{\"protocol\":2,\"findings\":[]}\n");
}

#[test]
fn as_an_external_check_a_request_that_cannot_be_answered_is_exit_code_2() {
    let dir = fresh_dir("accept-external-bad");
    // an example that would take the whole limit: a request refused before the examples run comes back at once
    let sleeping = case("a2", "sleep", "", "#exit 0\n", &[C1]);
    let good = request(std::slice::from_ref(&sleeping));
    let two_rules = good.replace("[\"ext.accept\"]", "[\"a\",\"b\"]");
    let protocol_one = good.replace("\"protocol\":2", "\"protocol\":1");
    // no harnesses in the model: the check was registered to read the contracts only
    let no_harnesses = json!({
        "protocol": 2, "rules": ["ext.accept"],
        "model": {"version": 5, "contexts": [{"id": "c", "name": "x", "basis": "contract", "uses": [],
            "contracts": []}]}
    })
    .to_string();
    // a Harness that is not of examples
    let a_sim = request_of("sim", std::slice::from_ref(&sleeping));
    // a Clause a Case cites that the model sent lacks
    let foreign = request(&[case("a2", "sleep", "", "#exit 0\n", &["no-such-clause"])]);
    // no such Context or Harness
    let other_context = good.replace("\"name\":\"x\"", "\"name\":\"y\"");
    let other_harness = good.replace("\"name\":\"acceptance\"", "\"name\":\"other\"");
    for bad in [
        "this is not JSON",
        "",
        "{}",
        &two_rules,
        &protocol_one,
        &no_harnesses,
        &a_sim,
        &foreign,
        &other_context,
        &other_harness,
    ] {
        let started = std::time::Instant::now();
        let output = external(&dir, &["--timeout", "8"], bad);
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
    // and each says what is wrong
    let said = |request: &str| err(&external(&dir, &[], request));
    assert!(
        said(&no_harnesses).contains("has no harnesses"),
        "{}",
        said(&no_harnesses)
    );
    assert!(
        said(&a_sim).contains("is of sim, not examples"),
        "{}",
        said(&a_sim)
    );
    assert!(
        said(&foreign).contains("no-such-clause"),
        "{}",
        said(&foreign)
    );
    assert!(
        said(&other_context).contains("no context `x`"),
        "{}",
        said(&other_context)
    );
    assert!(
        said(&other_harness).contains("no harness `acceptance`"),
        "{}",
        said(&other_harness)
    );
}

#[test]
fn the_two_forms_of_accept_are_held_apart_by_the_arguments() {
    let dir = fresh_dir("accept-forms");
    full(&dir, "h1", "echo", "", "#exit 0\n");
    let none = request(&[]);
    for args in [
        // `--external` needs the Context and the Harness, and takes no directory
        &["accept", "--external", "--", "./prog.sh"][..],
        &["accept", "--external", "--context", "x", "--", "./prog.sh"][..],
        &[
            "accept",
            "--external",
            "--harness",
            "acceptance",
            "--",
            "./prog.sh",
        ][..],
        &[
            "accept",
            "--external",
            "--context",
            "x",
            "--harness",
            "acceptance",
            "ex",
            "--",
            "./prog.sh",
        ][..],
        // the Context and the Harness are for `--external`
        &["accept", "--context", "x", "ex", "--", "./prog.sh"][..],
        &["accept", "--harness", "acceptance", "ex", "--", "./prog.sh"][..],
        // and a directory is needed without it
        &["accept", "--", "./prog.sh"][..],
    ] {
        let output = chidev(&dir, args, &none);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(out(&output), "", "{args:?}");
    }
    // the plain form is as it was
    let output = accept(&dir, &[], "");
    assert_eq!(
        out(&output),
        "ok h1\n1 passed, 0 failed\n",
        "{}",
        err(&output)
    );
}

/// `chidev exec` of `ops` in `dir`, on a store inside it: the result line of each operation.
fn exec(dir: &Path, ops: &[Value]) -> Vec<Value> {
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(dir.join("ops.jsonl"), text).expect("a file");
    let output = chidev(dir, &["exec", "ops.jsonl", "--store", "chidev.db"], "");
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
    assert_eq!(results.len(), ops.len(), "{results:#?}");
    results
}

#[test]
fn registered_in_a_project_it_checks_the_examples_of_a_harness_and_a_changed_example_makes_the_result_stale()
 {
    let dir = fresh_dir("accept-registered");
    full(&dir, "a1", "echo", "ok\n", "ok\n#exit 0\n");
    full(&dir, "a2", "exit3", "", "#exit 0\n");
    std::fs::write(
        dir.join("chidev.toml"),
        format!(
            "[external.\"ext.accept\"]\ncommand = '{}'\n\
             args = [\"accept\", \"--external\", \"--context\", \"core\", \"--harness\", \"acceptance\", \"--\", \"./prog.sh\"]\n\
             moments = [\"check\"]\nreads = [\"contracts\", \"harnesses\"]\ngate = false\n",
            env!("CARGO_BIN_EXE_chidev")
        ),
    )
    .expect("a file");
    let cite = |label: &str| json!({"contract":"examples","label":label});
    let put = json!({"op":"harness_put","context":"core","name":"acceptance","kind":"examples",
        "builds":["prog"],"examples_dir":"ex","cases":[
            {"name":"a1","clauses":[cite("C1")]},
            {"name":"a2","clauses":[cite("C2")]}]});
    let results = exec(
        &dir,
        &[
            json!({"op":"init"}),
            json!({"op":"context_add","name":"core"}),
            json!({"op":"contract_put","context":"core","name":"examples","title":"Examples","clauses":[
                {"label":"C1","text":"the program repeats what it is given"},
                {"label":"C2","text":"the program can end with the code 3"}]}),
            json!({"op":"interface_put","context":"core","name":"prog","source":"run\n",
                   "items":[{"key":"run","lines":[1,1]}]}),
            put.clone(),
            json!({"op":"check_run"}),
            json!({"op":"status"}),
        ],
    );
    assert!(results.iter().all(|r| r["ok"] == true), "{results:#?}");
    let report = &results[5]["external"][0];
    assert_eq!(report["rule"], "ext.accept", "{results:#?}");
    assert!(report["failure"].is_null(), "{results:#?}");
    let findings = report["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{results:#?}");
    assert_eq!(findings[0]["subject"]["kind"], "clause");
    assert_eq!(findings[0]["message"], "a2: exit code: expected 0, got 3");
    assert_eq!(
        results[6]["external"][0]["state"], "current",
        "{results:#?}"
    );

    // the files of the directory are not what the check reads: changing one and not putting it again changes
    // nothing the check was run on
    full(&dir, "a2", "exit3", "", "#exit 3\n");
    let results = exec(&dir, &[json!({"op":"status"})]);
    assert_eq!(
        results[0]["external"][0]["state"], "current",
        "{results:#?}"
    );

    // putting the Harness again with the changed example is a new revision of what the check reads
    let results = exec(&dir, &[put, json!({"op":"status"})]);
    assert_eq!(results[0]["revision"], 2, "{results:#?}");
    assert_eq!(results[1]["external"][0]["state"], "stale", "{results:#?}");

    // run again, the example passes
    let results = exec(&dir, &[json!({"op":"check_run"}), json!({"op":"status"})]);
    let report = &results[0]["external"][0];
    assert!(
        report["findings"].as_array().expect("findings").is_empty(),
        "{results:#?}"
    );
    assert_eq!(
        results[1]["external"][0]["state"], "current",
        "{results:#?}"
    );
}
