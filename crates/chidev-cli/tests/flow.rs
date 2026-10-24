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

#[test]
#[cfg(unix)]
fn external_runs_are_explicit_persisted_and_return_gate_failure_evidence() {
    use std::os::unix::fs::PermissionsExt;
    let dir = fresh_dir("external-executor");
    let checker = dir.join("checker");
    std::fs::write(&checker, concat!(
        "#!/bin/sh\ncat > request.json\n",
        "echo run >> runs\n",
        "printf '%s' '{\"protocol\":1,\"findings\":[{\"rule\":\"ext.contract\",\"message\":\"requires review\"}]}'\n"
    )).unwrap();
    std::fs::set_permissions(&checker, std::fs::Permissions::from_mode(0o755)).unwrap();
    let registration = concat!(
        "[external.\"ext.contract\"]\ncommand='./checker'\n",
        "hooks=['check','put']\nreads=['contracts']\ngate=false\n",
        "params = { strict = true }\n[checks.\"ext.contract\".params]\nstrict = false\n"
    );
    std::fs::write(dir.join("checks.toml"), registration).unwrap();
    let (results, code, err) = exec_in(
        &dir,
        concat!(
            "{\"op\":\"init\"}\n{\"op\":\"context_add\",\"name\":\"core\"}\n",
            "{\"op\":\"status\"}\n{\"op\":\"check\"}\n{\"op\":\"check_list\"}\n",
            "{\"op\":\"check_run\"}\n{\"op\":\"status\"}\n"
        ),
    );
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert_eq!(results[2]["external"][0]["state"], "unrun");
    assert!(
        results[4]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == "ext.contract" && r["executor"] == "external")
    );
    assert_eq!(results[5]["external"][0]["findings"][0]["level"], "warning");
    assert_eq!(results[6]["external"][0]["state"], "current");
    assert_eq!(std::fs::read_to_string(dir.join("runs")).unwrap(), "run\n");
    let input: Value =
        serde_json::from_slice(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    assert_eq!(input["protocol"], 1);
    assert_eq!(input["params"]["strict"], false);
    assert!(input["model"]["contexts"][0].get("interfaces").is_none());
    let (results, code, err) = exec_in(&dir, "{\"op\":\"status\"}\n");
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(results[0]["external"][0]["state"], "current");
    assert_eq!(std::fs::read_to_string(dir.join("runs")).unwrap(), "run\n");

    std::fs::write(
        dir.join("checks.toml"),
        registration.replace("gate=false", "gate=true"),
    )
    .unwrap();
    let (results, code, err) = exec_in(
        &dir,
        concat!(
            "{\"op\":\"contract_put\",\"context\":\"core\",\"name\":\"rules\",\"title\":\"Rules\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a rule\"}]}\n",
            "{\"op\":\"status\"}\n"
        ),
    );
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    assert_eq!(results[0]["ok"], false);
    assert_eq!(results[0]["external"][0]["findings"][0]["level"], "error");
    assert!(
        results[0]["external"][0]["evidence"]["command_digest"]
            .as_str()
            .is_some_and(|s| s.len() == 64)
    );
    assert_eq!(results[1]["external"][0]["state"], "stale");
}

#[test]
fn malformed_external_registration_stops_before_initializing_a_store() {
    let dir = fresh_dir("external-invalid");
    std::fs::write(
        dir.join("checks.toml"),
        "[external.\"ext.bad\"]\ncommand='anything'\nhooks=['check']\nreads=['unknown']\n",
    )
    .unwrap();
    let (results, code, err) = exec_in(&dir, "{\"op\":\"init\"}\n");
    assert_eq!(code, Some(2));
    assert!(results.is_empty());
    assert!(err.contains("reads"), "{err}");
    assert!(!dir.join("chidev.db").exists());
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
            "{{\"op\":\"finding_report\",\"rule\":\"judged.vague-clause\",\"rule_digest\":\"RULE_DIGEST\",\"judge\":\"agent-1\",\"read_at\":3,",
            "\"findings\":[{{\"subject\":{clause},\"message\":\"`should` states no condition\"}}]}}\n",
            "{{\"op\":\"check\"}}\n",
            "{{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"rounding\",\"title\":\"Rounding\",",
            "\"clauses\":[{{\"label\":\"C1\",\"text\":\"a total is rounded to whole cents\"}}]}}\n",
            "{{\"op\":\"check\"}}\n",
            "{{\"op\":\"status\"}}\n",
            "{{\"op\":\"finding_report\",\"rule\":\"judged.vague-clause\",\"rule_digest\":\"RULE_DIGEST\",\"judge\":\"agent-1\",\"read_at\":5,\"findings\":[]}}\n",
            "{{\"op\":\"check\"}}\n",
            "{{\"op\":\"status\"}}\n",
            "{{\"op\":\"finding_report\",\"rule\":\"cite.stale\",\"rule_digest\":\"RULE_DIGEST\",\"judge\":\"agent-1\",\"read_at\":6,\"findings\":[]}}\n",
            "{{\"op\":\"check_rules\",\"hook\":\"pin\"}}\n",
        ),
        clause = clause
    );
    // Feed the exact digest the command returned, so the test exercises the round trip.
    let lines: Vec<_> = ops.lines().collect();
    let (mut results, setup_code, setup_stderr) = exec_in(&dir, &lines[..4].join("\n"));
    assert_eq!(setup_code, Some(0), "{setup_stderr}\n{results:#?}");
    let digest = at(&results, 4)["rules"][0]["rule_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(digest.len(), 64);
    let rest = lines[4..].join("\n").replace("RULE_DIGEST", &digest);
    let (mut later, code, stderr) = exec_in(&dir, &rest);
    for result in &mut later {
        result["line"] = serde_json::json!(result["line"].as_u64().unwrap() + 4);
    }
    results.extend(later);
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
    assert_eq!(at(&results, 5)["rule_digest"], digest);
    assert_eq!(at(&results, 5)["state"], "current");
    let first = judged(at(&results, 6));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["level"], "warning");
    assert_eq!(first[0]["state"], "open");
    assert_eq!(first[0]["about"], "rounding C1");
    assert_eq!(first[0]["judged"]["judge"], "agent-1");
    assert_eq!(first[0]["judged"]["read_at"], 3);
    assert_eq!(first[0]["judged"]["rule_digest"], digest);
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

fn address_setup() -> String {
    use serde_json::json;
    [
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"contract_put","context":"core","name":"rules","title":"Rules","clauses":[{"label":"C1","text":"a value is returned"}]}),
        json!({"op":"interface_put","context":"core","name":"api","source":"fn value();\n","items":[{"signature":"fn value()","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"tests","source":"fn checks_value() {}\n","cases":[{"name":"checks_value","lines":[1,1],"items":[{"interface":"api","signature":"fn value()"}],"clauses":[{"contract":"rules","label":"C1"}]}]}),
        json!({"op":"cr_open","title":"Existing request","intent":"keep_behavior"}),
    ]
    .into_iter()
    .map(|op| format!("{op}\n"))
    .collect()
}

#[test]
fn addresses_accept_every_subject_and_legacy_inputs_refer_to_the_same_finding() {
    use serde_json::json;
    let dir = fresh_dir("addresses-all-subjects");
    let item = json!({"context":"core","interface":"api","signature":"fn value()"});
    let clause = json!({"context":"core","contract":"rules","label":"C1"});
    let subjects = [
        json!({"context":"core"}),
        json!({"interface":{"context":"core","interface":"api"}}),
        json!({"item":item}),
        json!({"clause":clause}),
        json!({"case":{"context":"core","harness":"tests","name":"checks_value"}}),
        json!({"change_request":"Existing request"}),
        Value::Null,
    ];
    // Unregistered rule ids are valid: addresses can retain historical and prospective references.
    let addresses: Vec<_> = subjects
        .iter()
        .map(|subject| json!({"rule":"future.rule","subject":subject}))
        .collect();
    let mut ops = address_setup();
    ops.push_str(&format!("{}\n", json!({"op":"cr_open","title":"All subjects","intent":"keep_behavior","addresses":addresses})));
    ops.push_str(&format!(
        "{}\n",
        json!({"op":"cr_open","title":"Legacy parity","intent":"keep_behavior","addresses":[
            {"rule":"future.rule","item":item},
            {"rule":"future.rule","subject":{"item":item}},
            {"rule":"future.rule","clause":clause},
            {"rule":"future.rule","subject":{"clause":clause}},
            {"rule":"future.rule"},
            {"rule":"future.rule","subject":null},
            {"rule":"future.rule","item":null},
            {"rule":"future.rule","clause":null}
        ]})
    ));
    let (results, code, stderr) = exec_in(&dir, &ops);
    assert_eq!(code, Some(0), "{stderr}\n{results:#?}");
    // Read in a fresh process, verifying that the CLI has kept UUID subjects in the existing storage format.
    let (results, code, stderr) = exec_in(&dir, "{\"op\":\"check_rules\"}\n");
    assert_eq!(code, Some(0), "{stderr}");
    let model = &results[0]["model"];
    let crs = model["change_requests"].as_array().unwrap();
    let all = crs.iter().find(|cr| cr["title"] == "All subjects").unwrap();
    let addresses = all["addresses"].as_array().unwrap();
    assert_eq!(addresses.len(), 7);
    let context = &model["contexts"][0];
    let existing = crs
        .iter()
        .find(|cr| cr["title"] == "Existing request")
        .unwrap();
    let expected = [
        ("context", context["id"].clone()),
        ("interface", context["interfaces"][0]["id"].clone()),
        ("item", context["interfaces"][0]["items"][0]["id"].clone()),
        (
            "clause",
            context["contracts"][0]["clauses"][0]["id"].clone(),
        ),
        ("case", context["harnesses"][0]["cases"][0]["id"].clone()),
        ("change_request", existing["id"].clone()),
    ];
    for (address, (kind, id)) in addresses.iter().zip(expected) {
        assert_eq!(address["rule"], "future.rule");
        assert_eq!(address["subject"], json!({"kind":kind,"id":id}));
    }
    assert_eq!(addresses[6]["subject"], Value::Null);
    let parity = crs
        .iter()
        .find(|cr| cr["title"] == "Legacy parity")
        .unwrap();
    assert_eq!(parity["addresses"].as_array().unwrap().len(), 3);
    assert_eq!(parity["addresses"][0], addresses[2]);
    assert_eq!(parity["addresses"][1], addresses[3]);
    assert_eq!(parity["addresses"][2], addresses[6]);
}

#[test]
fn malformed_address_selectors_never_fall_back_to_the_project_or_write_a_request() {
    let dir = fresh_dir("addresses-invalid-shapes");
    let mut ops = address_setup();
    // Raw JSON preserves duplicate keys, which a serde_json::Value would otherwise discard.
    let invalid = [
        r#"{"rule":"future.rule","subjet":null}"#,
        r#"{"rule":"future.rule","subject":{"unknown":"core"}}"#,
        r#"{"rule":"future.rule","subject":{}}"#,
        r#"{"rule":"future.rule","subject":{"context":null}}"#,
        r#"{"rule":"future.rule","subject":{"context":"core","change_request":"Existing request"}}"#,
        r#"{"rule":"future.rule","subject":{"interface":{"context":"core","interface":"api","extra":null}}}"#,
        r#"{"rule":"future.rule","subject":{"case":{"context":"core","harness":"tests","name":"checks_value","extra":null}}}"#,
        r#"{"rule":"future.rule","subject":{"item":{"context":"core","interface":"api","signature":"fn value()","extra":null}}}"#,
        r#"{"rule":"future.rule","subject":{"clause":{"context":"core","contract":"rules","label":"C1","extra":null}}}"#,
        r#"{"rule":"future.rule","subject":null,"item":null}"#,
        r#"{"rule":"future.rule","subject":null,"clause":null}"#,
        r#"{"rule":"future.rule","item":null,"clause":null}"#,
        r#"{"rule":"future.rule","subject":{"context":"core"},"item":null}"#,
        r#"{"rule":"future.rule","subject":null,"item":{"context":"core","interface":"api","signature":"fn value()"}}"#,
        r#"{"rule":"future.rule","subject":null,"subject":null}"#,
        r#"{"rule":"future.rule","item":null,"item":null}"#,
        r#"{"rule":"future.rule","clause":null,"clause":null}"#,
        r#"{"rule":"future.rule","rule":"future.other"}"#,
        r#"{"rule":"future.rule","item":{"context":"core","interface":"api","signature":"fn value()","extra":null}}"#,
        r#"{"rule":"future.rule","clause":{"context":"core","contract":"rules","label":"C1","extra":null}}"#,
    ];
    for (k, address) in invalid.iter().enumerate() {
        ops.push_str(&format!("{{\"op\":\"cr_open\",\"title\":\"Invalid {k}\",\"intent\":\"keep_behavior\",\"addresses\":[{address}]}}\n"));
    }
    ops.push_str("{\"op\":\"check_rules\"}\n");
    let (results, code, stderr) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{stderr}\n{results:#?}");
    assert!(results[..6].iter().all(|r| r["ok"] == true));
    for result in &results[6..6 + invalid.len()] {
        assert_eq!(result["ok"], false, "{result}");
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("not an operation"),
            "{result}"
        );
    }
    let last = results.last().unwrap();
    assert_eq!(last["store_revision"], 6);
    assert_eq!(
        last["model"]["change_requests"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn missing_address_targets_are_rejected_before_the_request_is_saved() {
    use serde_json::json;
    let dir = fresh_dir("addresses-missing-targets");
    let mut ops = address_setup();
    let invalid = [
        json!({"context":"missing"}),
        json!({"interface":{"context":"core","interface":"missing"}}),
        json!({"item":{"context":"core","interface":"api","signature":"fn missing()"}}),
        json!({"clause":{"context":"core","contract":"rules","label":"C2"}}),
        json!({"case":{"context":"core","harness":"tests","name":"missing"}}),
        json!({"change_request":"missing"}),
    ];
    for (k, subject) in invalid.iter().enumerate() {
        ops.push_str(&format!("{}\n", json!({"op":"cr_open","title":format!("Missing {k}"),"intent":"keep_behavior","addresses":[{"rule":"future.rule","subject":subject}]})));
    }
    ops.push_str("{\"op\":\"check_rules\"}\n");
    let (results, code, stderr) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{stderr}\n{results:#?}");
    assert!(results[..6].iter().all(|r| r["ok"] == true));
    for result in &results[6..6 + invalid.len()] {
        assert_eq!(result["ok"], false, "{result}");
    }
    let last = results.last().unwrap();
    assert_eq!(last["store_revision"], 6);
    assert_eq!(
        last["model"]["change_requests"].as_array().unwrap().len(),
        1
    );
}

fn versioned_rule_project(name: &str) -> (PathBuf, String, u64) {
    let dir = fresh_dir(name);
    std::fs::create_dir_all(dir.join("checks/judged")).unwrap();
    std::fs::write(dir.join("checks/judged/rule.toml"), VAGUE_RULE).unwrap();
    let (results, code, stderr) =
        exec_in(&dir, &format!("{CHECK_OPS}{{\"op\":\"check_rules\"}}\n"));
    assert_eq!(code, Some(0), "{stderr}\n{results:#?}");
    let view = results.last().unwrap();
    let digest = view["rules"][0]["rule_digest"].as_str().unwrap().to_owned();
    (dir, digest, view["store_revision"].as_u64().unwrap())
}

fn judged_report(digest: Option<Value>, read_at: u64, findings: Vec<Value>) -> Value {
    let mut report = serde_json::json!({
        "op":"finding_report", "rule":"judged.vague-clause", "judge":"reviewer", "read_at":read_at,
        "findings":findings
    });
    if let Some(digest) = digest {
        report["rule_digest"] = digest;
    }
    report
}

fn clause_finding() -> Value {
    serde_json::json!({
        "subject":{"clause":{"context":"pricing","contract":"rounding","label":"C1"}},
        "message":"requires a precise condition"
    })
}

fn run_values(dir: &Path, ops: &[Value]) -> (Vec<Value>, Option<i32>, String) {
    let lines = ops
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    exec_in(dir, &lines)
}

#[test]
fn a_changed_rule_stales_findings_and_empty_reports_and_refuses_wrong_versions_atomically() {
    use serde_json::json;
    let (dir, digest, read_at) = versioned_rule_project("rule-version-change");
    let (first, code, stderr) = run_values(
        &dir,
        &[
            judged_report(Some(json!(digest)), read_at, vec![clause_finding()]),
            json!({"op":"check_rules"}),
            json!({"op":"check"}),
        ],
    );
    assert_eq!(code, Some(0), "{stderr}\n{first:#?}");
    assert_eq!(first[0]["rule_digest"], digest);
    assert_eq!(first[0]["state"], "current");
    assert_eq!(first[1]["rules"][0]["state"], "current");
    assert_eq!(judged(&first[2])[0].get("stale"), None);
    let saved_at = first[1]["store_revision"].as_u64().unwrap();
    let changed = VAGUE_RULE.replace("Can a case be written", "Must a case be written");
    std::fs::write(dir.join("checks/judged/rule.toml"), &changed).unwrap();
    let (changed_view, code, stderr) = run_values(
        &dir,
        &[
            json!({"op":"check_rules"}),
            json!({"op":"check"}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(code, Some(0), "{stderr}\n{changed_view:#?}");
    assert_eq!(changed_view[0]["store_revision"], saved_at);
    let changed_digest = changed_view[0]["rules"][0]["rule_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(changed_digest, digest);
    assert_eq!(changed_view[0]["rules"][0]["state"], "stale");
    assert_eq!(changed_view[0]["rules"][0]["judged"]["rule_digest"], digest);
    assert_eq!(judged(&changed_view[1])[0]["stale"], true);
    assert_eq!(judged(&changed_view[1])[0]["judged"]["rule_digest"], digest);
    assert_eq!(changed_view[2]["judged"][0]["state"], "stale");

    for wrong in [
        digest.clone(),
        String::new(),
        "invalid".into(),
        changed_digest.to_uppercase(),
        "0".repeat(63),
    ] {
        let (results, code, stderr) = run_values(
            &dir,
            &[
                judged_report(Some(json!(wrong)), saved_at, vec![]),
                json!({"op":"check_rules"}),
                json!({"op":"check"}),
            ],
        );
        assert_eq!(code, Some(1), "{stderr}\n{results:#?}");
        assert_eq!(results[0]["ok"], false);
        assert_eq!(results[1]["store_revision"], saved_at);
        assert_eq!(results[1]["rules"][0]["judged"]["rule_digest"], digest);
        assert_eq!(judged(&results[2]).len(), 1);
    }

    let (empty, code, stderr) = run_values(
        &dir,
        &[
            judged_report(Some(json!(changed_digest)), saved_at, vec![]),
            json!({"op":"check_rules"}),
            json!({"op":"check"}),
        ],
    );
    assert_eq!(code, Some(0), "{stderr}\n{empty:#?}");
    assert_eq!(empty[0]["state"], "current");
    assert_eq!(empty[0]["replaced"], 1);
    assert_eq!(empty[1]["rules"][0]["state"], "current");
    assert!(judged(&empty[2]).is_empty());
    std::fs::write(
        dir.join("checks/judged/rule.toml"),
        changed.replace("Must a case be written", "Could a case be written"),
    )
    .unwrap();
    let (results, code, stderr) =
        run_values(&dir, &[json!({"op":"check_rules"}), json!({"op":"check"})]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(results[0]["rules"][0]["state"], "stale");
    assert!(judged(&results[1]).is_empty());
}

#[test]
fn formatting_descriptive_fields_and_policy_levels_do_not_invalidate_a_judgment() {
    use serde_json::json;
    let (dir, digest, read_at) = versioned_rule_project("rule-version-formatting");
    let (results, code, stderr) = run_values(
        &dir,
        &[judged_report(
            Some(json!(digest)),
            read_at,
            vec![clause_finding()],
        )],
    );
    assert_eq!(code, Some(0), "{stderr}\n{results:#?}");
    let formatted = format!(
        "# A comment does not change the parsed rule.\n{}\n",
        VAGUE_RULE
            .replace("hooks = [\"put\", \"check\"]", "hooks = [ \"check\" ]")
            .replace(
                "a clause too vague",
                "a reworded summary of a clause too vague"
            )
            .replace("reword it with", "a reworded fix with")
            .replace("[\"should\"]", "[\"must\"]")
            .replace("level = \"warning\"", "level = \"error\"")
    );
    std::fs::write(dir.join("checks/judged/rule.toml"), formatted).unwrap();
    std::fs::write(
        dir.join("checks.toml"),
        "[checks.\"judged.vague-clause\"]\nlevel='note'\n",
    )
    .unwrap();
    let (results, code, stderr) = run_values(
        &dir,
        &[
            json!({"op":"check_rules"}),
            json!({"op":"check"}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(code, Some(0), "{stderr}\n{results:#?}");
    assert_eq!(results[0]["rules"][0]["rule_digest"], digest);
    assert_eq!(results[0]["rules"][0]["state"], "current");
    assert_eq!(results[0]["rules"][0]["level"], "note");
    assert_eq!(judged(&results[1])[0]["level"], "note");
    assert_eq!(judged(&results[1])[0].get("stale"), None);
    assert_eq!(results[2]["judged"], json!([]));
}

#[test]
fn a_missing_null_or_empty_rule_version_is_refused_and_replaces_nothing() {
    use serde_json::json;
    let (dir, digest, read_at) = versioned_rule_project("rule-version-required");
    let (saved, code, stderr) = run_values(
        &dir,
        &[
            judged_report(Some(json!(digest)), read_at, vec![clause_finding()]),
            json!({"op":"check_rules"}),
        ],
    );
    assert_eq!(code, Some(0), "{stderr}\n{saved:#?}");
    let at = saved[1]["store_revision"].as_u64().unwrap();
    for supplied in [None, Some(Value::Null), Some(json!(""))] {
        let (results, code, stderr) = run_values(
            &dir,
            &[
                judged_report(supplied, at, vec![]),
                json!({"op":"check_rules"}),
                json!({"op":"check"}),
            ],
        );
        assert_eq!(code, Some(1), "{stderr}\n{results:#?}");
        assert_eq!(results[0]["ok"], false);
        assert_eq!(results[1]["store_revision"], at);
        assert_eq!(results[1]["rules"][0]["state"], "current");
        assert_eq!(results[1]["rules"][0]["judged"]["rule_digest"], digest);
        assert_eq!(judged(&results[2]).len(), 1);
    }
}
