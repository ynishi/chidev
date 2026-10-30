//! `measure` of kind `examples` end to end, through the `chidev` binary: the examples a Harness of examples holds are
//! run against a shell script that stands for the program, built from the code the Pin accepted, and the run is kept,
//! judged by the `examples.*` checks and listed by `status`. The program answers by its first argument, so nothing is
//! compiled and the build command only looks for the marker the pinned code has.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// The program as the Pin accepts it; the working tree's own `prog.sh` is a stub that does nothing.
const PROGRAM: &str = r#"# PINNED
case "$1" in
  echo) cat ;;
  exit3) cat > /dev/null; exit 3 ;;
esac
"#;

/// A project directory of its own under cargo's temporary one.
fn project(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("prog.sh"), "exit 1\n").expect("prog.sh");
    std::fs::write(
        dir.join("chidev.toml"),
        r#"[measure]
build = ["sh", "-c", "grep -q PINNED prog.sh"]
env = {}

[measure.examples.core]
program = ["sh", "{tree}/prog.sh"]
timeout = 10
budget = 120

[[measure.layout]]
context = "core"
interface = "prog"
path = "prog.sh"
"#,
    )
    .expect("chidev.toml");
    dir
}

/// `chidev exec` of `ops` in `dir`, on a store inside it: the result line of each operation.
fn run_ops(dir: &Path, ops: &[Value]) -> Vec<Value> {
    let file = dir.join("ops.jsonl");
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(&file, text).expect("a file");
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(dir)
        .arg("exec")
        .arg(&file)
        .arg("--store")
        .arg(dir.join("chidev.db"))
        .output()
        .expect("chidev runs");
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

fn findings<'a>(result: &'a Value, rule: &str) -> Vec<&'a Value> {
    result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"] == rule)
        .collect()
}

/// The ops that put the model: a Contract of two Clauses, the Interface of the program, a tests Harness that cites
/// its Item (which is what lets the Interface be pinned), and the Pin.
fn model() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
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

fn measure() -> Value {
    json!({"op":"measure","kind":"examples","context":"core","harness":"acceptance"})
}

#[test]
fn the_examples_are_run_against_the_pinned_program_and_kept_judged_and_listed() {
    let dir = project("examples-e2e");
    let mut setup = model();
    // a2 expects the code 0, and the program ends with 3
    setup.push(examples_put("#exit 0\n"));
    exec(&dir, &setup);

    // before a run the Harness was never run
    let before = exec(&dir, &[json!({"op":"check"})]);
    assert_eq!(
        findings(&before[0], "examples.unrun").len(),
        1,
        "{before:#?}"
    );

    let done = exec(&dir, &[measure()]);
    let m = &done[0];
    assert_eq!(m["kind"], "examples");
    assert_eq!(m["outcome"], "ok", "{m:#?}");
    let examples = m["examples"].as_array().expect("examples");
    assert_eq!(examples.len(), 2, "{m:#?}");
    assert_eq!(examples[0]["example"], "a1");
    assert_eq!(examples[0]["passed"], true);
    assert_eq!(examples[1]["example"], "a2");
    assert_eq!(examples[1]["passed"], false);
    assert_eq!(
        examples[1]["reasons"],
        json!(["exit code: got 3"]),
        "{m:#?}"
    );
    assert_eq!(m["evidence"][0]["step"], "examples-build");
    assert!(m["cases"].is_null() && m["reruns"].is_null());

    // the project was not touched, and the run's directory is gone
    assert_eq!(
        std::fs::read_to_string(dir.join("prog.sh")).expect("prog.sh"),
        "exit 1\n"
    );
    let leftovers = std::fs::read_dir(dir.join(".chidev/measure"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);

    // the check: the failing example is a finding about the Clause its Case cites, and nothing else is said
    let after = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let failed = findings(&after[0], "examples.failed");
    assert_eq!(failed.len(), 1, "{:#?}", after[0]);
    assert_eq!(failed[0]["level"], "error");
    assert_eq!(failed[0]["about"], "lines C2");
    assert_eq!(
        failed[0]["message"],
        "example `a2` failed: exit code: got 3"
    );
    for rule in ["examples.unrun", "examples.stale", "examples.error"] {
        assert!(findings(&after[0], rule).is_empty(), "{rule}");
    }
    // the status lists the run
    let standing = after[1]["measurements"]
        .as_array()
        .expect("measurements")
        .iter()
        .find(|s| s["kind"] == "examples")
        .expect("the run is listed");
    assert_eq!(standing["harness"], "acceptance");
    assert_eq!(standing["state"], "current");
    assert_eq!(standing["outcome"], "ok");
    assert_eq!(standing["examples"], json!({"passed": 1, "failed": 1}));
    assert!(standing["sim"].is_null() && standing["sites"].is_null());
    assert_eq!(standing["stamp"], m["stamp"]);

    // changing the expected output of a2 (the program does end with 3) makes the run stale, and says what moved
    exec(&dir, &[examples_put("#exit 3\n")]);
    let moved = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let stale = findings(&moved[0], "examples.stale");
    assert_eq!(stale.len(), 1, "{:#?}", moved[0]);
    assert_eq!(
        stale[0]["message"],
        "the latest run of the examples is stale: examples"
    );
    // a stale run is not judged for its failures
    assert!(findings(&moved[0], "examples.failed").is_empty());
    let standing = moved[1]["measurements"]
        .as_array()
        .expect("measurements")
        .iter()
        .find(|s| s["kind"] == "examples")
        .expect("the run is listed");
    assert_eq!(standing["state"], "stale");
    assert_eq!(standing["moved"], json!([{"part": "examples"}]));

    // run again: every example passes and nothing is said
    let again = exec(
        &dir,
        &[measure(), json!({"op":"check"}), json!({"op":"status"})],
    );
    assert!(
        again[0]["examples"]
            .as_array()
            .expect("examples")
            .iter()
            .all(|e| e["passed"] == true),
        "{:#?}",
        again[0]
    );
    for rule in [
        "examples.unrun",
        "examples.stale",
        "examples.error",
        "examples.failed",
    ] {
        assert!(findings(&again[1], rule).is_empty(), "{rule}");
    }
    assert_ne!(again[0]["stamp"], m["stamp"]);
    let standing = again[2]["measurements"]
        .as_array()
        .expect("measurements")
        .iter()
        .find(|s| s["kind"] == "examples")
        .expect("the run is listed");
    assert_eq!(standing["state"], "current");
    assert_eq!(standing["examples"], json!({"passed": 2, "failed": 0}));
}

#[test]
fn pinning_the_code_again_makes_the_run_stale_by_the_built_code_and_the_pin_is_not_refused() {
    let dir = project("examples-e2e-pin");
    let mut setup = model();
    setup.push(examples_put("#exit 0\n"));
    setup.push(measure());
    exec(&dir, &setup);
    let pinned = exec(
        &dir,
        &[
            json!({"op":"pin","context":"core","interface":"prog","code":format!("{PROGRAM}# again\n")}),
            json!({"op":"status"}),
        ],
    );
    // the pin reports the run as stale and is not refused for it
    assert_eq!(
        findings(&pinned[0], "examples.stale").len(),
        1,
        "{:#?}",
        pinned[0]
    );
    let standing = pinned[1]["measurements"]
        .as_array()
        .expect("measurements")
        .iter()
        .find(|s| s["kind"] == "examples")
        .expect("the run is listed");
    assert_eq!(standing["state"], "stale");
    assert_eq!(
        standing["moved"],
        json!([{"part": "built-code", "name": "prog"}])
    );
}

#[test]
fn a_measure_of_examples_takes_a_harness_of_examples_and_nothing_else() {
    let dir = project("examples-e2e-refused");
    let mut setup = model();
    setup.push(examples_put("#exit 0\n"));
    exec(&dir, &setup);
    let refused = run_ops(
        &dir,
        &[
            json!({"op":"measure","kind":"examples","context":"core","interface":"prog"}),
            json!({"op":"measure","kind":"examples","context":"core","harness":"prog-tests"}),
            json!({"op":"measure","kind":"examples","context":"core","harness":"nowhere"}),
        ],
    );
    assert_eq!(refused.len(), 3, "{refused:#?}");
    assert!(refused.iter().all(|r| r["ok"] == false), "{refused:#?}");
    // nothing was kept
    let status = exec(&dir, &[json!({"op":"status"})]);
    assert!(
        status[0]["measurements"]
            .as_array()
            .expect("measurements")
            .is_empty()
    );
}
