//! `chidev check` and `chidev measure` end to end, through the binary: each runs the operation of the same name on a
//! project whose store is there, prints the line `chidev exec` prints for that operation, and says by its exit code
//! whether the project had an `error` finding or the run failed (1), the operation was refused (1) or nothing could
//! be run (2). The program the examples are run against is a shell script, as in `examples_measure.rs`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

/// The program as the Pin accepts it; the working tree's own `prog.sh` is a stub that does nothing.
const PROGRAM: &str = r#"# PINNED
case "$1" in
  echo) cat ;;
  exit3) cat > /dev/null; exit 3 ;;
esac
"#;

/// The settings of the project, with the command that builds the program.
fn settings(build: &str) -> String {
    format!(
        r#"[measure]
build = ["sh", "-c", "{build}"]
env = {{}}

[measure.examples.core]
program = ["sh", "{{tree}}/prog.sh"]
timeout = 10
budget = 120

[[measure.layout]]
context = "core"
interface = "prog"
path = "prog.sh"
"#
    )
}

/// A project directory of its own under cargo's temporary one, with settings and no store.
fn bare(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("prog.sh"), "exit 1\n").expect("prog.sh");
    std::fs::write(dir.join("chidev.toml"), settings("grep -q PINNED prog.sh"))
        .expect("chidev.toml");
    dir
}

/// `chidev` with `args` in `dir`.
fn chidev(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("chidev runs")
}

/// The one result line a command printed, and its exit code.
fn line_of(out: &Output) -> (Value, i32) {
    let text = String::from_utf8(out.stdout.clone()).expect("utf-8");
    assert_eq!(text.lines().count(), 1, "one line: {text:?}");
    (
        serde_json::from_str(&text).expect("a JSON object"),
        out.status.code().expect("an exit code"),
    )
}

/// The store of a project that settles nothing about it.
fn store(dir: &Path) -> PathBuf {
    dir.join(".chidev").join("chidev.db")
}

/// `chidev exec` of `ops` in `dir`, on the store the project has by default: the result line of each operation.
fn run_ops(dir: &Path, ops: &[Value]) -> Vec<Value> {
    let file = dir.join("ops.jsonl");
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(&file, text).expect("a file");
    let out = chidev(dir, &["exec", file.to_str().expect("utf-8")]);
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect()
}

/// As [`run_ops`], and every operation succeeded.
fn exec(dir: &Path, ops: &[Value]) -> Vec<Value> {
    let lines = run_ops(dir, ops);
    assert_eq!(lines.len(), ops.len(), "{lines:#?}");
    assert!(lines.iter().all(|r| r["ok"] == true), "{lines:#?}");
    lines
}

fn rules_of(result: &Value) -> Vec<&str> {
    result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule"].as_str().expect("a rule"))
        .collect()
}

/// The ops that put the model: a Contract of two Clauses, the Interface of the program, a tests Harness that cites
/// its Item (which is what lets the Interface be pinned), and the Pin.
fn model() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"context_add","name":"other"}),
        json!({"op":"contract_put","context":"core","name":"lines","title":"Lines","clauses":[
            {"label":"C1","text":"the program repeats what it is given"},
            {"label":"C2","text":"the program can end with the code 3"}]}),
        json!({"op":"interface_put","context":"core","name":"prog","source":"run\n",
               "items":[{"key":"run","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"prog-tests","source":"tests\n","cases":[
            {"name":"runs","lines":[1,1],"items":[{"interface":"prog","key":"run"}]}]}),
        json!({"op":"pin","context":"core","interface":"prog","code":PROGRAM}),
    ]
}

/// The Harness of examples: `a1` repeats its input (cites C1), `a2` ends with the code 3 (cites C2) and `expected`
/// is what it is expected to end with.
fn examples_put(a2_expected: &str) -> Value {
    json!({"op":"harness_put","context":"core","name":"acceptance","kind":"examples","builds":["prog"],"cases":[
        {"name":"a1","clauses":[{"contract":"lines","label":"C1"}],
         "example":{"args":"echo","stdin":"hi\n","expected":"hi\n#exit 0\n"}},
        {"name":"a2","clauses":[{"contract":"lines","label":"C2"}],
         "example":{"args":"exit3","stdin":"","expected":a2_expected}}]})
}

/// A project with the model and the examples, where `a2` expects the code 0 and the program ends with 3: the first
/// run of the examples has a failing example, which is an `error` finding.
fn project(name: &str) -> PathBuf {
    let dir = bare(name);
    let mut setup = model();
    setup.push(examples_put("#exit 0\n"));
    exec(&dir, &setup);
    dir
}

const MEASURE: [&str; 6] = [
    "measure",
    "examples",
    "--context",
    "core",
    "--harness",
    "acceptance",
];

#[test]
fn check_prints_what_exec_prints_for_the_operation_and_exits_by_its_error_findings() {
    let dir = project("commands-check");

    // before a run, the examples were never run: warnings and notes, no error
    let (all, code) = line_of(&chidev(&dir, &["check"]));
    assert_eq!(all["ok"], true, "{all:#?}");
    assert!(rules_of(&all).contains(&"examples.unrun"), "{all:#?}");
    assert!(all.get("outside").is_none());
    assert_eq!(code, 0, "{all:#?}");
    assert_eq!(all, exec(&dir, &[json!({"op":"check"})])[0]);

    // a run whose example fails is an error finding
    let (done, code) = line_of(&chidev(&dir, &MEASURE));
    assert_eq!(code, 0, "{done:#?}");
    let (all, code) = line_of(&chidev(&dir, &["check"]));
    assert_eq!(code, 1, "{all:#?}");
    assert!(rules_of(&all).contains(&"examples.failed"), "{all:#?}");
    assert_eq!(all, exec(&dir, &[json!({"op":"check"})])[0]);

    // by id
    let (by_id, code) = line_of(&chidev(&dir, &["check", "examples.failed"]));
    assert_eq!(code, 1, "{by_id:#?}");
    assert_eq!(rules_of(&by_id), ["examples.failed"], "{by_id:#?}");
    assert_eq!(
        by_id["outside"].as_u64().expect("outside") as usize,
        rules_of(&all).len() - 1,
        "{by_id:#?}"
    );
    assert_eq!(
        by_id,
        exec(&dir, &[json!({"op":"check","rules":["examples.failed"]})])[0]
    );

    // by prefix, whole segments, and several at once
    let (family, code) = line_of(&chidev(&dir, &["check", "examples."]));
    assert_eq!(code, 1);
    assert!(
        rules_of(&family).iter().all(|r| r.starts_with("examples.")),
        "{family:#?}"
    );
    assert!(rules_of(&family).contains(&"examples.failed"));
    let (two, _) = line_of(&chidev(&dir, &["check", "examples.failed", "context."]));
    let rules = rules_of(&two);
    assert!(rules.contains(&"examples.failed"), "{two:#?}");
    assert!(
        rules
            .iter()
            .all(|r| *r == "examples.failed" || r.starts_with("context.")),
        "{two:#?}"
    );
    assert_eq!(
        two,
        exec(
            &dir,
            &[json!({"op":"check","rules":["examples.failed","context."]})]
        )[0]
    );

    // with a context, and with a context and rules
    let (in_core, code) = line_of(&chidev(&dir, &["check", "--context", "core"]));
    assert_eq!(code, 1);
    assert!(in_core["outside"].as_u64().expect("outside") > 0);
    assert_eq!(
        in_core,
        exec(&dir, &[json!({"op":"check","context":"core"})])[0]
    );
    let (narrow, code) = line_of(&chidev(&dir, &["check", "examples.", "--context", "other"]));
    // nothing of the family is about the other Context
    assert_eq!(code, 0, "{narrow:#?}");
    assert!(narrow["findings"].as_array().expect("findings").is_empty());
    assert_eq!(
        narrow,
        exec(
            &dir,
            &[json!({"op":"check","context":"other","rules":["examples."]})]
        )[0]
    );
}

#[test]
fn check_exits_0_when_no_finding_is_an_error() {
    let dir = project("commands-check-clean");
    // the program does end with 3
    exec(&dir, &[examples_put("#exit 3\n")]);
    let (done, code) = line_of(&chidev(&dir, &MEASURE));
    assert_eq!(
        (done["outcome"].as_str(), code),
        (Some("ok"), 0),
        "{done:#?}"
    );
    let (all, code) = line_of(&chidev(&dir, &["check"]));
    assert_eq!(code, 0, "{all:#?}");
    assert!(
        all["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .all(|f| f["level"] != "error"),
        "{all:#?}"
    );
    let (family, code) = line_of(&chidev(&dir, &["check", "examples."]));
    assert_eq!(code, 0);
    assert!(family["findings"].as_array().expect("findings").is_empty());
}

#[test]
fn an_error_finding_a_change_request_deals_with_is_not_owed_and_does_not_fail_the_command() {
    let dir = project("commands-check-addressed");
    let (done, _) = line_of(&chidev(&dir, &MEASURE));
    assert_eq!(done["outcome"], "ok");
    let (open, code) = line_of(&chidev(&dir, &["check", "examples.failed"]));
    assert_eq!(code, 1, "{open:#?}");
    assert_eq!(open["findings"][0]["state"], "open");

    // a change request that addresses the finding is dealing with it
    exec(
        &dir,
        &[
            json!({"op":"cr_open","title":"Fix a2","intent":"change_behavior","addresses":[
            {"rule":"examples.failed","subject":{"clause":
                {"context":"core","contract":"lines","label":"C2"}}}]}),
        ],
    );
    let (addressed, code) = line_of(&chidev(&dir, &["check", "examples.failed"]));
    assert_eq!(addressed["findings"][0]["level"], "error", "{addressed:#?}");
    assert_eq!(addressed["findings"][0]["state"], "addressed");
    assert_eq!(code, 0, "{addressed:#?}");
}

#[test]
fn check_refuses_what_exec_refuses_with_the_exit_code_1() {
    let dir = project("commands-check-refused");
    let before = std::fs::read(store(&dir)).expect("the store");

    // a rule that is not a check: nothing is run, and the names near it are said
    let (unknown, code) = line_of(&chidev(&dir, &["check", "examples.faild"]));
    assert_eq!(code, 1, "{unknown:#?}");
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["diagnostics"][0]["code"], "input.unknown-name");
    assert_eq!(
        unknown["diagnostics"][0]["did_you_mean"][0], "examples.failed",
        "{unknown:#?}"
    );
    assert_eq!(
        unknown,
        run_ops(&dir, &[json!({"op":"check","rules":["examples.faild"]})])[0]
    );
    // a part of a segment is no family
    let (part, code) = line_of(&chidev(&dir, &["check", "exam."]));
    assert_eq!((part["ok"].clone(), code), (json!(false), 1), "{part:#?}");
    // a family is written with its dot, as in a policy file: the bare word is an id, and none has it
    let (bare, code) = line_of(&chidev(&dir, &["check", "examples"]));
    assert_eq!((bare["ok"].clone(), code), (json!(false), 1), "{bare:#?}");
    assert_eq!(
        bare["diagnostics"][0]["did_you_mean"][0], "examples.",
        "{bare:#?}"
    );

    // a Context that is not there
    let (no_context, code) = line_of(&chidev(&dir, &["check", "--context", "nowhere"]));
    assert_eq!((no_context["ok"].clone(), code), (json!(false), 1));
    assert_eq!(
        no_context,
        run_ops(&dir, &[json!({"op":"check","context":"nowhere"})])[0]
    );

    // a selector that is not well formed
    let (bad, code) = line_of(&chidev(&dir, &["check", "Examples."]));
    assert_eq!((bad["ok"].clone(), code), (json!(false), 1), "{bad:#?}");

    // none of it kept anything
    assert_eq!(std::fs::read(store(&dir)).expect("the store"), before);
}

#[test]
fn check_and_measure_open_a_store_that_is_there_and_make_none() {
    let dir = bare("commands-no-store");
    for args in [
        vec!["check"],
        vec!["check", "--store", "elsewhere/chidev.db"],
        vec![
            "measure",
            "examples",
            "--context",
            "core",
            "--harness",
            "acceptance",
        ],
        vec![
            "measure",
            "mutate",
            "--context",
            "core",
            "--interface",
            "prog",
            "--store",
            "elsewhere/chidev.db",
        ],
    ] {
        let out = chidev(&dir, &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        let said = String::from_utf8(out.stderr).expect("utf-8");
        assert!(said.contains("there is no store file"), "{args:?}: {said}");
        // the path is said once
        assert_eq!(said.matches("chidev.db").count(), 1, "{args:?}: {said}");
    }
    // neither a store nor a directory for one
    assert!(!dir.join(".chidev").exists());
    assert!(!dir.join("elsewhere").exists());

    // `exec` makes it, as it did
    exec(&dir, &[json!({"op":"init"})]);
    assert!(store(&dir).exists());
    let (checked, code) = line_of(&chidev(&dir, &["check"]));
    assert_eq!((checked["ok"].clone(), code), (json!(true), 0));
    // and a store given by the flag is opened where it is
    std::fs::create_dir_all(dir.join("elsewhere")).expect("a directory");
    std::fs::copy(store(&dir), dir.join("elsewhere/chidev.db")).expect("a copy");
    let (checked, code) = line_of(&chidev(&dir, &["check", "--store", "elsewhere/chidev.db"]));
    assert_eq!((checked["ok"].clone(), code), (json!(true), 0));
}

#[test]
fn check_and_measure_stop_with_2_when_the_settings_cannot_be_read() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("commands-bad-settings");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("chidev.toml"), "[measure\n").expect("chidev.toml");
    for args in [&["check"][..], &MEASURE[..]] {
        let out = chidev(&dir, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty());
    }
    assert!(!dir.join(".chidev").exists());
}

#[test]
fn measure_examples_records_a_run_and_check_reports_on_it() {
    let dir = project("commands-measure");
    let (before, _) = line_of(&chidev(&dir, &["check", "examples."]));
    assert_eq!(rules_of(&before), ["examples.unrun"], "{before:#?}");

    let (done, code) = line_of(&chidev(&dir, &MEASURE));
    assert_eq!(code, 0, "{done:#?}");
    assert_eq!(done["ok"], true);
    assert_eq!(done["op"], "measure");
    assert_eq!(done["line"], 1);
    assert_eq!(done["kind"], "examples");
    assert_eq!(done["outcome"], "ok");
    let examples = done["examples"].as_array().expect("examples");
    assert_eq!(examples.len(), 2, "{done:#?}");
    assert_eq!(examples[0]["passed"], true);
    assert_eq!(examples[1]["passed"], false);

    // the run was kept, so the check judges it
    let (after, code) = line_of(&chidev(&dir, &["check", "examples."]));
    assert_eq!(code, 1, "{after:#?}");
    assert_eq!(rules_of(&after), ["examples.failed"], "{after:#?}");
    let status = exec(&dir, &[json!({"op":"status"})]);
    let standing = status[0]["measurements"]
        .as_array()
        .expect("measurements")
        .iter()
        .find(|s| s["kind"] == "examples")
        .expect("the run is listed");
    assert_eq!(standing["stamp"], done["stamp"]);
    assert_eq!(standing["state"], "current");
}

#[test]
fn measure_prints_the_line_exec_prints_for_the_operation() {
    let dir = project("commands-measure-same");
    // the line of a run has the id of the run, which differs, so the lines are compared without it and what changes
    // with the time
    let strip = |mut line: Value| {
        for key in ["id", "evidence"] {
            line.as_object_mut().expect("an object").remove(key);
        }
        line
    };
    let by_command = line_of(&chidev(&dir, &MEASURE)).0;
    let by_exec = exec(
        &dir,
        &[json!({"op":"measure","kind":"examples","context":"core","harness":"acceptance"})],
    )
    .remove(0);
    assert_eq!(strip(by_command), strip(by_exec));

    // a refusal is the same line, and exit 1
    for (args, op) in [
        (
            vec![
                "measure",
                "examples",
                "--context",
                "core",
                "--harness",
                "nowhere",
            ],
            json!({"op":"measure","kind":"examples","context":"core","harness":"nowhere"}),
        ),
        (
            vec![
                "measure",
                "mutate",
                "--context",
                "core",
                "--interface",
                "nowhere",
            ],
            json!({"op":"measure","kind":"mutate","context":"core","interface":"nowhere"}),
        ),
        (
            vec![
                "measure",
                "simulate",
                "--context",
                "core",
                "--harness",
                "acceptance",
            ],
            json!({"op":"measure","kind":"simulate","context":"core","harness":"acceptance"}),
        ),
        (
            vec![
                "measure",
                "examples",
                "--context",
                "nowhere",
                "--harness",
                "acceptance",
            ],
            json!({"op":"measure","kind":"examples","context":"nowhere","harness":"acceptance"}),
        ),
    ] {
        let (line, code) = line_of(&chidev(&dir, &args));
        assert_eq!((line["ok"].clone(), code), (json!(false), 1), "{args:?}");
        assert_eq!(line, run_ops(&dir, &[op])[0], "{args:?}");
    }
}

#[test]
fn measure_exits_1_when_the_run_it_recorded_failed() {
    let dir = project("commands-measure-failed");
    // a build that fails is a failed run (`no_build`), recorded
    std::fs::write(dir.join("chidev.toml"), settings("exit 1")).expect("chidev.toml");
    let (done, code) = line_of(&chidev(&dir, &MEASURE));
    assert_eq!(code, 1, "{done:#?}");
    assert_eq!(done["ok"], true);
    assert_eq!(done["outcome"], "failed", "{done:#?}");
    // it was kept
    let status = exec(&dir, &[json!({"op":"status"})]);
    assert_eq!(
        status[0]["measurements"]
            .as_array()
            .expect("measurements")
            .iter()
            .filter(|s| s["kind"] == "examples" && s["outcome"] == "failed")
            .count(),
        1,
        "{:#?}",
        status[0]
    );
}

#[test]
fn a_measure_of_a_kind_takes_the_asset_of_that_kind_and_no_other() {
    let dir = project("commands-measure-args");
    let before = std::fs::read(store(&dir)).expect("the store");
    for args in [
        // the wrong asset for the kind
        vec!["measure", "mutate", "--context", "core", "--harness", "x"],
        vec![
            "measure",
            "examples",
            "--context",
            "core",
            "--interface",
            "prog",
        ],
        vec![
            "measure",
            "simulate",
            "--context",
            "core",
            "--interface",
            "prog",
        ],
        // both, and neither, and no context
        vec![
            "measure",
            "mutate",
            "--context",
            "core",
            "--interface",
            "prog",
            "--harness",
            "x",
        ],
        vec!["measure", "mutate", "--context", "core"],
        vec!["measure", "examples", "--harness", "acceptance"],
        // no kind, and a kind there is not
        vec!["measure"],
        vec!["measure", "everything", "--context", "core"],
    ] {
        let out = chidev(&dir, &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(!out.stderr.is_empty(), "{args:?}");
    }
    // a clap refusal runs nothing
    assert_eq!(std::fs::read(store(&dir)).expect("the store"), before);
    // and `check` takes no `--harness`
    assert_eq!(
        chidev(&dir, &["check", "--harness", "x"]).status.code(),
        Some(2)
    );
}
