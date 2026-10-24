//! Runs `tests/flow.jsonl` through `chidev exec`: two Contexts get their Contract, Interfaces and Harnesses, are
//! pinned; a change request is opened, with a part a person decides through its Crux against the default; the
//! change is made and both change requests resolved; the Pins and citations go stale, the Harnesses are brought up
//! to date, the code is pinned again, and a change that would orphan a citation is refused. Each step reports what the
//! checks find: the Clause the new contract gains is cited by no Case until the Harness is brought up, and a change
//! request opened for it turns the finding from open to addressed.

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
    assert_eq!(results.len(), 28);
    assert!(
        results[..27].iter().all(|r| r["ok"] == true),
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

    // the clause changed (line 17), so both change requests resolve (lines 19, 20); then the interface changes
    // too: the pins and the citations are behind, and the change requests stay resolved
    let changed = at(&results, 22);
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
    // or line 21 would have been refused for orphaning the cases that cite it
    assert_eq!(at(&results, 21)["revision"], 2);
    assert_eq!(at(&results, 21)["id"], at(&results, 6)["id"]);

    // the new Clause C2 is cited by no Case when the contract is put (line 17): an open warning, with the name of
    // the clause; the change request opened for it (line 18) makes it addressed from then on, and it stays so
    // while that request is unresolved
    let uncited = |r: &Value| -> Option<Value> {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .find(|f| f["rule"] == "cite.uncited-clause" && f["about"] == "rounding C2")
            .cloned()
    };
    let found = uncited(at(&results, 17)).expect("C2 is uncited");
    assert_eq!(found["level"], "warning");
    assert_eq!(found["state"], "open");
    assert_eq!(
        uncited(at(&results, 22)).expect("still uncited")["state"],
        "addressed"
    );
    // the harness put (line 24) cites it: the finding is gone
    assert!(uncited(at(&results, 24)).is_none());
    // a pin made against what has moved is reported while it lasts, and not once it is pinned again
    let pin_stale = |r: &Value| {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter(|f| f["rule"] == "pin.stale")
            .count()
    };
    assert_eq!(pin_stale(at(&results, 22)), 2);
    assert_eq!(pin_stale(at(&results, 27)), 0);

    // the harnesses brought up to date and the code pinned again: everything is fresh
    let again = at(&results, 27);
    assert_eq!(
        pin_states(again),
        states(&[("price", "fresh"), ("print", "fresh")])
    );
    assert_eq!(again["stale_cites"], serde_json::json!([]));

    // dropping the item the harnesses cite is refused, and the run reports the failure
    let refused = at(&results, 28);
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

/// `chidev exec` run with `dir` as the project's root (its working directory), on a store inside it.
fn exec_in(dir: &Path, ops: &str) -> (Vec<Value>, Option<i32>, String) {
    let file = dir.join("ops.jsonl");
    std::fs::write(&file, ops).expect("a file");
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(dir)
        .arg("exec")
        .arg(&file)
        .arg("--store")
        .arg(dir.join("chidev.db"))
        .output()
        .expect("chidev runs");
    let lines = String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect();
    (
        lines,
        out.status.code(),
        String::from_utf8(out.stderr).expect("utf-8"),
    )
}

const CHECK_OPS: &str = concat!(
    "{\"op\":\"init\"}\n",
    "{\"op\":\"context_add\",\"name\":\"pricing\"}\n",
    "{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"rounding\",\"title\":\"Rounding\",",
    "\"clauses\":[{\"label\":\"C1\",\"text\":\"a total is rounded to whole cents\"}]}\n",
    "{\"op\":\"check\"}\n",
    "{\"op\":\"check_list\"}\n",
);

#[test]
fn the_policy_files_of_the_project_and_the_person_set_the_levels() {
    let dir = fresh_dir("policy");
    std::fs::create_dir_all(dir.join(".chidev")).expect("a directory");
    std::fs::write(
        dir.join("checks.toml"),
        "[checks.\"context.\"]\nlevel = \"off\"\n\n[checks.\"pin.unpinned\"]\nlevel = \"error\"\n",
    )
    .expect("a file");
    std::fs::write(
        dir.join(".chidev/checks.local.toml"),
        "[checks.\"pin.unpinned\"]\nlevel = \"warning\"\n",
    )
    .expect("a file");
    let (results, code, _) = exec_in(&dir, CHECK_OPS);
    assert_eq!(code, Some(0), "{results:#?}");

    let rules: Vec<&str> = at(&results, 4)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule"].as_str().expect("rule"))
        .collect();
    assert_eq!(rules, ["cite.uncited-clause"]);

    let listed = at(&results, 5);
    let entry = |id: &str| {
        listed["checks"]
            .as_array()
            .expect("checks")
            .iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("no check {id}"))
    };
    assert_eq!(listed["checks"].as_array().expect("checks").len(), 10);
    assert_eq!(entry("context.no-harness")["level"], "off");
    assert_eq!(entry("context.no-harness")["default_level"], "note");
    // the person's layer comes after the project's
    assert_eq!(entry("pin.unpinned")["level"], "warning");
    assert_eq!(entry("cite.stale")["level"], "warning");
    assert_eq!(entry("cite.stale")["mandatory"], false);
    assert_eq!(entry("cite.stale")["executor"], "builtin");
    assert!(listed["policy"].as_str().expect("policy").len() == 64);
}

#[test]
fn a_policy_that_names_no_check_stops_the_run_before_any_operation() {
    let dir = fresh_dir("bad-policy");
    std::fs::write(
        dir.join("checks.toml"),
        "[checks.\"cite.nothing\"]\nlevel = \"off\"\n",
    )
    .expect("a file");
    let (results, code, stderr) = exec_in(&dir, CHECK_OPS);
    assert_eq!(code, Some(2));
    assert!(results.is_empty());
    assert!(
        stderr.contains("cite.nothing") && stderr.contains("no check"),
        "{stderr}"
    );
}

const VAGUE_RULE: &str = r#"id = "judged.vague-clause"
summary = "a clause too vague for a case to check"
subject = "clause"
hooks = ["put", "check"]
level = "warning"
question = "Can a case be written from this clause alone?"
true = "the clause has no condition or value a case could check"
false = "the clause is a definition"
fix = "reword it with a condition and an observable result"
when = ["should"]
"#;

/// The findings of the judged rule in a `check` result.
fn judged(check: &Value) -> Vec<&Value> {
    check["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"] == "judged.vague-clause")
        .collect()
}

#[test]
fn an_agent_gets_a_judged_rule_and_the_model_reports_and_sees_the_finding_come_and_go() {
    let dir = fresh_dir("judged");
    std::fs::create_dir_all(dir.join("checks/judged")).expect("a directory");
    std::fs::write(dir.join("checks/judged/vague-clause.toml"), VAGUE_RULE).expect("a file");
    let clause = r#"{"clause":{"context":"pricing","contract":"rounding","label":"C1"}}"#;
    // The store's revision is 1 after `init`, 2 after the Context, 3 after the Contract; reading and checking
    // change nothing, a report is one commit and so is a put: the revisions the agent read are written below.
    let ops = format!(
        concat!(
            "{{\"op\":\"init\"}}\n",
            "{{\"op\":\"context_add\",\"name\":\"pricing\"}}\n",
            "{{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"rounding\",\"title\":\"Rounding\",",
            "\"clauses\":[{{\"label\":\"C1\",\"text\":\"a total should be rounded\"}}]}}\n",
            "{{\"op\":\"check_rules\"}}\n",
            "{{\"op\":\"finding_report\",\"rule\":\"judged.vague-clause\",\"judge\":\"agent-1\",\"read_at\":3,",
            "\"findings\":[{{\"subject\":{clause},\"message\":\"`should` states no condition\"}}]}}\n",
            "{{\"op\":\"check\"}}\n",
            "{{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"rounding\",\"title\":\"Rounding\",",
            "\"clauses\":[{{\"label\":\"C1\",\"text\":\"a total is rounded to whole cents\"}}]}}\n",
            "{{\"op\":\"check\"}}\n",
            "{{\"op\":\"status\"}}\n",
            "{{\"op\":\"finding_report\",\"rule\":\"judged.vague-clause\",\"judge\":\"agent-1\",\"read_at\":5,\"findings\":[]}}\n",
            "{{\"op\":\"check\"}}\n",
            "{{\"op\":\"status\"}}\n",
            "{{\"op\":\"finding_report\",\"rule\":\"cite.stale\",\"judge\":\"agent-1\",\"read_at\":6,\"findings\":[]}}\n",
            "{{\"op\":\"check_rules\",\"hook\":\"pin\"}}\n",
        ),
        clause = clause
    );
    let (results, code, stderr) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{results:#?}\n{stderr}");

    // the rule and a view of the model, read at revision 3
    let rules = at(&results, 4);
    assert_eq!(rules["ok"], true, "{rules:#?}");
    assert_eq!(rules["store_revision"], 3);
    let rule = &rules["rules"][0];
    assert_eq!(rule["id"], "judged.vague-clause");
    assert_eq!(rule["subject"], "clause");
    assert_eq!(rule["level"], "warning");
    assert_eq!(rule["state"], "never");
    assert_eq!(
        rule["question"],
        "Can a case be written from this clause alone?"
    );
    assert_eq!(rule["when"], serde_json::json!(["should"]));
    assert_eq!(rule["hooks"], serde_json::json!(["put", "check"]));
    let model = &rules["model"];
    assert_eq!(model["version"], 1);
    assert_eq!(model["store_revision"], 3);
    let contract = &model["contexts"][0]["contracts"][0];
    assert_eq!(contract["name"], "rounding");
    assert_eq!(contract["clauses"][0]["label"], "C1");
    assert_eq!(contract["clauses"][0]["text"], "a total should be rounded");
    assert_eq!(contract["clauses"][0]["revision"], 1);

    // reported: a finding like the builtin ones, with who judged it
    assert_eq!(at(&results, 5)["ok"], true, "{:#?}", at(&results, 5));
    assert_eq!(at(&results, 5)["findings"], 1);
    let first = judged(at(&results, 6));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["level"], "warning");
    assert_eq!(first[0]["state"], "open");
    assert_eq!(first[0]["about"], "rounding C1");
    assert_eq!(first[0]["judged"]["judge"], "agent-1");
    assert_eq!(first[0]["judged"]["read_at"], 3);
    assert_eq!(first[0].get("stale"), None);

    // the clause is reworded: the finding is still there, stale, and status says what to judge again
    let stale = judged(at(&results, 8));
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0]["stale"], true);
    assert_eq!(
        at(&results, 9)["judged"],
        serde_json::json!([{
            "rule": "judged.vague-clause",
            "summary": "a clause too vague for a case to check",
            "subject": "clause",
            "state": "stale"
        }])
    );

    // judged again with nothing found: the finding is gone, and nothing is left to judge
    assert_eq!(at(&results, 10)["replaced"], 1);
    assert!(judged(at(&results, 11)).is_empty());
    assert_eq!(at(&results, 12)["judged"], serde_json::json!([]));

    // a builtin check takes no report; a rule filtered by hook: this one belongs to put and check
    assert_eq!(at(&results, 13)["ok"], false);
    assert!(
        at(&results, 13)["error"]
            .as_str()
            .expect("error")
            .contains("not judged")
    );
    assert_eq!(at(&results, 14)["rules"], serde_json::json!([]));
}

#[test]
fn a_judged_rule_that_asks_to_be_mandatory_or_is_malformed_stops_the_run() {
    for (name, text, expect) in [
        (
            "mandatory",
            format!("{VAGUE_RULE}mandatory = true\n"),
            "cannot be mandatory",
        ),
        (
            "prefix",
            VAGUE_RULE.replace("judged.vague", "cite.vague"),
            "starts with `judged.`",
        ),
        ("toml", "id = ".to_string(), "vague.toml"),
    ] {
        let dir = fresh_dir(&format!("judged-bad-{name}"));
        std::fs::create_dir_all(dir.join("checks/judged")).expect("a directory");
        std::fs::write(dir.join("checks/judged/vague.toml"), text).expect("a file");
        let (results, code, stderr) = exec_in(&dir, "{\"op\":\"init\"}\n");
        assert_eq!(code, Some(2), "{name}");
        assert!(results.is_empty(), "{name}");
        assert!(stderr.contains(expect), "{name}: {stderr}");
    }
}

#[test]
fn a_policy_holds_a_judged_rule_like_any_check() {
    let dir = fresh_dir("judged-policy");
    std::fs::create_dir_all(dir.join("checks/judged")).expect("a directory");
    std::fs::write(dir.join("checks/judged/vague-clause.toml"), VAGUE_RULE).expect("a file");
    std::fs::write(
        dir.join("checks.toml"),
        "[checks.\"judged.\"]\nlevel = \"off\"\n",
    )
    .expect("a file");
    let ops = "{\"op\":\"init\"}\n{\"op\":\"check_rules\"}\n{\"op\":\"check_list\"}\n";
    let (results, code, _) = exec_in(&dir, ops);
    assert_eq!(code, Some(0), "{results:#?}");
    assert_eq!(at(&results, 2)["rules"], serde_json::json!([]));
    let listed = at(&results, 3)["checks"].as_array().expect("checks");
    let entry = listed
        .iter()
        .find(|c| c["id"] == "judged.vague-clause")
        .expect("listed");
    assert_eq!(
        (entry["executor"].as_str(), entry["level"].as_str()),
        (Some("judged"), Some("off"))
    );
    assert_eq!(entry["default_level"], "warning");
    assert_eq!(entry["mandatory"], false);
}
