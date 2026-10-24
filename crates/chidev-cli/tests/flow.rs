//! Runs `tests/flow.jsonl` through `chidev exec`: two Contexts get their Contract, Interfaces and Harnesses, are
//! pinned; a change request is opened, with a part a person decides through its Crux against the default; the
//! change is made and both change requests resolved; the Pins and citations go stale, the Harnesses are brought up
//! to date, the code is pinned again, and a change that would orphan a citation is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A directory of its own under cargo's temporary one, empty, so that a run starts from no store.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

fn exec(file: &Path, store: &Path) -> (Vec<Value>, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .arg("exec")
        .arg(file)
        .arg("--store")
        .arg(store)
        .output()
        .expect("chidev runs");
    let lines = String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect();
    (lines, out.status.code())
}

/// The flow, run on a store of its own.
fn run() -> (Vec<Value>, Option<i32>) {
    let dir = fresh_dir("flow");
    exec(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/flow.jsonl")),
        &dir.join("chidev.db"),
    )
}

/// The result of the operation on line `n` of the flow.
fn at(results: &[Value], n: u64) -> &Value {
    results
        .iter()
        .find(|r| r["line"] == n)
        .unwrap_or_else(|| panic!("no result for line {n}"))
}

fn pin_states(status: &Value) -> Vec<(String, String)> {
    status["pins"]
        .as_array()
        .expect("pins")
        .iter()
        .map(|p| {
            (
                p["interface"].as_str().expect("name").to_string(),
                p["state"].as_str().expect("state").to_string(),
            )
        })
        .collect()
}

fn states(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn the_flow_runs_from_contract_to_pin_through_a_change_and_back() {
    let (results, code) = run();
    assert_eq!(results.len(), 27);
    assert!(
        results[..26].iter().all(|r| r["ok"] == true),
        "{results:#?}"
    );

    // pinned against the first contract and interface: both fresh
    let first = at(&results, 12);
    assert_eq!(
        pin_states(first),
        states(&[("price", "fresh"), ("print", "fresh")])
    );

    // the request is open; its part decided against the default, with the clause written on the default unchanged
    let decided = at(&results, 16);
    let crs = &decided["change_requests"];
    assert_eq!(crs[0]["title"], "Fair totals");
    assert_eq!(crs[0]["state"], "open");
    assert_eq!(crs[0]["crux"], Value::Null);
    assert_eq!(crs[1]["parent"], "Fair totals");
    assert_eq!(crs[1]["issuer"], "designer");
    assert_eq!(crs[1]["crux"], "ambiguity");
    assert_eq!(crs[1]["state"], "decided");
    assert_eq!(crs[1]["decision"], "half to even");
    assert_eq!(decided["unapplied"][0]["unchanged"][0], "rounding C1");

    // the clause changed (line 17), so both change requests resolve (lines 18, 19); then the interface changes
    // too: the pins and the citations are behind, and the change requests stay resolved
    let changed = at(&results, 21);
    assert_eq!(changed["change_requests"][0]["state"], "resolved");
    assert_eq!(changed["change_requests"][1]["state"], "resolved");
    assert_eq!(
        pin_states(changed),
        states(&[("price", "stale"), ("print", "stale")])
    );
    assert_eq!(
        changed["pins"][0]["why"],
        serde_json::json!(["the interface changed", "contract `rounding`"])
    );
    assert_eq!(
        changed["pins"][1]["why"],
        serde_json::json!(["interface `price`"])
    );
    assert_eq!(changed["stale_cites"].as_array().expect("cites").len(), 3);
    assert_eq!(changed["unapplied"], serde_json::json!([]));
    // the interface was revised, not replaced; its item kept its id across the signature change (`replaces`),
    // or line 20 would have been refused for orphaning the cases that cite it
    assert_eq!(at(&results, 20)["revision"], 2);
    assert_eq!(at(&results, 20)["id"], at(&results, 6)["id"]);

    // the harnesses brought up to date and the code pinned again: everything is fresh
    let again = at(&results, 26);
    assert_eq!(
        pin_states(again),
        states(&[("price", "fresh"), ("print", "fresh")])
    );
    assert_eq!(again["stale_cites"], serde_json::json!([]));

    // dropping the item the harnesses cite is refused, and the run reports the failure
    let refused = at(&results, 27);
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .expect("error")
            .contains("cannot reach")
    );
    assert_eq!(code, Some(1));
}

#[test]
fn a_second_run_continues_the_project_the_first_kept() {
    let dir = fresh_dir("persist");
    let store = dir.join("nested").join("chidev.db");
    let first = dir.join("first.jsonl");
    let second = dir.join("second.jsonl");
    std::fs::write(
        &first,
        "{\"op\":\"init\"}\n{\"op\":\"context_add\",\"name\":\"pricing\"}\n",
    )
    .expect("a file");
    std::fs::write(
        &second,
        concat!(
            "{\"op\":\"init\"}\n",
            "{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"rounding\",\"title\":\"Rounding\",",
            "\"clauses\":[{\"label\":\"C1\",\"text\":\"a total is rounded to whole cents\"}]}\n",
            "{\"op\":\"status\"}\n",
        ),
    )
    .expect("a file");

    let (results, code) = exec(&first, &store);
    assert_eq!(code, Some(0), "{results:#?}");

    // the project is there: `init` is refused, and the Context the first run added takes a Contract
    let (results, code) = exec(&second, &store);
    assert_eq!(code, Some(1), "{results:#?}");
    assert_eq!(at(&results, 1)["ok"], false);
    assert!(
        at(&results, 1)["error"]
            .as_str()
            .expect("error")
            .contains("already")
    );
    assert_eq!(at(&results, 2)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 3)["ok"], true);
}
