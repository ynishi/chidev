//! The blind simulation end to end, through the `chidev` binary, for a program that is not Rust: a bank account in
//! Python 3 (standard library only) and a sim script that drives it as a black box under the protocol
//! `chidev-sim 1`. Nothing in the project is compiled; the build command only byte-compiles the program.
//!
//! The program pays out up to 10 from an empty account, which clause C2 forbids, so the sim finds C2 broken in the
//! seeds that withdraw from an empty account and in the regress case that does. A second script that reads the
//! clock is not repeatable: its second runs give another schedule and `sim.nondeterministic` (mandatory) says so.
//!
//! It starts a few dozen Python processes and takes a couple of seconds.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

const BANK: &str = include_str!("fixtures/sim/bank.py");
const SIM: &str = include_str!("fixtures/sim/sim.py");

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
    let python = Command::new("python3")
        .arg("--version")
        .output()
        .expect("python3 is needed for the simulation end to end");
    assert!(python.status.success());
    let dir = fresh_dir(name);
    // The working tree's own program is a stub that answers nothing: the simulation must run the pinned code.
    std::fs::write(dir.join("bank.py"), "import sys\nsys.exit(1)\n").expect("bank.py");
    std::fs::write(
        dir.join("checks.toml"),
        r#"[measure]
build = ["python3", "-m", "py_compile", "bank.py"]
env = {}

[measure.sim.ledger]
interpreter = ["python3"]
program = ["python3", "{work}/bank.py"]
seeds = 8
sample = 3
timeout = 30
budget = 300

[[measure.layout]]
context = "ledger"
interface = "bank"
path = "bank.py"

[[measure.layout]]
context = "ledger"
harness = "bank-sim"
path = "sim.py"
"#,
    )
    .expect("checks.toml");
    dir
}

/// The ops that put the model: a Contract of two Clauses, the Interface of the program, and the sim Harness whose
/// script is `script`, with a sweep citing both Clauses and a regress case citing C2.
fn model(script: &str) -> Vec<Value> {
    let deposit = json!({"interface":"bank","signature":"deposit N"});
    let withdraw = json!({"interface":"bank","signature":"withdraw N"});
    let balance = json!({"interface":"bank","signature":"balance"});
    vec![
        json!({"op":"context_add","name":"ledger"}),
        json!({"op":"contract_put","context":"ledger","name":"account","title":"Account",
               "clauses":[{"label":"C1","text":"a deposit adds its amount to the balance"},
                          {"label":"C2","text":"a withdrawal of more than the balance is refused and leaves the balance as it was"}]}),
        json!({"op":"interface_put","context":"ledger","name":"bank",
               "source":"deposit N\nwithdraw N\nbalance\n",
               "items":[{"signature":"deposit N","lines":[1,1]},
                        {"signature":"withdraw N","lines":[2,2]},
                        {"signature":"balance","lines":[3,3]}]}),
        harness_put(script, &[deposit, withdraw, balance]),
    ]
}

fn harness_put(script: &str, items: &[Value]) -> Value {
    json!({"op":"harness_put","context":"ledger","name":"bank-sim","kind":"sim","source":script,"cases":[
        {"name":"sweep","lines":[1,1],"items":items,
         "clauses":[{"contract":"account","label":"C1"},{"contract":"account","label":"C2"}]},
        {"name":"regress-overdraw","lines":[2,2],"items":[{"interface":"bank","signature":"withdraw N"}],
         "clauses":[{"contract":"account","label":"C2"}]}]})
}

fn all_items() -> Vec<Value> {
    ["deposit N", "withdraw N", "balance"]
        .iter()
        .map(|s| json!({"interface":"bank","signature":s}))
        .collect()
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
fn a_program_that_breaks_a_clause_is_found_and_a_script_that_reads_the_clock_is_nondeterministic() {
    let dir = project("sim-e2e");
    let all = Instant::now();

    let mut setup = vec![json!({"op":"init"})];
    setup.extend(model(SIM));
    setup.push(json!({"op":"pin","context":"ledger","interface":"bank","code":BANK}));
    let put = exec(&dir, &setup);
    // the sim Harness is well formed: nothing on `sim.shape`
    assert!(findings(&put[4], "sim.shape").is_empty(), "{:#?}", put[4]);

    // Before a simulation: the Harness was never run.
    let before = exec(&dir, &[json!({"op":"check"})]);
    assert_eq!(findings(&before[0], "sim.unrun").len(), 1, "{before:#?}");
    assert!(findings(&before[0], "sim.uncited-clause").is_empty());

    // The simulation.
    let started = Instant::now();
    let measured = exec(
        &dir,
        &[json!({"op":"measure","kind":"simulate","context":"ledger","harness":"bank-sim"})],
    );
    let sim_time = started.elapsed();
    let m = &measured[0];
    assert_eq!(m["kind"], "simulate");
    assert_eq!(m["outcome"], "ok", "{m:#?}");
    let cases = m["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 9, "8 seeds and the regress case");
    assert!(cases.iter().all(|c| c["exit"] == "normal"), "{cases:#?}");
    assert!(
        cases
            .iter()
            .all(|c| c["schedule"].is_string() && c["trace"].is_string())
    );
    // the regress case always withdraws from an empty account: it breaks C2 at step 1
    let regress = cases
        .iter()
        .find(|c| c["case"] == "regress-overdraw")
        .expect("the regress case");
    assert_eq!(regress["findings"][0]["clause"], "C2");
    assert_eq!(regress["findings"][0]["step"], 1);
    // seed-0 and up to three cases with findings were run again, and are repeatable
    let reruns = m["reruns"].as_array().expect("reruns");
    assert!(!reruns.is_empty() && reruns.len() <= 4, "{reruns:#?}");
    assert!(reruns.iter().all(|r| r["result"] == "same"), "{reruns:#?}");
    let steps: Vec<&str> = m["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .map(|e| e["step"].as_str().expect("step"))
        .collect();
    assert_eq!(steps[0], "sim-build");
    assert!(steps.contains(&"rerun-build") && steps.contains(&"sim-regress-overdraw"));

    // The project was not touched, and the run's directory is gone.
    assert_eq!(
        std::fs::read_to_string(dir.join("bank.py")).expect("bank.py"),
        "import sys\nsys.exit(1)\n"
    );
    let leftovers = std::fs::read_dir(dir.join(".chidev/measure"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);

    // The check: C2 has a finding, C1 none; the run is repeatable; the status shows it current.
    let after = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let broken = findings(&after[0], "sim.finding");
    assert_eq!(broken.len(), 1, "{:#?}", after[0]);
    assert_eq!(broken[0]["level"], "error");
    let about = broken[0]["about"].as_str().expect("about");
    assert_eq!(about, "account C2");
    let message = broken[0]["message"].as_str().expect("message");
    assert!(message.starts_with("clause C2 broke in "), "{message}");
    assert!(message.contains("step"), "{message}");
    assert!(findings(&after[0], "sim.nondeterministic").is_empty());
    assert!(findings(&after[0], "sim.stale").is_empty());
    assert!(findings(&after[0], "sim.unrun").is_empty());
    assert!(findings(&after[0], "sim.error").is_empty());
    let standing = &after[1]["measurements"][0];
    assert_eq!(standing["harness"], "bank-sim");
    assert_eq!(standing["kind"], "simulate");
    assert_eq!(standing["state"], "current");
    assert_eq!(standing["sim"]["findings"], 1);
    assert_eq!(standing["sim"]["differing"], 0);
    assert_eq!(standing["stamp"], m["stamp"]);

    // A script that reads the clock: another schedule on every run. The Harness changed, so the first
    // simulation is stale; simulated again, the second runs give other schedules.
    let clock = SIM.replace("CLOCK = False", "CLOCK = True");
    exec(&dir, &[harness_put(&clock, &all_items())]);
    let moved = exec(&dir, &[json!({"op":"check"})]);
    assert_eq!(findings(&moved[0], "sim.stale").len(), 1, "{moved:#?}");
    assert!(findings(&moved[0], "sim.finding").is_empty());
    let again = exec(
        &dir,
        &[
            json!({"op":"measure","kind":"simulate","context":"ledger","harness":"bank-sim"}),
            json!({"op":"check"}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(again[0]["outcome"], "ok");
    assert_ne!(again[0]["stamp"], m["stamp"]);
    let reruns = again[0]["reruns"].as_array().expect("reruns");
    assert!(
        reruns.iter().all(|r| r["result"] == "schedule-differs"),
        "{reruns:#?}"
    );
    let nondeterministic = findings(&again[1], "sim.nondeterministic");
    assert!(!nondeterministic.is_empty(), "{:#?}", again[1]);
    assert!(nondeterministic.iter().all(|f| f["level"] == "error"));
    assert!(
        nondeterministic[0]["message"]
            .as_str()
            .expect("message")
            .contains("gave two schedules")
    );
    assert!(findings(&again[1], "sim.stale").is_empty());
    assert_eq!(
        again[2]["measurements"][0]["sim"]["differing"].as_u64(),
        Some(reruns.len() as u64)
    );

    eprintln!(
        "sim e2e: first simulation {:.1}s ({} cases, {} second runs), whole test {:.1}s",
        sim_time.as_secs_f64(),
        cases.len(),
        reruns.len(),
        all.elapsed().as_secs_f64()
    );
}

#[test]
fn a_sim_harness_that_is_not_one_sweep_and_regress_cases_is_a_shape_error_and_is_not_simulated() {
    let dir = project("sim-e2e-shape");
    let mut ops = vec![json!({"op":"init"})];
    ops.extend(model(SIM));
    ops.push(json!({"op":"pin","context":"ledger","interface":"bank","code":BANK}));
    exec(&dir, &ops);
    // a case that is neither `sweep` nor `regress-<name>`, and no clause cited by the sweep's twin
    let bad = json!({"op":"harness_put","context":"ledger","name":"bank-sim","kind":"sim","source":SIM,"cases":[
        {"name":"sweep","lines":[1,1],"items":[{"interface":"bank","signature":"deposit N"}],
         "clauses":[{"contract":"account","label":"C1"}]},
        {"name":"a_test","lines":[2,2],"items":[{"interface":"bank","signature":"balance"}],"clauses":[]}]});
    let put = exec(&dir, &[bad]);
    let shape = findings(&put[0], "sim.shape");
    assert_eq!(shape.len(), 1, "{:#?}", put[0]);
    assert_eq!(shape[0]["level"], "error");
    let checked = exec(&dir, &[json!({"op":"check"})]);
    // C2 is cited by no sim case any more
    assert_eq!(
        findings(&checked[0], "sim.uncited-clause").len(),
        1,
        "{checked:#?}"
    );

    // the measure refuses it, and a tests harness cannot be simulated either
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(&dir)
        .arg("exec")
        .arg(write_ops(
            &dir,
            &[json!({"op":"measure","kind":"simulate","context":"ledger","harness":"bank-sim"})],
        ))
        .arg("--store")
        .arg(dir.join("chidev.db"))
        .output()
        .expect("chidev runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("not well formed"), "{stdout}");
}

fn write_ops(dir: &Path, ops: &[Value]) -> PathBuf {
    let file = dir.join("refused.jsonl");
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(&file, text).expect("a file");
    file
}
