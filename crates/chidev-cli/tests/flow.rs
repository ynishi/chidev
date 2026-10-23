//! Runs `tests/flow.jsonl` through `chidev exec`: two Contexts get their Contract, Interfaces and Harnesses, are
//! pinned, a Crux is raised from a request and decided against its default, the change is made and the Crux
//! resolved, the Pins and citations go stale, the Harnesses are brought up to date, the code is pinned again, and a
//! change that would orphan a citation is refused.

use std::process::Command;

use serde_json::Value;

fn run() -> (Vec<Value>, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .arg("exec")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/flow.jsonl"))
        .output()
        .expect("chidev runs");
    let lines = String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect();
    (lines, out.status.code())
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
    assert_eq!(results.len(), 25);
    assert!(
        results[..24].iter().all(|r| r["ok"] == true),
        "{results:#?}"
    );

    // pinned against the first contract and interface: both fresh
    let first = at(&results, 12);
    assert_eq!(
        pin_states(first),
        states(&[("price", "fresh"), ("print", "fresh")])
    );

    // the decision differs from the default and the clause written on the default has not changed yet
    let decided = at(&results, 15);
    assert_eq!(decided["cruxes"][0]["state"], "decided");
    assert_eq!(decided["cruxes"][0]["decision"], "half to even");
    assert_eq!(decided["unapplied"][0]["unchanged"][0], "rounding C1");

    // the clause changed (line 16), so the Crux resolves (line 17); then the interface changes too: the pins and
    // the citations are behind, and the Crux stays resolved
    let changed = at(&results, 19);
    assert_eq!(changed["cruxes"][0]["state"], "resolved");
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
    // or line 18 would have been refused for orphaning the cases that cite it
    assert_eq!(at(&results, 18)["revision"], 2);
    assert_eq!(at(&results, 18)["id"], at(&results, 6)["id"]);

    // the harnesses brought up to date and the code pinned again: everything is fresh
    let again = at(&results, 24);
    assert_eq!(
        pin_states(again),
        states(&[("price", "fresh"), ("print", "fresh")])
    );
    assert_eq!(again["stale_cites"], serde_json::json!([]));

    // dropping the item the harnesses cite is refused, and the run reports the failure
    let refused = at(&results, 25);
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .expect("error")
            .contains("cannot reach")
    );
    assert_eq!(code, Some(1));
}
