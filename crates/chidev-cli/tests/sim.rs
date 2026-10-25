//! The blind simulation end to end, through the `chidev` binary, for a program that is not Rust: a bank account in
//! Python 3 (standard library only) and a sim script that drives it as a black box under the protocol
//! `chidev-sim 2`. Nothing in the project is compiled; the build command only byte-compiles the program.
//!
//! The program pays out up to 10 from an empty account, which clause C2 forbids, so the sim finds `account:C2`
//! broken in the seeds that withdraw from an empty account and in the regress case that does. A second script that
//! reads the clock is not repeatable: its second runs give another schedule and `sim.nondeterministic` (mandatory)
//! says so. The sim Harness cites Clauses only and names the Interface it builds in `builds`; a tests Harness is
//! what lets the Interface be pinned. A third test has one sim cover two Contracts that both have a `C1`.
//!
//! It starts a few dozen Python processes and takes a couple of seconds.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

const BANK: &str = include_str!("fixtures/sim/bank.py");
const SIM: &str = include_str!("fixtures/sim/sim.py");

/// A directory of its own under cargo's temporary one, with an empty `chidev.toml` (it is a project's root).
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("chidev.toml"), "").expect("a file");
    dir
}

/// `chidev exec` with `dir` as the project's root, on a store inside it: the result line of each operation, and
/// what the program wrote to its standard error. A refused operation is a result with `"ok": false`.
fn run_ops(dir: &Path, ops: &[Value]) -> (Vec<Value>, String) {
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
    (lines, stderr)
}

/// As [`run_ops`], and every operation succeeded.
fn exec(dir: &Path, ops: &[Value]) -> Vec<Value> {
    let (lines, stderr) = run_ops(dir, ops);
    assert!(
        lines.iter().all(|r| r["ok"] == true),
        "{lines:#?}\nstderr: {stderr}"
    );
    lines
}

fn project(name: &str) -> PathBuf {
    project_running(name, r#"["python3", "{tree}/bank.py"]"#)
}

/// As [`project`], the program under test being the command `program` (a TOML array).
fn project_running(name: &str, program: &str) -> PathBuf {
    let python = Command::new("python3")
        .arg("--version")
        .output()
        .expect("python3 is needed for the simulation end to end");
    assert!(python.status.success());
    let dir = fresh_dir(name);
    // The working tree's own program is a stub that answers nothing: the simulation must run the pinned code.
    std::fs::write(dir.join("bank.py"), "import sys\nsys.exit(1)\n").expect("bank.py");
    std::fs::write(
        dir.join("chidev.toml"),
        format!(
            r#"[measure]
build = ["python3", "-m", "py_compile", "bank.py"]
env = {{}}

[measure.sim.ledger]
interpreter = ["python3"]
program = {program}
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
"#
        ),
    )
    .expect("chidev.toml");
    dir
}

/// The ops that put the model: a Contract of two Clauses, the Interface of the program, a tests Harness that cites
/// its Items (which is what lets the Interface be pinned), and the sim Harness whose script is `script`, with a
/// sweep citing both Clauses and a regress case citing C2. The sim cites no Item; it builds the Interface.
fn model(script: &str) -> Vec<Value> {
    let deposit = json!({"interface":"bank","key":"deposit"});
    let withdraw = json!({"interface":"bank","key":"withdraw"});
    let balance = json!({"interface":"bank","key":"balance"});
    vec![
        json!({"op":"context_add","name":"ledger"}),
        json!({"op":"contract_put","context":"ledger","name":"account","title":"Account",
               "clauses":[{"label":"C1","text":"a deposit adds its amount to the balance"},
                          {"label":"C2","text":"a withdrawal of more than the balance is refused and leaves the balance as it was"}]}),
        json!({"op":"interface_put","context":"ledger","name":"bank",
               "source":"deposit N\nwithdraw N\nbalance\n",
               "items":[{"key":"deposit","lines":[1,1]},
                        {"key":"withdraw","lines":[2,2]},
                        {"key":"balance","lines":[3,3]}]}),
        json!({"op":"harness_put","context":"ledger","name":"bank-tests","source":"tests\n","cases":[
            {"name":"bank_basics","lines":[1,1],"items":[deposit, withdraw, balance]}]}),
        harness_put(script),
    ]
}

fn harness_put(script: &str) -> Value {
    json!({"op":"harness_put","context":"ledger","name":"bank-sim","kind":"sim","builds":["bank"],"source":script,"cases":[
        {"name":"sweep","lines":[1,1],
         "clauses":[{"contract":"account","label":"C1"},{"contract":"account","label":"C2"}]},
        {"name":"regress-overdraw","lines":[2,2],
         "clauses":[{"contract":"account","label":"C2"}]}]})
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
    // the sim Harness is well formed: the put of it went through (a malformed one is refused)
    let put = exec(&dir, &setup);
    assert_eq!(put[5]["ok"], true, "{:#?}", put[5]);

    // Before a simulation: the Harness was never run, and the shape a store kept is not reported for it.
    let before = exec(&dir, &[json!({"op":"check"})]);
    assert!(
        findings(&before[0], "sim.shape-kept").is_empty(),
        "{before:#?}"
    );
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
    assert_eq!(regress["findings"][0]["clause"], "account:C2");
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
    assert_eq!(broken[0]["rule"], "sim.finding");
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
    exec(&dir, &[harness_put(&clock)]);
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
fn a_sim_harness_that_is_not_one_sweep_and_regress_cases_is_refused_by_the_rule_it_breaks() {
    let dir = project("sim-e2e-shape");
    let mut ops = vec![json!({"op":"init"})];
    ops.extend(model(SIM));
    ops.push(json!({"op":"pin","context":"ledger","interface":"bank","code":BANK}));
    exec(&dir, &ops);
    // a case that is neither `sweep` nor `regress-<name>`
    let bad = json!({"op":"harness_put","context":"ledger","name":"bank-sim","kind":"sim","builds":["bank"],"source":SIM,"cases":[
        {"name":"sweep","lines":[1,1],
         "clauses":[{"contract":"account","label":"C1"}]},
        {"name":"a_test","lines":[2,2],"clauses":[{"contract":"account","label":"C2"}]}]});
    let (put, _) = run_ops(&dir, &[bad]);
    assert_eq!(put[0]["ok"], false, "{:#?}", put[0]);
    let diagnostics = put[0]["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(diagnostics.len(), 1, "{:#?}", put[0]);
    let d = &diagnostics[0];
    assert_eq!(d["code"], "sim.case-name");
    assert_eq!(d["level"], "error");
    assert_eq!(d["values"]["reason"], "neither-sweep-nor-regress");
    // the case and the harness are named, never given by id
    assert_eq!(
        d["subjects"],
        json!([
            {"kind":"case","name":"case bank-sim a_test"},
            {"kind":"harness","name":"harness bank-sim"}
        ])
    );
    assert_eq!(
        d["message"],
        "the name of `case bank-sim a_test` is not one the sim `harness bank-sim` can have: neither-sweep-nor-regress"
    );

    // nothing was kept: the sim Harness the model had stands, with the shape it had
    let checked = exec(&dir, &[json!({"op":"check"})]);
    assert!(
        findings(&checked[0], "sim.shape-kept").is_empty(),
        "{checked:#?}"
    );
    assert!(
        findings(&checked[0], "sim.uncited-clause").is_empty(),
        "{checked:#?}"
    );
    // and the rule can be read: `explain` gives it by its id
    let rule = exec(&dir, &[json!({"op":"explain","rule":"sim.case-name"})]);
    assert_eq!(rule[0]["kind"], "invariant");
}

/// A script that does not drive the program at all: on every case it finds `audit:C1` broken and says that
/// `account:C1` never came up. `account` and `audit` both have a Clause labelled C1.
const TWO_CONTRACTS: &str = r#"import sys
case, flag, out, dash = sys.argv[1:5]
open(out + "/schedule", "w").write(case)
open(out + "/trace", "w").write("-")
print("FINDING %s step 1 audit:C1: the audit log is missing" % case)
print("UNREACHED account:C1: no deposit")
print("sim: findings=1")
sys.exit(1)
"#;

/// The sim Harness `bank-sim` over `account` (C1, C2) and `audit` (C1), whose sweep cites `clauses`.
fn two_contract_sim(clauses: &[(&str, &str)]) -> Value {
    let cited: Vec<Value> = clauses
        .iter()
        .map(|(contract, label)| json!({"contract":contract,"label":label}))
        .collect();
    json!({"op":"harness_put","context":"ledger","name":"bank-sim","kind":"sim","builds":["bank"],
           "source":TWO_CONTRACTS,"cases":[{"name":"sweep","lines":[1,1],"clauses":cited}]})
}

#[test]
fn one_sim_over_two_contracts_with_a_label_in_common_puts_each_finding_on_its_own_clause() {
    let dir = project("sim-e2e-two");
    let mut ops = vec![json!({"op":"init"})];
    ops.extend(model(TWO_CONTRACTS));
    ops.push(
        json!({"op":"contract_put","context":"ledger","name":"audit","title":"Audit",
                    "clauses":[{"label":"C1","text":"every withdrawal is logged"}]}),
    );
    ops.push(json!({"op":"pin","context":"ledger","interface":"bank","code":BANK}));
    exec(&dir, &ops);

    // The sim takes on `account` only: `audit` is left to the tests, and nothing is said of its Clause. Then it
    // takes on both, citing the two C1 (a label two Clauses have is fine now that a script names the Contract).
    let narrow = exec(&dir, &[json!({"op":"check"})]);
    assert!(
        findings(&narrow[0], "sim.uncited-clause").is_empty(),
        "{narrow:#?}"
    );
    let put = exec(
        &dir,
        &[two_contract_sim(&[
            ("account", "C1"),
            ("account", "C2"),
            ("audit", "C1"),
        ])],
    );
    assert_eq!(put[0]["ok"], true, "{put:#?}");
    // and with only one Clause of `audit` cited and one of `account`, `account`'s C2 is the silence reported
    let partial = exec(
        &dir,
        &[
            two_contract_sim(&[("account", "C1"), ("audit", "C1")]),
            json!({"op":"check"}),
        ],
    );
    assert!(
        findings(&partial[1], "sim.shape-kept").is_empty(),
        "{partial:#?}"
    );
    let uncited = findings(&partial[1], "sim.uncited-clause");
    assert_eq!(uncited.len(), 1, "{partial:#?}");
    assert_eq!(uncited[0]["about"], "account C2");
    exec(
        &dir,
        &[two_contract_sim(&[
            ("account", "C1"),
            ("account", "C2"),
            ("audit", "C1"),
        ])],
    );

    let measured = exec(
        &dir,
        &[json!({"op":"measure","kind":"simulate","context":"ledger","harness":"bank-sim"})],
    );
    let m = &measured[0];
    assert_eq!(m["outcome"], "ok", "{m:#?}");
    // the report named the Clauses by Contract, and the record holds them so
    let first = &m["cases"][0];
    assert_eq!(first["findings"][0]["clause"], "audit:C1");
    assert_eq!(first["unreached"][0]["clause"], "account:C1");

    let after = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let broken = findings(&after[0], "sim.finding");
    assert_eq!(broken.len(), 1, "{:#?}", after[0]);
    assert_eq!(broken[0]["about"], "audit C1");
    let missed = findings(&after[0], "sim.unreached");
    assert_eq!(missed.len(), 1, "{:#?}", after[0]);
    assert_eq!(missed[0]["about"], "account C1");
    assert!(findings(&after[0], "sim.uncited-clause").is_empty());
    assert!(findings(&after[0], "sim.stale").is_empty());

    // The Pin of the Interface the sim builds does not stand on the sim: putting the sim again moves the sim's
    // stamp but leaves the Pin fresh. Pinning other code makes the simulation stale, as the program it ran is gone.
    let pin_state = |status: &Value| {
        status["pins"]
            .as_array()
            .expect("pins")
            .iter()
            .find(|p| p["interface"] == "bank")
            .map(|p| p["state"].clone())
            .expect("the pin of bank")
    };
    assert_eq!(pin_state(&after[1]), "fresh", "{:#?}", after[1]);
    exec(
        &dir,
        &[two_contract_sim(&[("account", "C1"), ("audit", "C1")])],
    );
    let moved = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    assert_eq!(findings(&moved[0], "sim.stale").len(), 1, "{moved:#?}");
    assert_eq!(pin_state(&moved[1]), "fresh", "{:#?}", moved[1]);
    exec(
        &dir,
        &[
            two_contract_sim(&[("account", "C1"), ("account", "C2"), ("audit", "C1")]),
            json!({"op":"measure","kind":"simulate","context":"ledger","harness":"bank-sim"}),
            json!({"op":"pin","context":"ledger","interface":"bank","code":format!("{BANK}\n# other\n")}),
        ],
    );
    let repinned = exec(&dir, &[json!({"op":"check"})]);
    assert_eq!(
        findings(&repinned[0], "sim.stale").len(),
        1,
        "{repinned:#?}"
    );
}

#[test]
fn a_program_that_cannot_be_started_is_a_failed_run_with_no_finding_and_only_sim_error_speaks() {
    // The program is a path that is not there: the script cannot start it in any case and says so.
    let dir = project_running("sim-e2e-nostart", r#"["{tree}/no-such-program"]"#);
    let mut setup = vec![json!({"op":"init"})];
    setup.extend(model(SIM));
    setup.push(json!({"op":"pin","context":"ledger","interface":"bank","code":BANK}));
    exec(&dir, &setup);

    let measured = exec(
        &dir,
        &[json!({"op":"measure","kind":"simulate","context":"ledger","harness":"bank-sim"})],
    );
    let m = &measured[0];
    assert_eq!(m["outcome"], "failed", "{m:#?}");
    let failure = m["failure"].as_str().expect("a failure");
    assert!(failure.starts_with("no_start: "), "{failure}");
    assert!(failure.contains("no-such-program"), "{failure}");
    // every case is recorded as one that could not start, with no finding and nothing unreached
    let cases = m["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 9, "8 seeds and the regress case");
    for c in cases {
        let exit = c["exit"].as_str().expect("exit");
        assert!(exit.starts_with("cannot_start:"), "{c:#?}");
        assert!(c["findings"].as_array().expect("findings").is_empty());
        assert!(c["unreached"].as_array().expect("unreached").is_empty());
    }
    // no second runs: there was nothing to repeat
    assert!(m.get("reruns").is_none(), "{m:#?}");

    let after = exec(&dir, &[json!({"op":"check"}), json!({"op":"status"})]);
    let errors = findings(&after[0], "sim.error");
    assert_eq!(errors.len(), 1, "{:#?}", after[0]);
    assert!(
        errors[0]["message"]
            .as_str()
            .expect("message")
            .contains("no_start"),
        "{errors:#?}"
    );
    assert!(
        findings(&after[0], "sim.finding").is_empty(),
        "{:#?}",
        after[0]
    );
    assert!(findings(&after[0], "sim.unreached").is_empty());
    assert!(findings(&after[0], "sim.nondeterministic").is_empty());
    let standing = &after[1]["measurements"][0];
    assert_eq!(standing["outcome"], "failed");
    assert!(
        standing["failure"]
            .as_str()
            .expect("failure")
            .starts_with("no_start")
    );
}

#[test]
fn a_sim_harness_builds_only_interfaces_the_context_has_and_a_tests_harness_builds_none() {
    let dir = project("sim-e2e-builds");
    let mut ops = vec![json!({"op":"init"})];
    ops.extend(model(SIM));
    exec(&dir, &ops);
    let mut ghost = harness_put(SIM);
    ghost["builds"] = json!(["bank", "ghost"]);
    let mut tests = json!({"op":"harness_put","context":"ledger","name":"bank-tests","builds":["bank"],
        "source":"tests\n","cases":[{"name":"bank_basics","lines":[1,1],
        "items":[{"interface":"bank","key":"balance"}]}]});
    tests["kind"] = json!("tests");
    let (refused, _) = run_ops(&dir, &[ghost, tests]);
    assert_eq!(refused[0]["ok"], false, "{refused:#?}");
    assert!(refused[0].to_string().contains("ghost"), "{refused:#?}");
    assert_eq!(refused[1]["ok"], false, "{refused:#?}");
    assert!(refused[1].to_string().contains("builds"), "{refused:#?}");
    // a sim Case cites no Item
    let mut cites_item = harness_put(SIM);
    cites_item["cases"][0]["items"] = json!([{"interface":"bank","key":"balance"}]);
    let (refused, _) = run_ops(&dir, &[cites_item]);
    assert_eq!(refused[0]["ok"], false, "{refused:#?}");
    assert!(
        refused[0].to_string().contains("cites an item"),
        "{refused:#?}"
    );
    // and builds as a change set entry is read the same way
    let set = json!({"op":"change_set","puts":[
        {"put":"harness","context":"ledger","name":"bank-sim","kind":"sim","builds":["bank"],"source":SIM,
         "cases":[{"name":"sweep","lines":[1,1],"clauses":[{"contract":"account","label":"C1"}]}]}]});
    exec(&dir, &[set]);
}
