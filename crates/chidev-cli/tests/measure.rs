//! Mutation testing end to end, through the `chidev` binary and a real cargo crate: the Contract, Interface and
//! Harness are put, the code is pinned, `measure` runs the pinned code against the Harness with every mutant the
//! bundled Rust provider makes, `check` shows the mutant the Harness missed, and changing the Harness makes the
//! measurement stale.
//!
//! The Harness leaves the boundary of `fits` (`used == limit`) unchecked, so the mutant `<=` to `<` survives; the
//! mutant `-` to `+` of `spare` is caught by its case. The working tree holds a *different* body of the code
//! (a stub that fails the tests): the measure has to run the code the Pin accepted, not the file in the tree.
//!
//! It builds and tests a crate three times over (the baseline and one pass per mutant), so it needs cargo and takes
//! a few seconds with a warm toolchain.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

const PINNED: &str = include_str!("fixtures/measure/stock.rs");
const HARNESS: &str = include_str!("fixtures/measure/stock_tests.rs");

fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

/// `chidev exec` with `dir` as the project's root, on a store inside it.
fn exec(dir: &Path, ops: &[Value]) -> Vec<Value> {
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
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let lines: Vec<Value> = String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect();
    assert!(
        lines.iter().all(|r| r["ok"] == true),
        "{lines:#?}\nstderr: {stderr}"
    );
    lines
}

fn project(name: &str) -> PathBuf {
    project_with(name, "")
}

/// As [`project`], with `extra` lines (TOML) added to the `[measure]` table.
fn project_with(name: &str, extra: &str) -> PathBuf {
    let dir = fresh_dir(name);
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"inventory\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .expect("Cargo.toml");
    std::fs::write(dir.join("src/lib.rs"), "pub mod stock;\n").expect("lib.rs");
    // The working tree's body of the code is a stub the tests would fail on: the measure must not run it.
    std::fs::write(
        dir.join("src/stock.rs"),
        "pub fn fits(_: u32, _: u32) -> bool { false }\npub fn spare(_: u32, _: u32) -> u32 { 0 }\n",
    )
    .expect("stock.rs");
    std::fs::write(
        dir.join("checks.toml"),
        format!(
            r#"[measure]
build = ["cargo", "build", "--quiet", "--tests", "--offline"]
test = ["cargo", "test", "--quiet", "--offline"]
baseline_timeout = 300
timeout = 120
budget = 900
{extra}

[measure.mutants]
rust = [{bin:?}, "provider", "mutants-rust"]

[[measure.layout]]
context = "inventory"
interface = "stock"
lang = "rust"
path = "src/stock.rs"

[[measure.layout]]
context = "inventory"
harness = "stock-tests"
path = "tests/stock.rs"
"#,
            bin = env!("CARGO_BIN_EXE_chidev"),
            extra = extra
        ),
    )
    .expect("checks.toml");
    dir
}

fn harness_put(source: &str) -> Value {
    json!({"op":"harness_put","context":"inventory","name":"stock-tests","source":source,"cases":[
        {"name":"a_load_under_the_limit_fits","lines":[3,6],
         "items":[{"interface":"stock","signature":"fn fits(used: u32, limit: u32) -> bool"}],
         "clauses":[{"contract":"capacity","label":"C1"}]},
        {"name":"a_load_over_the_limit_does_not_fit","lines":[8,11],
         "items":[{"interface":"stock","signature":"fn fits(used: u32, limit: u32) -> bool"}],
         "clauses":[{"contract":"capacity","label":"C1"}]},
        {"name":"spare_is_the_limit_less_the_used","lines":[13,16],
         "items":[{"interface":"stock","signature":"fn spare(limit: u32, used: u32) -> u32"}],
         "clauses":[{"contract":"capacity","label":"C2"}]}]})
}

fn findings<'a>(result: &'a Value, rule: &str) -> Vec<&'a Value> {
    result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"] == rule)
        .collect()
}

#[test]
fn the_pinned_code_is_mutated_the_missed_mutant_is_found_and_a_changed_harness_makes_it_stale() {
    let dir = project("measure-e2e-survivor");
    let all = Instant::now();

    // The assets and the pin.
    exec(
        &dir,
        &[
            json!({"op":"init"}),
            json!({"op":"context_add","name":"inventory"}),
            json!({"op":"contract_put","context":"inventory","name":"capacity","title":"Capacity",
                   "clauses":[{"label":"C1","text":"a load up to the limit fits"},
                              {"label":"C2","text":"spare is the limit less the used"}]}),
            json!({"op":"interface_put","context":"inventory","name":"stock",
                   "source":"pub fn fits(used: u32, limit: u32) -> bool;\npub fn spare(limit: u32, used: u32) -> u32;\n",
                   "items":[{"signature":"fn fits(used: u32, limit: u32) -> bool","lines":[1,1]},
                            {"signature":"fn spare(limit: u32, used: u32) -> u32","lines":[2,2]}]}),
            harness_put(HARNESS),
            json!({"op":"pin","context":"inventory","interface":"stock","code":PINNED}),
        ],
    );

    // Before a measure: nothing measured.
    let before = exec(&dir, &[json!({"op":"check"})]);
    assert_eq!(findings(&before[0], "mutation.unmeasured").len(), 1);

    // The measure.
    let started = Instant::now();
    let measured = exec(
        &dir,
        &[json!({"op":"measure","kind":"mutate","context":"inventory","interface":"stock"})],
    );
    let measure_time = started.elapsed();
    let m = &measured[0];
    assert_eq!(m["outcome"], "ok", "{m:#?}");
    assert_eq!(m["failure"], Value::Null);
    assert_eq!(
        (m["sites"].as_u64(), m["killed"].as_u64()),
        (Some(2), Some(1))
    );
    // every site was evaluated: the run is not capped
    assert_eq!(
        (m["evaluated"].as_u64(), m["capped"].as_bool()),
        (Some(2), Some(false))
    );
    assert_eq!(
        (
            m["survived"].as_u64(),
            m["stillborn"].as_u64(),
            m["timeout"].as_u64()
        ),
        (Some(1), Some(0), Some(0))
    );
    let mutants = m["mutants"].as_array().expect("mutants");
    assert_eq!(mutants.len(), 2);
    let survivor = mutants
        .iter()
        .find(|x| x["outcome"] == "survived")
        .expect("one survivor");
    assert!(
        survivor["key"]
            .as_str()
            .expect("key")
            .starts_with("fits/rel/1/")
    );
    assert_eq!(survivor["change"], "<= -> <");
    assert_eq!(survivor["site"], "3:10");
    let killed = mutants
        .iter()
        .find(|x| x["outcome"] == "killed")
        .expect("one killed");
    assert!(
        killed["key"]
            .as_str()
            .expect("key")
            .starts_with("spare/arith/1/")
    );
    let steps: Vec<&str> = m["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .map(|e| e["step"].as_str().expect("step"))
        .collect();
    assert_eq!(steps, ["baseline-build", "baseline-test", "provider"]);

    // The project was not touched, and the run's directory is gone.
    assert!(
        std::fs::read_to_string(dir.join("src/stock.rs"))
            .expect("stock.rs")
            .contains("{ false }")
    );
    assert!(!dir.join("tests").exists());
    let leftovers = std::fs::read_dir(dir.join(".chidev/measure"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);

    // The check shows the survivor; the status shows the run as current.
    let after = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let survived = findings(&after[0], "mutation.survived");
    assert_eq!(survived.len(), 1, "{:#?}", after[0]);
    assert_eq!(survived[0]["level"], "note");
    assert_eq!(survived[0]["about"], "interface stock");
    let message = survived[0]["message"].as_str().expect("message");
    assert!(
        message.contains("fits/rel/1/") && message.contains("rel `<=` -> `<`"),
        "{message}"
    );
    assert!(findings(&after[0], "mutation.stale").is_empty());
    assert!(findings(&after[0], "mutation.unmeasured").is_empty());
    assert!(findings(&after[0], "mutation.capped").is_empty());
    let standing = &after[1]["measurements"][0];
    assert_eq!(
        (
            standing["sites"].as_u64(),
            standing["evaluated"].as_u64(),
            standing["capped"].as_bool()
        ),
        (Some(2), Some(2), Some(false))
    );
    assert_eq!(standing["interface"], "stock");
    assert_eq!(standing["state"], "current");
    assert_eq!(standing["outcome"], "ok");
    assert_eq!(
        (standing["killed"].as_u64(), standing["survived"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(standing["stamp"], m["stamp"]);

    // A changed Harness: the run no longer describes it.
    exec(
        &dir,
        &[harness_put(&format!(
            "{HARNESS}// the boundary is still unchecked\n"
        ))],
    );
    let moved = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let stale = findings(&moved[0], "mutation.stale");
    assert_eq!(stale.len(), 1, "{:#?}", moved[0]);
    assert_eq!(stale[0]["level"], "warning");
    assert_eq!(stale[0]["about"], "interface stock");
    assert!(findings(&moved[0], "mutation.survived").is_empty());
    assert_eq!(moved[1]["measurements"][0]["state"], "stale");
    assert_eq!(moved[1]["measurements"][0]["stamp"], m["stamp"]);

    // Measured again against the new Harness, the run is current and has a new stamp.
    let again = exec(
        &dir,
        &[json!({"op":"measure","kind":"mutate","context":"inventory","interface":"stock"})],
    );
    assert_eq!(again[0]["outcome"], "ok");
    assert_ne!(again[0]["stamp"], m["stamp"]);
    assert_eq!(again[0]["survived"], 1);
    let fresh = exec(&dir, &[json!({"op":"check"})]);
    assert!(findings(&fresh[0], "mutation.stale").is_empty());
    assert_eq!(findings(&fresh[0], "mutation.survived").len(), 1);

    eprintln!(
        "measure e2e: first measure {:.1}s, whole test {:.1}s",
        measure_time.as_secs_f64(),
        all.elapsed().as_secs_f64()
    );
}

const MESSAGE: &str = include_str!("fixtures/measure/message.rs");
const MESSAGE_TESTS: &str = include_str!("fixtures/measure/message_tests.rs");

/// A project whose only Interface, `message`, has a refusal text no test reads; `extra` is added to `[measure]`.
fn message_project(name: &str, extra: &str) -> PathBuf {
    let dir = fresh_dir(name);
    std::fs::create_dir_all(dir.join("src")).expect("src");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"inventory\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .expect("Cargo.toml");
    std::fs::write(dir.join("src/lib.rs"), "pub mod message;\n").expect("lib.rs");
    std::fs::write(
        dir.join("src/message.rs"),
        "pub fn check(_: u32) -> Result<u32, String> { Ok(0) }\n",
    )
    .expect("message.rs");
    std::fs::write(
        dir.join("checks.toml"),
        format!(
            r#"[measure]
build = ["cargo", "build", "--quiet", "--tests", "--offline"]
test = ["cargo", "test", "--quiet", "--offline"]
baseline_timeout = 300
timeout = 120
budget = 900
{extra}

[measure.mutants]
rust = [{bin:?}, "provider", "mutants-rust"]

[[measure.layout]]
context = "inventory"
interface = "message"
lang = "rust"
path = "src/message.rs"

[[measure.layout]]
context = "inventory"
harness = "message-tests"
path = "tests/message.rs"
"#,
            bin = env!("CARGO_BIN_EXE_chidev"),
            extra = extra
        ),
    )
    .expect("checks.toml");
    dir
}

fn message_setup(dir: &Path) {
    let signature = "fn check(n: u32) -> Result<u32, String>";
    exec(
        dir,
        &[
            json!({"op":"init"}),
            json!({"op":"context_add","name":"inventory"}),
            json!({"op":"contract_put","context":"inventory","name":"quantity","title":"Quantity",
                   "clauses":[{"label":"C1","text":"zero is refused"},
                              {"label":"C2","text":"a positive quantity passes"}]}),
            json!({"op":"interface_put","context":"inventory","name":"message",
                   "source":"pub fn check(n: u32) -> Result<u32, String>;\n",
                   "items":[{"signature":signature,"lines":[1,1]}]}),
            json!({"op":"harness_put","context":"inventory","name":"message-tests",
                   "source":MESSAGE_TESTS,"cases":[
                {"name":"zero_is_refused","lines":[3,6],
                 "items":[{"interface":"message","signature":signature}],
                 "clauses":[{"contract":"quantity","label":"C1"}]},
                {"name":"a_positive_quantity_passes","lines":[8,11],
                 "items":[{"interface":"message","signature":signature}],
                 "clauses":[{"contract":"quantity","label":"C2"}]}]}),
            json!({"op":"pin","context":"inventory","interface":"message","code":MESSAGE}),
        ],
    );
}

fn mutate_message(dir: &Path) -> Value {
    exec(
        dir,
        &[json!({"op":"measure","kind":"mutate","context":"inventory","interface":"message"})],
    )
    .remove(0)
}

#[test]
fn a_message_no_test_reads_is_a_surviving_str_mutant_unless_str_is_left_out() {
    // By default every class is made: the refusal text is changed two ways, and the tests, which only ask whether
    // the result is an error, notice neither.
    let dir = message_project("measure-e2e-str", "");
    message_setup(&dir);
    let m = mutate_message(&dir);
    assert_eq!(m["outcome"], "ok", "{m:#?}");
    // `==`, `0` (the comparison and the literal), and the two edits of the text
    assert_eq!(m["sites"].as_u64(), Some(4), "{m:#?}");
    let mutants = m["mutants"].as_array().expect("mutants");
    let survivors: Vec<&str> = mutants
        .iter()
        .filter(|x| x["outcome"] == "survived")
        .map(|x| x["key"].as_str().expect("key"))
        .collect();
    assert_eq!(survivors.len(), 2, "{m:#?}");
    assert!(
        survivors[0].starts_with("check/str-empty/1/"),
        "{survivors:?}"
    );
    assert!(
        survivors[1].starts_with("check/str-marker/1/"),
        "{survivors:?}"
    );
    assert!(
        mutants
            .iter()
            .filter(|x| x["outcome"] == "killed")
            .all(|x| !x["key"].as_str().expect("key").contains("/str-"))
    );
    let checked = exec(&dir, &[json!({"op":"check"})]);
    assert_eq!(findings(&checked[0], "mutation.survived").len(), 2);

    // Without `str` the text is not touched: the same tests kill what is left, and nothing survives.
    let dir = message_project(
        "measure-e2e-no-str",
        "operators = [\"rel\", \"eq\", \"arith\", \"logic\", \"bool\", \"int\"]",
    );
    message_setup(&dir);
    let m = mutate_message(&dir);
    assert_eq!(m["outcome"], "ok", "{m:#?}");
    assert_eq!(
        (m["sites"].as_u64(), m["survived"].as_u64()),
        (Some(2), Some(0)),
        "{m:#?}"
    );
    assert!(
        m["mutants"]
            .as_array()
            .expect("mutants")
            .iter()
            .all(|x| x["key"].as_str().is_some_and(|k| !k.contains("/str-")))
    );
    let checked = exec(&dir, &[json!({"op":"check"})]);
    assert!(findings(&checked[0], "mutation.survived").is_empty());
}

#[test]
fn a_run_limited_by_max_mutants_says_how_many_sites_it_left_out() {
    // The provider finds 2 sites; `max_mutants = 1` lets the run evaluate one of them.
    let dir = project_with("measure-e2e-capped", "max_mutants = 1");
    exec(
        &dir,
        &[
            json!({"op":"init"}),
            json!({"op":"context_add","name":"inventory"}),
            json!({"op":"contract_put","context":"inventory","name":"capacity","title":"Capacity",
                   "clauses":[{"label":"C1","text":"a load up to the limit fits"},
                              {"label":"C2","text":"spare is the limit less the used"}]}),
            json!({"op":"interface_put","context":"inventory","name":"stock",
                   "source":"pub fn fits(used: u32, limit: u32) -> bool;\npub fn spare(limit: u32, used: u32) -> u32;\n",
                   "items":[{"signature":"fn fits(used: u32, limit: u32) -> bool","lines":[1,1]},
                            {"signature":"fn spare(limit: u32, used: u32) -> u32","lines":[2,2]}]}),
            harness_put(HARNESS),
            json!({"op":"pin","context":"inventory","interface":"stock","code":PINNED}),
        ],
    );
    let measured = exec(
        &dir,
        &[json!({"op":"measure","kind":"mutate","context":"inventory","interface":"stock"})],
    );
    let m = &measured[0];
    // the run is `ok`, and says it did not look at everything
    assert_eq!(m["outcome"], "ok", "{m:#?}");
    assert_eq!(
        (
            m["sites"].as_u64(),
            m["evaluated"].as_u64(),
            m["capped"].as_bool()
        ),
        (Some(2), Some(1), Some(true)),
        "{m:#?}"
    );
    assert_eq!(m["mutants"].as_array().expect("mutants").len(), 1);

    let after = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let capped = findings(&after[0], "mutation.capped");
    assert_eq!(capped.len(), 1, "{:#?}", after[0]);
    assert_eq!(capped[0]["level"], "note");
    assert_eq!(capped[0]["about"], "interface stock");
    assert_eq!(capped[0]["message"], "evaluated 1 of 2 sites");
    assert!(findings(&after[0], "measure.failed").is_empty());
    let standing = &after[1]["measurements"][0];
    assert_eq!(standing["outcome"], "ok");
    assert_eq!(
        (
            standing["sites"].as_u64(),
            standing["evaluated"].as_u64(),
            standing["capped"].as_bool()
        ),
        (Some(2), Some(1), Some(true))
    );
}

#[test]
fn a_baseline_that_fails_is_a_failed_measurement_and_a_failed_finding() {
    let dir = project("measure-e2e-baseline");
    // A Harness whose source does not compile against the code: nothing to measure against.
    exec(
        &dir,
        &[
            json!({"op":"init"}),
            json!({"op":"context_add","name":"inventory"}),
            json!({"op":"contract_put","context":"inventory","name":"capacity","title":"Capacity",
                   "clauses":[{"label":"C1","text":"a load up to the limit fits"},
                              {"label":"C2","text":"spare is the limit less the used"}]}),
            json!({"op":"interface_put","context":"inventory","name":"stock",
                   "source":"pub fn fits(used: u32, limit: u32) -> bool;\npub fn spare(limit: u32, used: u32) -> u32;\n",
                   "items":[{"signature":"fn fits(used: u32, limit: u32) -> bool","lines":[1,1]},
                            {"signature":"fn spare(limit: u32, used: u32) -> u32","lines":[2,2]}]}),
            harness_put(
                &HARNESS.replace("assert_eq!(spare(5, 2), 3);", "assert_eq!(spare(5, 2), 4);"),
            ),
            json!({"op":"pin","context":"inventory","interface":"stock","code":PINNED}),
        ],
    );
    let measured = exec(
        &dir,
        &[json!({"op":"measure","kind":"mutate","context":"inventory","interface":"stock"})],
    );
    let m = &measured[0];
    assert_eq!(m["outcome"], "failed");
    assert!(
        m["failure"]
            .as_str()
            .expect("failure")
            .starts_with("baseline: the baseline-test failed"),
        "{m:#?}"
    );
    assert_eq!(m["mutants"].as_array().expect("mutants").len(), 0);
    let steps: Vec<&str> = m["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .map(|e| e["step"].as_str().expect("step"))
        .collect();
    assert_eq!(steps, ["baseline-build", "baseline-test"]);
    let checked = exec(&dir, &[json!({"op":"check"})]);
    let failed = findings(&checked[0], "measure.failed");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["level"], "warning");
    assert!(findings(&checked[0], "mutation.unmeasured").is_empty());
    assert!(findings(&checked[0], "mutation.survived").is_empty());
}
