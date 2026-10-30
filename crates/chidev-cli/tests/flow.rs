//! Runs `tests/flow.jsonl` through `chidev exec`: two Contexts get their Contract, Interfaces and Harnesses, are
//! pinned; a change request is opened, with a part whose Crux a person decides once, on a stated basis; the
//! change is made and both change requests resolved; the Pins and citations go stale, the Harnesses are brought up
//! to date, the code is pinned again, and a change that would orphan a citation is refused. Each step reports what the
//! checks find: the Clause the new contract gains is cited by no Case until the Harness is brought up, and a change
//! request opened for it turns the finding from open to addressed.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A directory of its own under cargo's temporary one, with an empty `chidev.toml` (it is a project's root) and
/// nothing else, so that a run starts from no store.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("chidev.toml"), "").expect("a file");
    dir
}

/// `chidev exec` of `file` with `dir` as the project's root, on `store`.
fn exec(dir: &Path, file: &Path, store: &Path) -> (Vec<Value>, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(dir)
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

/// The operations of `ops` (each a line of JSON), each run through a `chidev exec` of its own on `store`, so that
/// one can use an id an earlier one gave back: `@N.crux` is the id of the Crux that the operation on line N raised,
/// and `@N.option(choice)` the id of its option with that choice. The results are numbered as if the operations were
/// one file, and the code is 1 when any of them was refused, else 0.
fn exec_each(dir: &Path, store: &Path, ops: &[String]) -> (Vec<Value>, Option<i32>) {
    let mut results: Vec<Value> = Vec::with_capacity(ops.len());
    for (k, op) in ops.iter().enumerate() {
        let file = dir.join("each.jsonl");
        std::fs::write(&file, format!("{}\n", with_ids(op.trim_end(), &results))).expect("a file");
        let (mut one, _) = exec(dir, &file, store);
        let mut result = one.remove(0);
        result["line"] = (k + 1).into();
        results.push(result);
    }
    let code = i32::from(results.iter().any(|r| r["ok"] == false));
    (results, Some(code))
}

/// `op` with the ids of earlier results written in (see [`exec_each`]).
fn with_ids(op: &str, results: &[Value]) -> String {
    let mut out = String::new();
    let mut rest = op;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let digits = tail
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(tail.len());
        let n: usize = tail[..digits].parse().expect("a line number after `@`");
        let result = &results[n - 1];
        let tail = &tail[digits..];
        if let Some(t) = tail.strip_prefix(".crux") {
            out.push_str(result["crux"].as_str().expect("the id of a Crux"));
            rest = t;
        } else if let Some(t) = tail.strip_prefix(".option(") {
            let end = t.find(')').expect("a closing parenthesis");
            let id = result["options"]
                .as_array()
                .expect("the options of a Crux")
                .iter()
                .find(|o| o["choice"] == t[..end])
                .and_then(|o| o["id"].as_str())
                .unwrap_or_else(|| panic!("no option `{}` in {result}", &t[..end]));
            out.push_str(id);
            rest = &t[end + 1..];
        } else {
            panic!("`@{n}` is followed by `.crux` or `.option(choice)`");
        }
    }
    out.push_str(rest);
    out
}

/// The flow, run on a store of its own.
fn run() -> (Vec<Value>, Option<i32>) {
    let dir = fresh_dir("flow");
    let ops: Vec<String> =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/flow.jsonl"))
            .expect("the flow")
            .lines()
            .map(str::to_string)
            .collect();
    exec_each(&dir, &dir.join("chidev.db"), &ops)
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
    assert_eq!(results.len(), 30);
    assert!(
        results[..29].iter().all(|r| r["ok"] == true),
        "{results:#?}"
    );

    // pinned against the first contract and interface: both fresh
    let first = at(&results, 12);
    assert_eq!(
        pin_states(first),
        states(&[("price", "fresh"), ("print", "fresh")])
    );

    // the request is open; its part decided, with the clause still to be written
    let raised = at(&results, 15);
    let crux = raised["crux"].as_str().expect("the id of the Crux");
    let half_to_even = raised["options"][1]["id"].as_str().expect("an option id");
    let decided = at(&results, 17);
    let crs = &decided["change_requests"];
    assert_eq!(crs[0]["title"], "Fair totals");
    assert_eq!(crs[0]["state"], "open");
    assert_eq!(crs[0]["cruxes"], serde_json::json!([]));
    assert_eq!(crs[1]["parent"], "Fair totals");
    assert_eq!(crs[1]["issuer"], "designer");
    assert_eq!(crs[1]["cruxes"], serde_json::json!([crux]));
    assert_eq!(crs[1]["state"], "decided");
    // the Crux is a record of its own, with the change request it stops, its options and its answer
    let cruxes = decided["cruxes"].as_array().expect("cruxes");
    assert_eq!(cruxes.len(), 1);
    let asked = &cruxes[0];
    assert_eq!(asked["id"], crux);
    assert_eq!(asked["raised_in"], "How is a half cent rounded?");
    assert_eq!(asked["kind"], "ambiguity");
    assert_eq!(asked["state"], "answered");
    assert_eq!(asked["recommended"]["option"], half_to_even);
    assert_eq!(asked["recommended"]["confidence"], 70);
    assert_eq!(asked["options"][1]["id"], half_to_even);
    assert_eq!(asked["options"][1]["choice"], "half to even");
    assert_eq!(
        asked["answer"]["decision"],
        serde_json::json!({ "chosen": {
            "option": half_to_even,
            "reason": "fair to the customer over many totals"
        } })
    );
    assert!(asked["answer"]["id"].is_string());
    assert_eq!(
        asked["answer"]["basis"],
        serde_json::json!({ "arbitrary": { "reason": "the shop accepts the cost of fairness" } })
    );
    assert_eq!(
        asked["answer"]["by"],
        serde_json::json!({ "kind": "person", "name": "alice" })
    );
    assert!(asked["answer"]["at"].as_u64().expect("a time") > 0);
    assert_eq!(decided["awaiting"], serde_json::json!([]));
    // the situation and the issuer given to cr_open are kept and shown
    assert_eq!(crs[0]["issuer"], "customer support");
    assert_eq!(
        crs[0]["situation"],
        "Customers say the totals favour the shop."
    );
    assert_eq!(
        crs[1]["situation"],
        "C1 rounds half up, which favours the shop."
    );
    // the part's target is not touched yet; the parent names none
    assert_eq!(crs[0]["targets"], serde_json::json!([]));
    assert_eq!(
        crs[1]["targets"],
        serde_json::json!([{ "target": "rounding C1", "progress": "not touched yet" }])
    );

    // the clause changed (line 18), so both change requests resolve (lines 21, 22); then the interface changes
    // too: the pins and the citations are behind, and the change requests stay resolved
    let changed = at(&results, 24);
    assert_eq!(changed["change_requests"][0]["state"], "resolved");
    assert_eq!(changed["change_requests"][1]["state"], "resolved");
    assert_eq!(
        changed["change_requests"][1]["resolution"]["resolution"],
        "done"
    );
    assert_eq!(
        changed["change_requests"][1]["targets"][0]["progress"],
        "revised"
    );
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
    assert!(changed.get("unapplied").is_none());
    // the interface was revised, not replaced; its item kept its id across the change of its declaration (the same key),
    // or line 23 would have been refused for orphaning the cases that cite it
    assert_eq!(at(&results, 23)["revision"], 2);
    assert_eq!(at(&results, 23)["id"], at(&results, 6)["id"]);

    // the new Clause C2 is cited by no Case once the contract is put (line 18), and the put does not say so (an
    // edit does not finish the model); the check (line 19) does: an open warning, with the name of the clause. The
    // change request opened for it (line 20) makes it addressed from then on, and it stays so while that request
    // is unresolved
    let uncited = |r: &Value| -> Option<Value> {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .find(|f| f["rule"] == "cite.uncited-clause" && f["about"] == "rounding C2")
            .cloned()
    };
    assert!(uncited(at(&results, 18)).is_none());
    let found = uncited(at(&results, 19)).expect("C2 is uncited");
    assert_eq!(found["level"], "warning");
    assert_eq!(found["state"], "open");
    assert_eq!(
        uncited(at(&results, 24)).expect("still uncited")["state"],
        "addressed"
    );
    // the harness put (line 25) cites it: the status after the pins (line 29) does not find it (a put never says
    // it, so the put itself proves nothing here)
    assert!(uncited(at(&results, 29)).is_none());
    // a pin made against what has moved is reported by the put that moved it, while the status says so for as
    // long as it lasts, and not once it is pinned again
    let pin_stale = |r: &Value| {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter(|f| f["rule"] == "pin.stale")
            .count()
    };
    assert_eq!(pin_stale(at(&results, 23)), 2);
    assert_eq!(pin_stale(at(&results, 24)), 2);
    assert_eq!(pin_stale(at(&results, 29)), 0);

    // the harnesses brought up to date and the code pinned again: everything is fresh
    let again = at(&results, 29);
    assert_eq!(
        pin_states(again),
        states(&[("price", "fresh"), ("print", "fresh")])
    );
    assert_eq!(again["stale_cites"], serde_json::json!([]));

    // dropping the item the harnesses cite is refused, and the run reports the failure
    let refused = at(&results, 30);
    assert_eq!(refused["ok"], false);
    assert!(message(refused).contains("cannot reach"), "{refused:#?}");
    assert_eq!(diag(refused)["code"], "project.cite-unreachable");
    assert_eq!(code, Some(1));
}

/// The contract of the change-request scenario: C1 is the settled change's clause, C2 the detail.
fn discount_put(c1: &str, c2: &str) -> String {
    format!(
        concat!(
            "{{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"discount\",\"title\":\"Discount\",",
            "\"clauses\":[{{\"label\":\"C1\",\"text\":\"{}\"}},{{\"label\":\"C2\",\"text\":\"{}\"}}]}}\n"
        ),
        c1, c2
    )
}

/// The same contract put as a change set applied for the change request titled `cr`.
fn discount_set(cr: &str, c1: &str, c2: &str) -> String {
    format!(
        concat!(
            "{{\"op\":\"change_set\",\"for\":\"{}\",\"puts\":[{{\"put\":\"contract\",\"context\":\"pricing\",",
            "\"name\":\"discount\",\"title\":\"Discount\",",
            "\"clauses\":[{{\"label\":\"C1\",\"text\":\"{}\"}},{{\"label\":\"C2\",\"text\":\"{}\"}}]}}]}}\n"
        ),
        cr, c1, c2
    )
}

#[test]
fn a_settled_change_and_a_question_in_one_change_request_both_have_to_be_applied() {
    let dir = fresh_dir("obligation");
    let ops = [
        "{\"op\":\"init\"}\n".to_string(),
        "{\"op\":\"context_add\",\"name\":\"pricing\"}\n".to_string(),
        discount_put(
            "any discount is accepted",
            "a refused discount is an InvalidDiscount",
        ),
        // 4: the settled change (C1, a line)
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Refuse big discounts\",\"intent\":\"change_behavior\",",
            "\"targets\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C1\"}}]}"
        )
        .to_string(),
        // 5: and the question about a detail (which error; C2 for a new one), raised in it
        concat!(
            "{\"op\":\"cr_crux_raise\",\"in\":\"Refuse big discounts\",\"question\":\"Which error is raised?\",",
            "\"kind\":\"ambiguity\",\"options\":[",
            "{\"choice\":\"reuse InvalidDiscount\",\"recommended\":{\"reason\":\"no new type\",\"confidence\":60}},",
            "{\"choice\":\"new error\",\"targets\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C2\"}}]}",
            "],\"importance\":\"normal\"}"
        )
        .to_string(),
        "{\"op\":\"status\"}".to_string(),
        "{\"op\":\"check\"}".to_string(),
        concat!(
            "{\"op\":\"cr_crux_decide\",\"crux\":\"@5.crux\",\"option\":\"@5.option(reuse InvalidDiscount)\",",
            "\"basis\":{\"arbitrary\":\"one error type is enough\"},\"by\":{\"person\":\"alice\"}}"
        )
        .to_string(),
        // 9: decided, yet C1 never changed: resolving looks at no revision, so it resolves, and the check says
        // that nothing was applied for it
        "{\"op\":\"cr_resolve\",\"title\":\"Refuse big discounts\",\"resolution\":\"done\",\"reason\":\"nothing to change after all\",\"by\":{\"person\":\"alice\"}}".to_string(),
        "{\"op\":\"status\"}".to_string(),
        // 11: a resolved request takes no more work
        discount_set(
            "Refuse big discounts",
            "a discount above 50% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        // 12: opened again, it takes the work, and is resolved anew
        "{\"op\":\"cr_reopen\",\"title\":\"Refuse big discounts\",\"reason\":\"the clause did need to change\",\"by\":{\"person\":\"bob\"}}".to_string(),
        discount_set(
            "Refuse big discounts",
            "a discount above 50% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        "{\"op\":\"cr_resolve\",\"title\":\"Refuse big discounts\",\"resolution\":\"done\",\"reason\":\"C1 was changed\",\"by\":{\"person\":\"bob\"}}".to_string(),
        // 15: a request about the Contract, and a question whose second option names C2 as a target: inside the frame
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Name the error\",\"intent\":\"change_behavior\",",
            "\"targets\":[{\"contract\":{\"context\":\"pricing\",\"contract\":\"discount\"}}]}"
        )
        .to_string(),
        concat!(
            "{\"op\":\"cr_crux_raise\",\"in\":\"Name the error\",\"question\":\"Is the error new?\",",
            "\"kind\":\"ambiguity\",\"options\":[",
            "{\"choice\":\"reuse InvalidDiscount\"},",
            "{\"choice\":\"new error\",\"targets\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C2\"}}]}",
            "]}"
        )
        .to_string(),
        // 17: done needs the question answered
        "{\"op\":\"cr_resolve\",\"title\":\"Name the error\",\"resolution\":\"done\",\"reason\":\"r\",\"by\":{\"person\":\"alice\"}}".to_string(),
        concat!(
            "{\"op\":\"cr_crux_decide\",\"crux\":\"@16.crux\",\"option\":\"@16.option(new error)\",",
            "\"basis\":{\"arbitrary\":\"callers tell the errors apart\"},\"by\":{\"person\":\"alice\"}}"
        )
        .to_string(),
        "{\"op\":\"status\"}".to_string(),
        // 20: C2 was never touched; resolved as done all the same, and the checks say so
        "{\"op\":\"cr_resolve\",\"title\":\"Name the error\",\"resolution\":\"done\",\"reason\":\"callers are fine\",\"by\":{\"person\":\"alice\"}}".to_string(),
        "{\"op\":\"status\"}".to_string(),
        // 22: an unknown field of cr_open is refused, the old lists included
        "{\"op\":\"cr_open\",\"title\":\"Old words\",\"intent\":\"change_behavior\",\"change\":[{\"project\":true}]}".to_string(),
    ];
    let (results, code) = exec_each(&dir, &dir.join("chidev.db"), &ops);
    assert_eq!(code, Some(1), "{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [11, 17, 22], "{results:#?}");

    // cr_open echoes what it recorded, by name
    let opened = at(&results, 4);
    assert_eq!(
        opened["expects"],
        serde_json::json!([{
            "kind": "target",
            "target": { "clause": { "context": "pricing", "contract": "discount", "label": "C1" } },
            "recorded": 1
        }])
    );

    // the question raised in it gives back its id and the id of each option
    let raised = at(&results, 5);
    let crux = raised["crux"].as_str().expect("the id of the Crux");
    let reuse = raised["options"][0]["id"].as_str().expect("an option id");
    let new_error = raised["options"][1]["id"].as_str().expect("an option id");
    assert_eq!(raised["options"][0]["choice"], "reuse InvalidDiscount");
    assert_eq!(raised["options"][1]["choice"], "new error");

    // the undecided question is visible: in status, and in check as a warning; the line is behind as well
    let waiting = at(&results, 6);
    assert_eq!(
        waiting["awaiting"],
        serde_json::json!([{
            "crux": crux,
            "raised_in": "Refuse big discounts",
            "question": "Which error is raised?",
            "kind": "ambiguity",
            "options": [
                { "id": reuse, "choice": "reuse InvalidDiscount", "wording": null, "expects": [],
                  "consequence": null },
                { "id": new_error, "choice": "new error", "wording": null,
                  "expects": [{ "kind": "target", "target": "discount C2" }], "consequence": null }
            ],
            "recommended": { "option": reuse, "reason": "no new type", "confidence": 60 },
            "importance": "normal"
        }])
    );
    // the request holds the id of its Crux, and the Crux is in the list of them
    assert_eq!(
        waiting["change_requests"][0]["cruxes"],
        serde_json::json!([crux])
    );
    assert_eq!(waiting["cruxes"][0]["id"], crux);
    assert_eq!(waiting["cruxes"][0]["state"], "open");
    assert_eq!(waiting["cruxes"][0]["answer"], Value::Null);
    assert_eq!(
        waiting["policy"],
        serde_json::json!({ "revision": 1, "lines": [] })
    );
    assert_eq!(
        waiting["change_requests"][0]["targets"],
        serde_json::json!([{ "target": "discount C1", "progress": "not touched yet" }])
    );
    assert!(waiting.get("unapplied").is_none());
    let finding = |r: &Value, rule: &str| -> Option<Value> {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .find(|f| f["rule"] == rule)
            .cloned()
    };
    let note = finding(at(&results, 7), "cr.awaiting-decision").expect("awaiting is found");
    assert_eq!(note["level"], "warning");
    // nothing judges the revisions any more: no check says the targets have not followed
    assert!(finding(at(&results, 7), "cr.unapplied").is_none());

    // decided, and C1 never changed: it resolves all the same, and the check says nothing was applied for it
    assert_eq!(at(&results, 8)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 9)["ok"], true, "{results:#?}");
    let unworked = finding(at(&results, 9), "cr.resolved-without-work").expect("found");
    assert_eq!(unworked["level"], "warning");
    let decided = at(&results, 10);
    assert_eq!(decided["awaiting"], serde_json::json!([]));
    let refuse = &decided["change_requests"][0];
    assert_eq!(refuse["resolution"]["resolution"], "done");
    assert_eq!(
        refuse["resolution"]["reason"],
        "nothing to change after all"
    );
    assert_eq!(
        refuse["resolution"]["by"],
        serde_json::json!({ "kind": "person", "name": "alice" })
    );
    assert_eq!(
        refuse["targets"],
        serde_json::json!([{ "target": "discount C1", "progress": "not touched yet" }])
    );
    // a resolved request takes no more work
    let closed = message(at(&results, 11));
    assert!(closed.contains("resolved"), "{closed}");
    assert_eq!(diag(at(&results, 11))["code"], "cr.already-resolved");

    // reopened it takes the work, and is resolved anew: no finding this time, and the history keeps all three steps
    assert_eq!(at(&results, 12)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 13)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 14)["ok"], true, "{results:#?}");
    assert!(finding(at(&results, 14), "cr.resolved-without-work").is_none());
    assert!(finding(at(&results, 14), "cr.targets-untouched").is_none());

    // the second question: done needs the answer; after the choice of the second option C2 is a target
    assert_eq!(at(&results, 17)["ok"], false);
    assert_eq!(diag(at(&results, 17))["code"], "cr.crux-unanswered");
    assert!(message(at(&results, 17)).contains("no decision"));
    assert_eq!(at(&results, 18)["ok"], true, "{results:#?}");
    let chosen = &at(&results, 19)["change_requests"][1];
    assert_eq!(chosen["state"], "decided");
    assert!(
        chosen["targets"].as_array().expect("targets").contains(
            &serde_json::json!({ "target": "discount C2", "progress": "not touched yet" })
        ),
        "{chosen:#?}"
    );
    // resolved as done with C2 untouched and no work at all: a finding for the missing work
    assert_eq!(at(&results, 20)["ok"], true, "{results:#?}");
    assert!(finding(at(&results, 20), "cr.resolved-without-work").is_some());
    let last = at(&results, 21);
    assert_eq!(last["awaiting"], serde_json::json!([]));
    assert_eq!(last["change_requests"][0]["state"], "resolved");
    assert_eq!(last["change_requests"][1]["state"], "resolved");
    let history = last["change_requests"][0]["history"]
        .as_array()
        .expect("history");
    assert_eq!(
        history
            .iter()
            .map(|s| s["step"].clone())
            .collect::<Vec<_>>(),
        ["resolved", "reopened", "resolved"]
    );
    assert_eq!(history[0]["reason"], "nothing to change after all");
    assert_eq!(history[1]["reason"], "the clause did need to change");
    // a reopened step has no resolution to name
    assert!(history[1].get("resolution").is_none() && history[1].get("of").is_none());
    assert_eq!(
        history[1]["by"],
        serde_json::json!({ "kind": "person", "name": "bob" })
    );
    assert_eq!(history[2]["reason"], "C1 was changed");
    assert_eq!(
        last["change_requests"][0]["resolution"]["reason"],
        "C1 was changed"
    );
    assert_eq!(
        last["change_requests"][0]["targets"],
        serde_json::json!([{ "target": "discount C1", "progress": "revised" }])
    );
    assert!(last.get("unapplied").is_none());

    // a field cr_open does not know is refused, whatever it was once called
    let old = message(at(&results, 22));
    assert!(old.contains("change"), "{old}");
    assert_eq!(diag(at(&results, 22))["code"], "input.unknown-field");
}

/// The e2e case "remove release": a parent without targets, a child with targets, one change set `for` the
/// child; the child and then the parent are resolved as done, and a change set `for` an unrelated request that
/// touches what another request frames moves a target outside the frame of the request it was made for.
#[test]
fn work_is_attributed_to_the_request_it_was_applied_for_and_stays_in_its_frame() {
    use serde_json::json;
    let dir = fresh_dir("attribution");
    let clause =
        |label: &str| json!({"clause":{"context":"core","contract":"stock","label":label}});
    let stock = |c1: &str, with_release: bool| {
        let mut clauses = vec![json!({"label":"C1","text":c1})];
        if with_release {
            clauses.push(json!({"label":"C7","text":"a unit is released"}));
        }
        json!({"put":"contract","context":"core","name":"stock","title":"Stock","clauses":clauses})
    };
    let interface = |with_release: bool| {
        let mut items = vec![json!({"key":"reserve","lines":[1,1]})];
        let mut source = "fn reserve()\n";
        if with_release {
            items.push(json!({"key":"release","lines":[2,2]}));
            source = "fn reserve()\nfn release()\n";
        }
        json!({"put":"interface","context":"core","name":"stock","source":source,"items":items})
    };
    let harness = |with_release: bool| {
        let mut cases = vec![json!({"name":"reserve_refuses","lines":[1,1],
            "items":[{"interface":"stock","key":"reserve"}],
            "clauses":[{"contract":"stock","label":"C1"}]})];
        let mut source = "fn reserve_refuses() {}\n";
        if with_release {
            cases.push(json!({"name":"release_frees","lines":[2,2],
                "items":[{"interface":"stock","key":"release"}],
                "clauses":[{"contract":"stock","label":"C7"}]}));
            source = "fn reserve_refuses() {}\nfn release_frees() {}\n";
        }
        json!({"put":"harness","context":"core","name":"stock-tests","source":source,"cases":cases})
    };
    let ops: String = [
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        // 3: the stock, in a change set for no request
        json!({"op":"change_set","puts":[stock("a unit is reserved", true), interface(true), harness(true)]}),
        // 4: the parent names no target: it is the request as the requester said it, and it is split
        json!({"op":"cr_open","title":"Remove release","intent":"change_behavior",
            "situation":"release is gone from the product","issuer":"product"}),
        // 5: the part names the three things that are release
        json!({"op":"cr_open","title":"Drop release","intent":"change_behavior","parent":"Remove release",
            "targets":[clause("C7"),
                      {"item":{"context":"core","interface":"stock","key":"release"}},
                      {"case":{"context":"core","harness":"stock-tests","name":"release_frees"}}]}),
        // 6, 7: two requests that have nothing to do with it
        json!({"op":"cr_open","title":"Reword C1","intent":"change_behavior","targets":[clause("C1")]}),
        json!({"op":"cr_open","title":"Unrelated","intent":"change_behavior",
            "targets":[{"harness":{"context":"core","harness":"stock-tests"}}]}),
        // 8: the removal, one change set for the part
        json!({"op":"change_set","for":"Drop release",
            "puts":[stock("a unit is reserved", false), interface(false), harness(false)]}),
        // 9, 10: the part is resolved as done, then the parent, which names nothing, once its part is
        json!({"op":"cr_resolve","title":"Drop release","resolution":"done",
            "reason":"release is gone from the stock","by":{"person":"alice"}}),
        json!({"op":"cr_resolve","title":"Remove release","resolution":"done",
            "reason":"its only part is done","by":{"person":"alice"}}),
        // 11: a change set for an unrelated request touches the clause that the first one reworded
        json!({"op":"change_set","for":"Unrelated","puts":[stock("a unit is reserved, once", false)]}),
        // 12: "Reword C1" is not done, and says why
        json!({"op":"cr_resolve","title":"Reword C1","resolution":"wont_do",
            "reason":"C1 was reworded under Unrelated","by":{"role":"orchestrator"}}),
        // 13, 14, 15: it comes back, takes its own work, and is done
        json!({"op":"cr_reopen","title":"Reword C1","reason":"that rewording was not the one asked for",
            "by":{"person":"alice"}}),
        json!({"op":"change_set","for":"Reword C1","puts":[stock("a unit is reserved, at most once", false)]}),
        json!({"op":"cr_resolve","title":"Reword C1","resolution":"done",
            "reason":"C1 says at most once","by":{"person":"alice"}}),
        // 16: what the set at 11 did is still found where the whole is checked
        json!({"op":"check"}),
        // 17, 18: a set for a request that is resolved, and for one that is not there
        json!({"op":"change_set","for":"Reword C1","puts":[stock("again", false)]}),
        json!({"op":"change_set","for":"No such request","puts":[stock("again", false)]}),
        // 19, 20: a reference has to exist; "Unrelated" is replaced by "Reword C1"
        json!({"op":"cr_resolve","title":"Unrelated","resolution":{"duplicate":"No such request"},
            "reason":"the same","by":{"person":"alice"}}),
        json!({"op":"cr_resolve","title":"Unrelated","resolution":{"superseded":"Reword C1"},
            "reason":"Reword C1 carries the wording","by":{"person":"alice"}}),
        json!({"op":"status"}),
    ]
    .iter()
    .map(|op| format!("{op}\n"))
    .collect();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [17, 18, 19], "{results:#?}");

    let out_of_frame = |r: &Value| -> Vec<Value> {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter(|f| f["rule"] == "cr.out-of-frame")
            .cloned()
            .collect()
    };
    let finding = |r: &Value, rule: &str| -> bool {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|f| f["rule"] == rule)
    };
    // the removal stays inside what the part named; the part resolves, and so does its parent, which names nothing
    assert!(
        out_of_frame(at(&results, 8)).is_empty(),
        "{:#?}",
        at(&results, 8)
    );
    assert_eq!(at(&results, 9)["ok"], true);
    assert_eq!(at(&results, 10)["ok"], true);
    // the part did the work, so the parent that closed after it has no finding for the lack of any
    assert!(!finding(at(&results, 9), "cr.resolved-without-work"));
    assert!(!finding(at(&results, 10), "cr.resolved-without-work"));
    assert!(!finding(at(&results, 10), "cr.targets-untouched"));

    // the set for the unrelated request is outside its frame (the harness), and says which request
    let outside = out_of_frame(at(&results, 11));
    assert_eq!(outside.len(), 1, "{:#?}", at(&results, 11));
    assert_eq!(outside[0]["level"], "warning");
    assert!(
        outside[0]["about"]
            .as_str()
            .expect("about")
            .contains("Unrelated"),
        "{outside:#?}"
    );

    // the clause moved, but not under "Reword C1": it is not done, and the record says so, with no finding
    // (a request that is not done is no claim of work)
    assert_eq!(at(&results, 12)["ok"], true, "{results:#?}");
    assert!(!finding(at(&results, 12), "cr.resolved-without-work"));
    // opened again it takes its own work and is done; the set for it moves nothing outside its own frame, and
    // the finding about the unrelated request, made at 11, is not this put's: it is not said again
    assert_eq!(at(&results, 13)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 14)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 15)["ok"], true, "{results:#?}");
    assert!(
        out_of_frame(at(&results, 14)).is_empty(),
        "{:#?}",
        at(&results, 14)
    );
    // where the whole is checked, only the unrelated request is outside its frame
    let still = out_of_frame(at(&results, 16));
    assert_eq!(still.len(), 1, "{still:#?}");
    assert!(
        still[0]["about"]
            .as_str()
            .expect("about")
            .contains("Unrelated")
    );
    // a resolved request takes no more work, a request has to exist, and so does the one a resolution points at
    assert!(error_at(&results, 17).contains("resolved"));
    assert!(error_at(&results, 18).contains("No such request"));
    assert!(error_at(&results, 19).contains("No such request"));
    assert_eq!(at(&results, 20)["ok"], true, "{results:#?}");

    // status: the footprints, and the move outside the frame, by name
    let status = at(&results, 21);
    let cr = |title: &str| -> &Value {
        status["change_requests"]
            .as_array()
            .expect("change requests")
            .iter()
            .find(|c| c["title"] == title)
            .unwrap_or_else(|| panic!("no change request {title}"))
    };
    let work = cr("Drop release")["work"].as_array().expect("work");
    assert_eq!(work.len(), 1);
    assert!(work[0]["store_revision"].as_u64().expect("revision") > 0);
    let moved: Vec<&str> = work[0]["moved"]
        .as_array()
        .expect("moved")
        .iter()
        .map(|m| m.as_str().expect("a target"))
        .collect();
    for target in [
        "stock C7 (retired)",
        "stock release (retired)",
        "case stock-tests release_frees (retired)",
        "contract stock",
        "interface stock",
        "harness stock-tests",
    ] {
        assert!(moved.contains(&target), "{target} in {moved:?}");
    }
    assert_eq!(cr("Drop release")["out_of_frame"], json!([]));
    // each target of the part reads retired, from the footprint of its work
    assert_eq!(
        cr("Drop release")["targets"],
        json!([
            { "target": "stock C7 (retired)", "progress": "retired" },
            { "target": "stock release (retired)", "progress": "retired" },
            { "target": "case stock-tests release_frees (retired)", "progress": "retired" }
        ])
    );
    // the parent names nothing and did no work of its own; its situation and issuer are kept
    let parent = cr("Remove release");
    assert_eq!(parent["work"], json!([]));
    assert_eq!(parent["targets"], json!([]));
    assert_eq!(parent["situation"], "release is gone from the product");
    assert_eq!(parent["issuer"], "product");
    assert_eq!(parent["resolution"]["resolution"], "done");
    assert_eq!(parent["resolution"]["reason"], "its only part is done");
    let unrelated = cr("Unrelated");
    assert_eq!(unrelated["work"].as_array().expect("work").len(), 1);
    // it was resolved (superseded) since: a resolved request is a record and is not reported for its work
    assert_eq!(unrelated["out_of_frame"], json!([]));
    // the harness the unrelated request names was changed by the part, not by its own work: not touched yet
    assert_eq!(
        unrelated["targets"],
        json!([{ "target": "harness stock-tests", "progress": "not touched yet" }])
    );
    assert_eq!(unrelated["resolution"]["resolution"], "superseded");
    assert_eq!(unrelated["resolution"]["of"], "Reword C1");
    // "Reword C1": not done, opened again, done; the history keeps all three and the target reads revised
    let reword = cr("Reword C1");
    let steps: Vec<(&str, Option<&str>)> = reword["history"]
        .as_array()
        .expect("history")
        .iter()
        .map(|s| (s["step"].as_str().expect("step"), s["resolution"].as_str()))
        .collect();
    assert_eq!(
        steps,
        [
            ("resolved", Some("wont_do")),
            ("reopened", None),
            ("resolved", Some("done"))
        ]
    );
    assert_eq!(
        reword["history"][0]["reason"],
        "C1 was reworded under Unrelated"
    );
    assert_eq!(
        reword["history"][0]["by"],
        json!({ "kind": "role", "name": "orchestrator" })
    );
    assert_eq!(
        reword["targets"],
        json!([{ "target": "stock C1", "progress": "revised" }])
    );
    assert!(status.get("unapplied").is_none());
}

fn policy_put(lines: &[(&str, &str)]) -> String {
    let lines: Vec<Value> = lines
        .iter()
        .map(|(id, text)| serde_json::json!({ "id": id, "text": text }))
        .collect();
    format!(
        "{}\n",
        serde_json::json!({ "op": "policy_put", "lines": lines })
    )
}

#[test]
fn a_crux_is_decided_once_on_a_basis_by_a_decision_maker_and_a_new_crux_supersedes_it() {
    let dir = fresh_dir("decisions");
    // a question raised in the change request titled `cr`, with the options `half up` and `half to even`
    let raise = |cr: &str, tail: &str| {
        format!(
            "{{\"op\":\"cr_crux_raise\",\"in\":\"{cr}\",\"question\":\"How is a half rounded?\",\"options\":[{{\"choice\":\"half up\"}},{{\"choice\":\"half to even\"{}}}],{tail}}}",
            if tail.contains("conflict") {
                ""
            } else {
                ",\"recommended\":{\"reason\":\"no bias\",\"confidence\":85}"
            }
        )
    };
    let open = |title: &str, intent: &str| {
        format!("{{\"op\":\"cr_open\",\"title\":\"{title}\",\"intent\":\"{intent}\"}}")
    };
    let ops = [
        "{\"op\":\"init\"}".to_string(),
        "{\"op\":\"context_add\",\"name\":\"pricing\"}".to_string(),
        discount_put("a half rounds up", "a refused discount is an InvalidDiscount"),
        // 4: the decision Policy
        policy_put(&[
            ("P1", "take the recommendation when its confidence is 80 or more"),
            ("P2", "when the importance is low and the options cost the same, either is fine"),
        ]),
        // 5, 6: a Crux whose recommendation is 85 sure
        open("Round half to even?", "change_behavior"),
        raise(
            "Round half to even?",
            "\"kind\":\"ambiguity\",\"importance\":\"low\"",
        ),
        // 7, 8: a Crux nothing derives
        open("Name the error", "change_behavior"),
        raise("Name the error", "\"kind\":\"conflict\",\"importance\":\"high\""),
        "{\"op\":\"status\"}".to_string(),
        // 10: the orchestrator applies P1
        concat!(
            "{\"op\":\"cr_crux_decide\",\"crux\":\"@6.crux\",\"option\":\"@6.option(half to even)\",",
            "\"basis\":{\"policy\":\"P1\"},\"by\":{\"role\":\"orchestrator\"}}"
        )
        .to_string(),
        // 11: a person decides what no line derives
        concat!(
            "{\"op\":\"cr_crux_decide\",\"crux\":\"@8.crux\",\"option\":\"@8.option(half up)\",",
            "\"reason\":\"keep what callers know\",",
            "\"basis\":{\"arbitrary\":\"the requester wants the old behaviour kept\"},\"by\":{\"person\":\"alice\"}}"
        )
        .to_string(),
        "{\"op\":\"status\"}".to_string(),
        // 13: an answer is never rewritten
        concat!(
            "{\"op\":\"cr_crux_decide\",\"crux\":\"@6.crux\",\"option\":\"@6.option(half up)\",",
            "\"basis\":{\"arbitrary\":\"changed my mind\"},\"by\":{\"person\":\"alice\"}}"
        )
        .to_string(),
        // 14, 15: another question replaces the first decision
        open("Round half up after all", "change_behavior"),
        raise(
            "Round half up after all",
            "\"kind\":\"ambiguity\",\"supersedes\":\"@6.crux\"",
        ),
        // 16, 17: a question that nobody has decided
        open("Plain", "keep_behavior"),
        raise("Plain", "\"kind\":\"ambiguity\""),
        // 18, 19: there is nothing to replace in a Crux that has no decision
        open("Bad supersede", "change_behavior"),
        raise(
            "Bad supersede",
            "\"kind\":\"ambiguity\",\"supersedes\":\"@17.crux\"",
        ),
        // 20: an Assumption appended to an open change request
        concat!(
            "{\"op\":\"cr_assume\",\"title\":\"Plain\",\"statement\":\"a half cent is a tie\",",
            "\"reason\":\"the rounding code treats it so\",\"evidence\":[",
            "{\"repo\":{\"path\":\"src/round.rs\",\"lines\":[10,14]}},",
            "{\"knowledge\":{\"reference\":\"IEEE 754 roundTiesToEven\"}}]}"
        )
        .to_string(),
        // 21: evidence that leaves the repository
        concat!(
            "{\"op\":\"cr_assume\",\"title\":\"Plain\",\"statement\":\"x\",\"reason\":\"y\",",
            "\"evidence\":[{\"repo\":{\"path\":\"../secret\"}}]}"
        )
        .to_string(),
        // 22: a Policy line the policy does not have
        concat!(
            "{\"op\":\"cr_crux_decide\",\"crux\":\"@15.crux\",\"option\":\"@15.option(half up)\",",
            "\"basis\":{\"policy\":\"P9\"},\"by\":{\"role\":\"harness-lead\"}}"
        )
        .to_string(),
        // 23: the harness lead applies P2
        concat!(
            "{\"op\":\"cr_crux_decide\",\"crux\":\"@15.crux\",\"option\":\"@15.option(half up)\",",
            "\"basis\":{\"policy\":\"P2\"},\"by\":{\"role\":\"harness-lead\"}}"
        )
        .to_string(),
        "{\"op\":\"status\"}".to_string(),
        // 25: the policy is replaced: P1 is reworded and P2 is gone
        policy_put(&[("P1", "take the recommendation when its confidence is 90 or more")]),
        "{\"op\":\"status\"}".to_string(),
    ];
    let (results, code) = exec_each(&dir, &dir.join("chidev.db"), &ops);
    assert_eq!(code, Some(1), "{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [13, 19, 21, 22], "{results:#?}");

    // the policy: revision 2 after the first put (1 is the empty one)
    assert_eq!(at(&results, 4)["revision"], 2);

    // the ids a raise gave back, and the Crux of a status by its id
    let id_of = |line: u64| {
        at(&results, line)["crux"]
            .as_str()
            .expect("an id")
            .to_string()
    };
    let option_of = |line: u64, choice: &str| -> String {
        at(&results, line)["options"]
            .as_array()
            .expect("options")
            .iter()
            .find(|o| o["choice"] == choice)
            .and_then(|o| o["id"].as_str())
            .expect("an option")
            .to_string()
    };
    let crux_in = |status: &Value, id: &str| -> Value {
        status["cruxes"]
            .as_array()
            .expect("cruxes")
            .iter()
            .find(|q| q["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no crux {id}"))
    };

    // both Cruxes are awaited, with what a decision maker needs to choose
    let waiting = at(&results, 9);
    let awaiting = waiting["awaiting"].as_array().expect("awaiting");
    assert_eq!(awaiting.len(), 2);
    let find = |id: &str| -> &Value {
        awaiting
            .iter()
            .find(|a| a["crux"] == id)
            .unwrap_or_else(|| panic!("{id} is not awaited"))
    };
    let derived = find(&id_of(6));
    assert_eq!(derived["raised_in"], "Round half to even?");
    assert_eq!(derived["question"], "How is a half rounded?");
    assert_eq!(derived["kind"], "ambiguity");
    assert_eq!(derived["importance"], "low");
    assert_eq!(
        derived["recommended"],
        serde_json::json!({
            "option": option_of(6, "half to even"),
            "reason": "no bias",
            "confidence": 85
        })
    );
    assert_eq!(derived["options"].as_array().expect("options").len(), 2);
    let free = find(&id_of(8));
    assert_eq!(free["recommended"], Value::Null);
    assert_eq!(free["importance"], "high");
    assert_eq!(free["kind"], "conflict");
    assert_eq!(
        waiting["policy"]["lines"][0],
        serde_json::json!({
            "id": "P1",
            "text": "take the recommendation when its confidence is 80 or more"
        })
    );
    assert_eq!(waiting["policy"]["revision"], 2);

    // status shows both answers: who decided, on what basis, and when
    let decided = at(&results, 12);
    assert_eq!(decided["awaiting"], serde_json::json!([]));
    let first = crux_in(decided, &id_of(6))["answer"].clone();
    assert_eq!(
        first["decision"],
        serde_json::json!({ "chosen": { "option": option_of(6, "half to even"), "reason": null } })
    );
    assert!(first["id"].is_string());
    assert_eq!(
        first["basis"],
        serde_json::json!({ "policy": { "line": "P1", "policy_revision": 2 } })
    );
    assert_eq!(
        first["by"],
        serde_json::json!({ "kind": "role", "name": "orchestrator" })
    );
    assert!(first["at"].as_u64().expect("a time") > 0);
    let second = crux_in(decided, &id_of(8))["answer"].clone();
    assert_eq!(
        second["decision"],
        serde_json::json!({ "chosen": {
            "option": option_of(8, "half up"),
            "reason": "keep what callers know"
        } })
    );
    assert_eq!(
        second["basis"],
        serde_json::json!({ "arbitrary": { "reason": "the requester wants the old behaviour kept" } })
    );
    assert_eq!(
        second["by"],
        serde_json::json!({ "kind": "person", "name": "alice" })
    );

    // the answer stands
    assert!(error_at(&results, 13).contains("answered already"));
    assert_eq!(diag(at(&results, 13))["code"], "cr.crux-answered");
    // nothing to replace, a Policy line that is not there, evidence outside the repository
    assert!(error_at(&results, 19).contains("no decision to replace"));
    assert_eq!(diag(at(&results, 19))["code"], "cr.supersedes-unanswered");
    assert!(error_at(&results, 21).contains(".."));
    assert!(error_at(&results, 22).contains("`P9`"));
    assert_eq!(diag(at(&results, 22))["code"], "cr.policy-line-unknown");

    // the third question supersedes the first decision, by its id; the Assumption is on its change request
    let later = at(&results, 24);
    let replacing = crux_in(later, &id_of(15));
    assert_eq!(replacing["supersedes"], id_of(6));
    assert_eq!(replacing["raised_in"], "Round half up after all");
    assert_eq!(replacing["state"], "answered");
    assert_eq!(
        replacing["answer"]["by"],
        serde_json::json!({ "kind": "role", "name": "harness-lead" })
    );
    let request = |title: &str| -> Value {
        later["change_requests"]
            .as_array()
            .expect("change requests")
            .iter()
            .find(|c| c["title"] == title)
            .cloned()
            .unwrap_or_else(|| panic!("no change request {title}"))
    };
    assert_eq!(request("Round half up after all")["state"], "decided");
    assert_eq!(
        request("Round half up after all")["cruxes"],
        serde_json::json!([id_of(15)])
    );
    // the question nobody decided keeps its request open and is still awaited
    assert_eq!(request("Plain")["state"], "open");
    assert_eq!(
        later["awaiting"]
            .as_array()
            .expect("awaiting")
            .iter()
            .map(|a| a["crux"].clone())
            .collect::<Vec<_>>(),
        [serde_json::json!(id_of(17))]
    );
    assert_eq!(
        request("Plain")["assumptions"],
        serde_json::json!([{
            "statement": "a half cent is a tie",
            "reason": "the rounding code treats it so",
            "evidence": [
                { "repo": { "path": "src/round.rs", "lines": [10, 14] } },
                { "knowledge": { "reference": "IEEE 754 roundTiesToEven" } }
            ]
        }])
    );

    // the policy changed: both decisions that rested on it are noted, and neither was rewritten
    let replaced = at(&results, 25);
    assert_eq!(replaced["revision"], 3);
    let gone: Vec<&Value> = replaced["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"] == "crux.policy-line-gone")
        .collect();
    assert_eq!(gone.len(), 2, "{replaced:#?}");
    assert!(gone.iter().all(|f| f["level"] == "note"));
    let after = at(&results, 26);
    assert_eq!(crux_in(after, &id_of(6))["answer"], first);
    assert_eq!(after["policy"]["lines"].as_array().expect("lines").len(), 1);
}

/// The first diagnostic of a refused result.
fn diag(result: &Value) -> &Value {
    assert_eq!(result["ok"], false, "{result:#?}");
    &result["diagnostics"][0]
}

/// The messages of every diagnostic of a refused result, one per line.
fn message(result: &Value) -> String {
    assert_eq!(result["ok"], false, "{result:#?}");
    result["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .map(|d| d["message"].as_str().expect("a message"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The messages of the diagnostics of the operation on line `n`.
fn error_at(results: &[Value], n: u64) -> String {
    message(at(results, n))
}

#[test]
fn a_context_is_added_and_a_clause_removed_through_change_requests() {
    let dir = fresh_dir("add-and-remove");
    let ops = [
        "{\"op\":\"init\"}\n".to_string(),
        "{\"op\":\"context_add\",\"name\":\"pricing\"}\n".to_string(),
        discount_put(
            "a discount above 50% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        // 4: adding a Contract is a change of the Context that holds it
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Add rules\",\"intent\":\"change_behavior\",",
            "\"targets\":[{\"context\":\"pricing\"}]}\n"
        )
        .to_string(),
        // 5: no Contract was added yet: the Context is not touched, and status says so
        "{\"op\":\"status\"}\n".to_string(),
        // 6: made for the request, so it shows in its targets
        concat!(
            "{\"op\":\"change_set\",\"for\":\"Add rules\",\"puts\":[{\"put\":\"contract\",\"context\":\"pricing\",",
            "\"name\":\"rules\",\"title\":\"Rules\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a rule holds\"}]}]}\n"
        )
        .to_string(),
        "{\"op\":\"status\"}\n".to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Add rules\",\"resolution\":\"done\",\"reason\":\"the contract is added\",\"by\":{\"person\":\"alice\"}}\n".to_string(),
        // 9: removing a Clause, while the other stays as it is
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Drop C2\",\"intent\":\"change_behavior\",",
            "\"targets\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C2\"}}],",
            "\"keep\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C1\"}}]}\n"
        )
        .to_string(),
        "{\"op\":\"change_set\",\"for\":\"Drop C2\",\"puts\":[{\"put\":\"contract\",\"context\":\"pricing\",\"name\":\"discount\",\"title\":\"Discount\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a discount above 50% is refused\"}]}]}\n"
            .to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Drop C2\",\"resolution\":\"done\",\"reason\":\"C2 is gone\",\"by\":{\"person\":\"alice\"}}\n".to_string(),
        "{\"op\":\"status\"}\n".to_string(),
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, Vec::<u64>::new(), "{results:#?}");

    assert_eq!(
        at(&results, 4)["expects"],
        serde_json::json!([{ "kind": "target", "target": { "context": "pricing" }, "recorded": 2 }])
    );
    // no Contract was added yet: the Context is not touched; then it is revised
    assert_eq!(
        at(&results, 5)["change_requests"][0]["targets"],
        serde_json::json!([{ "target": "context pricing", "progress": "not touched yet" }])
    );
    assert_eq!(
        at(&results, 7)["change_requests"][0]["targets"],
        serde_json::json!([{ "target": "context pricing", "progress": "revised" }])
    );

    // the Clause went: it reads retired, and the kept one did not move
    let dropped = &last_status(&results)["change_requests"][1];
    assert_eq!(
        dropped["targets"],
        serde_json::json!([{ "target": "discount C2 (retired)", "progress": "retired" }])
    );
    assert_eq!(
        dropped["keep"],
        serde_json::json!([{ "target": "discount C1", "moved": false }])
    );
    // (two Clauses are in the order of their ids, which are not known here)
    let recorded = at(&results, 9)["expects"].as_array().expect("expects");
    assert_eq!(recorded.len(), 2);
    for (kind, label) in [("target", "C2"), ("keep", "C1")] {
        assert!(
            recorded.contains(&serde_json::json!({
                "kind": kind,
                "target": { "clause": { "context": "pricing", "contract": "discount", "label": label } },
                "recorded": 1
            })),
            "{recorded:?}"
        );
    }

    let last = at(&results, 12);
    assert_eq!(last["change_requests"][0]["state"], "resolved");
    assert_eq!(last["change_requests"][1]["state"], "resolved");
    assert_eq!(last["keep_moved"], serde_json::json!([]));
}

/// The last `status` of a run.
fn last_status(results: &[Value]) -> &Value {
    results
        .iter()
        .rev()
        .find(|r| r["op"] == "status")
        .expect("a status")
}

#[test]
fn resolving_a_request_whose_intent_and_targets_disagree_is_told_so() {
    let dir = fresh_dir("intent-at-resolve");
    let ops = [
        "{\"op\":\"init\"}\n".to_string(),
        "{\"op\":\"context_add\",\"name\":\"pricing\"}\n".to_string(),
        discount_put(
            "a discount above 50% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        // 4: it keeps the behaviour, yet names a Clause to revise
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Keeps wrongly\",\"intent\":\"keep_behavior\",",
            "\"targets\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C1\"}}]}\n"
        )
        .to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Keeps wrongly\",\"resolution\":\"wont_do\",\"reason\":\"r\",\"by\":{\"person\":\"alice\"}}\n".to_string(),
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    // the resolution is told, although the request is resolved in the state the checks read
    let told = at(&results, 5)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .any(|f| f["rule"] == "cr.intent-mismatch");
    assert!(told, "{results:#?}");
}

#[test]
fn a_part_is_resolved_before_its_parent_and_a_kept_clause_must_stay_as_it_is() {
    let dir = fresh_dir("parts-and-keeps");
    let ops = [
        "{\"op\":\"init\"}\n".to_string(),
        "{\"op\":\"context_add\",\"name\":\"pricing\"}\n".to_string(),
        discount_put(
            "a discount above 50% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        "{\"op\":\"cr_open\",\"title\":\"Umbrella\",\"intent\":\"keep_behavior\"}\n".to_string(),
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Part\",\"intent\":\"keep_behavior\",\"parent\":\"Umbrella\",",
            "\"keep\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C1\"}}]}\n"
        )
        .to_string(),
        // 6: the parent waits for its part, however it would end
        "{\"op\":\"cr_resolve\",\"title\":\"Umbrella\",\"resolution\":\"wont_do\",\"reason\":\"r\",\"by\":{\"person\":\"alice\"}}\n".to_string(),
        discount_put(
            "a discount above 40% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        "{\"op\":\"status\"}\n".to_string(),
        "{\"op\":\"check\"}\n".to_string(),
        // 10: the clause it keeps moved: resolving is a decision, not a reading of the clause, so it resolves
        "{\"op\":\"cr_resolve\",\"title\":\"Part\",\"resolution\":\"done\",\"reason\":\"nothing else to do\",\"by\":{\"person\":\"alice\"}}\n".to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Umbrella\",\"resolution\":\"done\",\"reason\":\"its part is done\",\"by\":{\"person\":\"alice\"}}\n".to_string(),
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [6], "{results:#?}");

    let parent = error_at(&results, 6);
    assert!(
        parent.contains("not resolved") && parent.contains("Part"),
        "{parent}"
    );
    // a part that is open keeps its parent from ending in any way, but no revision is asked of the parent

    assert_eq!(
        at(&results, 8)["keep_moved"],
        serde_json::json!([{ "cr": "Part", "moved": ["discount C1"] }])
    );
    // the check says so too, as a warning
    let found = at(&results, 9)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|f| f["rule"] == "cr.keep-moved")
        .cloned()
        .expect("cr.keep-moved is found");
    assert_eq!(found["level"], "warning");

    // the keep is reported in status, with whether it moved, and does not refuse the resolution
    assert_eq!(
        at(&results, 8)["change_requests"][1]["keep"],
        serde_json::json!([{ "target": "discount C1", "moved": true }])
    );
    assert_eq!(at(&results, 10)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 11)["ok"], true, "{results:#?}");
    // the resolution itself is told the keep moved: the request is resolved in the state the checks read, and is
    // judged all the same
    let kept = at(&results, 10)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|f| f["rule"] == "cr.keep-moved")
        .cloned()
        .expect("cr.keep-moved is found when the request is resolved");
    assert_eq!(kept["level"], "warning");
    // no work was applied for either: the checks of the resolve hook say so
    let unworked = at(&results, 10)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|f| f["rule"] == "cr.resolved-without-work")
        .cloned()
        .expect("found");
    assert_eq!(unworked["level"], "warning");
}

/// Two parts of one request: one keeps an Interface, the other names an Item of it. Opening the second tells the
/// contradiction at once, about the part that keeps, naming the part that targets; resolving the one that keeps as
/// superseded by a part that keeps the Item alone clears it.
#[test]
fn a_part_that_names_what_a_sibling_keeps_is_told_when_it_is_opened_and_clears_when_the_keep_is_narrowed()
 {
    use serde_json::json;
    let dir = fresh_dir("keep-targeted");
    let ops: String = [
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"interface_put","context":"core","name":"accept",
            "source":"fn accept()\nfn limit()\n",
            "items":[{"key":"accept","lines":[1,1]},{"key":"limit","lines":[2,2]}]}),
        json!({"op":"cr_open","title":"Split","intent":"change_behavior"}),
        // 5: a part that keeps the whole Interface
        json!({"op":"cr_open","title":"Stop an example","intent":"keep_behavior","parent":"Split",
            "keep":[{"interface":{"context":"core","interface":"accept"}}]}),
        // 6: a part that names an Item of it
        json!({"op":"cr_open","title":"Set the time limit","intent":"change_behavior","parent":"Split",
            "targets":[{"item":{"context":"core","interface":"accept","key":"limit"}}]}),
        // 7: where the whole is checked
        json!({"op":"check"}),
        // 8, 9: the part that keeps is superseded by one that keeps the Item it needs
        json!({"op":"cr_open","title":"Stop an example, narrowed","intent":"keep_behavior","parent":"Split",
            "keep":[{"item":{"context":"core","interface":"accept","key":"accept"}}]}),
        json!({"op":"cr_resolve","title":"Stop an example",
            "resolution":{"superseded":"Stop an example, narrowed"},
            "reason":"it kept more than it needs","by":{"person":"alice"}}),
        json!({"op":"check"}),
    ]
    .iter()
    .map(|op| format!("{op}\n"))
    .collect();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    let targeted = |r: &Value| -> Vec<Value> {
        r["findings"]
            .as_array()
            .map(|all| {
                all.iter()
                    .filter(|f| f["rule"] == "cr.keep-targeted")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };
    // opening the part that keeps tells nothing yet
    assert!(
        targeted(at(&results, 5)).is_empty(),
        "{:#?}",
        at(&results, 5)
    );
    // opening the part that names the Item tells it, about the part that keeps
    let told = targeted(at(&results, 6));
    assert_eq!(told.len(), 1, "{:#?}", at(&results, 6));
    assert_eq!(told[0]["level"], "warning");
    assert!(
        told[0]["about"]
            .as_str()
            .expect("about")
            .contains("Stop an example"),
        "{told:#?}"
    );
    let said = told[0]["message"].as_str().expect("message");
    assert!(
        said.ends_with(
            ": change request `Set the time limit` targets accept limit (kept: interface accept)"
        ),
        "{said}"
    );
    // the whole is told the same until it is dealt with
    assert_eq!(targeted(at(&results, 7)).len(), 1, "{:#?}", at(&results, 7));
    // the narrower keep is not named by anyone, and the one it supersedes is a record
    assert!(
        targeted(at(&results, 8)).is_empty(),
        "{:#?}",
        at(&results, 8)
    );
    assert_eq!(at(&results, 9)["ok"], true, "{results:#?}");
    assert!(
        targeted(at(&results, 9)).is_empty(),
        "{:#?}",
        at(&results, 9)
    );
    assert!(
        targeted(at(&results, 10)).is_empty(),
        "{:#?}",
        at(&results, 10)
    );
}

/// A part names a Contract; the Harness put for it changes the Case that cites a Clause of the Contract and adds
/// one: that is the part's work, inside its frame, at the put and where the whole is checked. A put for another
/// request, whose Case cites a Clause outside its frame, is outside it, and resolving that request says so.
#[test]
fn the_cases_that_follow_a_framed_clause_are_inside_the_frame_and_resolving_tells_what_is_outside()
{
    use serde_json::json;
    let dir = fresh_dir("frame-follows-cites");
    let stock = |c1: &str| {
        json!({"put":"contract","context":"core","name":"stock","title":"Stock",
            "clauses":[{"label":"C1","text":c1},{"label":"C7","text":"a unit is released"}]})
    };
    let interface = json!({"put":"interface","context":"core","name":"stock",
        "source":"fn reserve()\nfn release()\n",
        "items":[{"key":"reserve","lines":[1,1]},{"key":"release","lines":[2,2]}]});
    let case = |name: &str, line: u64, item: &str, label: &str| {
        json!({"name":name,"lines":[line,line],"items":[{"interface":"stock","key":item}],
            "clauses":[{"contract":"stock","label":label}]})
    };
    let harness = |cases: Vec<Value>| {
        let source: String = cases
            .iter()
            .map(|c| format!("fn {}() {{}}\n", c["name"].as_str().expect("name")))
            .collect();
        json!({"put":"harness","context":"core","name":"stock-tests","source":source,"cases":cases})
    };
    let reserve_refuses = case("reserve_refuses", 1, "reserve", "C1");
    let reserve_twice = case("reserve_twice", 2, "reserve", "C1");
    let release_frees = case("release_frees", 3, "release", "C7");
    let reserve_thrice = case("reserve_thrice", 4, "reserve", "C1");
    let ops: String = [
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        // 3: the stock, in a change set for no request
        json!({"op":"change_set","puts":[stock("a unit is reserved"), interface,
            harness(vec![reserve_refuses.clone()])]}),
        // 4, 5: a part that names the Contract, and one that names the Clause C7 alone
        json!({"op":"cr_open","title":"Reword stock","intent":"change_behavior",
            "targets":[{"contract":{"context":"core","contract":"stock"}}]}),
        json!({"op":"cr_open","title":"Release only","intent":"change_behavior",
            "targets":[{"clause":{"context":"core","contract":"stock","label":"C7"}}]}),
        // 6: the Contract is reworded and the Harness put for it: the Case that cites C1 changes, one is added
        json!({"op":"change_set","for":"Reword stock","puts":[stock("a unit is reserved, once"),
            harness(vec![reserve_refuses.clone(), reserve_twice.clone()])]}),
        // 7: where the whole is checked
        json!({"op":"check"}),
        // 8: a Harness put for the other request adds a Case that cites C7 and a Case that cites C1 only
        json!({"op":"change_set","for":"Release only","puts":[
            harness(vec![reserve_refuses, reserve_twice, release_frees, reserve_thrice])]}),
        // 9: resolving it tells what is outside its frame
        json!({"op":"cr_resolve","title":"Release only","resolution":"done",
            "reason":"release is checked","by":{"person":"alice"}}),
    ]
    .iter()
    .map(|op| format!("{op}\n"))
    .collect();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    let outside = |r: &Value| -> Vec<Value> {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter(|f| f["rule"] == "cr.out-of-frame")
            .cloned()
            .collect()
    };
    let about = |f: &Value| f["about"].as_str().expect("about").to_string();
    // the put for the part is inside its frame: the Case it changed and the Case it added follow the Contract's Clause
    assert!(
        outside(at(&results, 6)).is_empty(),
        "{:#?}",
        at(&results, 6)
    );
    // and so it is where the whole is checked, for that request
    assert!(
        outside(at(&results, 7))
            .iter()
            .all(|f| !about(f).contains("Reword stock")),
        "{:#?}",
        at(&results, 7)
    );
    // the other request names C7: the Case that cites C7 is inside and covers the Harness, and the Case added that
    // cites C1 only is outside
    let at_put = outside(at(&results, 8));
    assert_eq!(at_put.len(), 1, "{:#?}", at(&results, 8));
    assert!(about(&at_put[0]).contains("Release only"));
    // resolving is told the same, as a warning
    let at_resolve = outside(at(&results, 9));
    assert_eq!(at_resolve.len(), 1, "{:#?}", at(&results, 9));
    assert_eq!(at_resolve[0]["level"], "warning");
    assert!(about(&at_resolve[0]).contains("Release only"));
}

#[test]
fn a_change_set_applied_again_for_a_request_is_ok_and_adds_no_work() {
    use serde_json::json;
    let dir = fresh_dir("change-set-again");
    let stock = json!({"put":"contract","context":"core","name":"stock","title":"Stock",
        "clauses":[{"label":"C1","text":"a unit is reserved"}]});
    let reword = json!({"op":"change_set","for":"Reword stock","puts":[
        {"put":"contract","context":"core","name":"stock","title":"Stock",
            "clauses":[{"label":"C1","text":"a unit is reserved, once"}]}]});
    let ops: String = [
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"change_set","puts":[stock]}),
        json!({"op":"cr_open","title":"Reword stock","intent":"change_behavior",
            "targets":[{"clause":{"context":"core","contract":"stock","label":"C1"}}]}),
        // 5, 6: the same change set twice: the second moves nothing
        reword.clone(),
        reword,
        json!({"op":"status"}),
    ]
    .iter()
    .map(|op| format!("{op}\n"))
    .collect();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert_eq!(at(&results, 5)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 6)["ok"], true, "{results:#?}");
    let status = at(&results, 7);
    let request = status["change_requests"]
        .as_array()
        .expect("change requests")
        .iter()
        .find(|c| c["title"] == "Reword stock")
        .expect("the request");
    // the work holds the change set that moved the Clause, once: the second one is no work
    let work = request["work"].as_array().expect("work");
    assert_eq!(work.len(), 1, "{work:#?}");
    assert!(
        !work[0]["moved"].as_array().expect("moved").is_empty(),
        "{work:#?}"
    );
}

#[test]
fn parts_that_share_a_harness_are_designed_side_by_side_and_each_put_moves_only_its_own_cases() {
    use serde_json::json;
    let dir = fresh_dir("shared-harness");
    let stock = |c1: &str, c2: &str| {
        json!({"put":"contract","context":"core","name":"stock","title":"Stock",
            "clauses":[{"label":"C1","text":c1},{"label":"C2","text":c2}]})
    };
    let interface = json!({"put":"interface","context":"core","name":"stock",
        "source":"fn reserve()\nfn release()\n",
        "items":[{"key":"reserve","lines":[1,1]},{"key":"release","lines":[2,2]}]});
    // Case `reserve_refuses` cites C1 (part A's Clause), `release_frees` cites C2 (part B's)
    let harness = json!({"put":"harness","context":"core","name":"stock-tests",
        "source":"fn reserve_refuses() {}\nfn release_frees() {}\n",
        "cases":[
            {"name":"reserve_refuses","lines":[1,1],"items":[{"interface":"stock","key":"reserve"}],
                "clauses":[{"contract":"stock","label":"C1"}]},
            {"name":"release_frees","lines":[2,2],"items":[{"interface":"stock","key":"release"}],
                "clauses":[{"contract":"stock","label":"C2"}]}]});
    let clause =
        |label: &str| json!({"clause":{"context":"core","contract":"stock","label":label}});
    let ops: String = [
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        // 3: the stock, in a change set for no request
        json!({"op":"change_set","puts":[stock("one", "two"), interface, harness.clone()]}),
        // 4, 5, 6: a parent and two parts, each part targeting one Clause
        json!({"op":"cr_open","title":"Rework stock","intent":"change_behavior",
            "situation":"both Clauses are reworded"}),
        json!({"op":"cr_open","title":"Reword C1","intent":"change_behavior","parent":"Rework stock",
            "targets":[clause("C1")]}),
        json!({"op":"cr_open","title":"Reword C2","intent":"change_behavior","parent":"Rework stock",
            "targets":[clause("C2")]}),
        // 7, 8: both designs, side by side, before any Harness is put
        json!({"op":"change_set","for":"Reword C1","puts":[stock("one, revised", "two")]}),
        json!({"op":"change_set","for":"Reword C2","puts":[stock("one, revised", "two, revised")]}),
        // 9: both Cases are stale
        json!({"op":"status"}),
        // 10, 11: the Harness put again, same code, for part A
        json!({"op":"change_set","for":"Reword C1","puts":[harness.clone()]}),
        json!({"op":"status"}),
        // 12, 13: and for part B
        json!({"op":"change_set","for":"Reword C2","puts":[harness]}),
        json!({"op":"status"}),
    ]
    .iter()
    .map(|op| format!("{op}\n"))
    .collect();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    let out_of_frame = |r: &Value| -> usize {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter(|f| f["rule"] == "cr.out-of-frame")
            .count()
    };
    let stale_cases = |status: &Value| -> Vec<String> {
        status["stale_cites"]
            .as_array()
            .expect("stale cites")
            .iter()
            .map(|c| c["case"].as_str().expect("a case").to_string())
            .collect()
    };
    let last_work = |status: &Value, title: &str| -> Vec<String> {
        let request = status["change_requests"]
            .as_array()
            .expect("change requests")
            .iter()
            .find(|c| c["title"] == title)
            .unwrap_or_else(|| panic!("no change request {title}"));
        let work = request["work"].as_array().expect("work");
        work.last().expect("some work")["moved"]
            .as_array()
            .expect("moved")
            .iter()
            .map(|m| m.as_str().expect("a target").to_string())
            .collect()
    };
    let out_of_frame_of = |status: &Value, title: &str| -> Value {
        status["change_requests"]
            .as_array()
            .expect("change requests")
            .iter()
            .find(|c| c["title"] == title)
            .unwrap_or_else(|| panic!("no change request {title}"))["out_of_frame"]
            .clone()
    };
    // both Cases are stale after the two designs
    assert_eq!(
        stale_cases(at(&results, 9)).len(),
        2,
        "{:#?}",
        at(&results, 9)
    );

    // the put for A brings up the Case that cites C1 and leaves the other where it was
    assert_eq!(at(&results, 10)["ok"], true, "{:#?}", at(&results, 10));
    assert_eq!(out_of_frame(at(&results, 10)), 0, "{:#?}", at(&results, 10));
    let after_a = at(&results, 11);
    let stale = stale_cases(after_a);
    assert_eq!(stale.len(), 1, "{after_a:#?}");
    assert!(stale[0].contains("release_frees"), "{stale:?}");
    let moved = last_work(after_a, "Reword C1");
    assert!(
        moved.iter().any(|m| m.contains("reserve_refuses")),
        "{moved:?}"
    );
    assert!(
        !moved.iter().any(|m| m.contains("release_frees")),
        "{moved:?}"
    );
    assert_eq!(out_of_frame_of(after_a, "Reword C1"), json!([]));
    assert_eq!(out_of_frame_of(after_a, "Reword C2"), json!([]));

    // the put for B brings up the other, and nothing is stale
    assert_eq!(at(&results, 12)["ok"], true, "{:#?}", at(&results, 12));
    assert_eq!(out_of_frame(at(&results, 12)), 0, "{:#?}", at(&results, 12));
    let after_b = at(&results, 13);
    assert!(stale_cases(after_b).is_empty(), "{after_b:#?}");
    let moved = last_work(after_b, "Reword C2");
    assert!(
        moved.iter().any(|m| m.contains("release_frees")),
        "{moved:?}"
    );
    assert!(
        !moved.iter().any(|m| m.contains("reserve_refuses")),
        "{moved:?}"
    );
    assert_eq!(out_of_frame_of(after_b, "Reword C1"), json!([]));
    assert_eq!(out_of_frame_of(after_b, "Reword C2"), json!([]));
}

#[test]
fn a_reference_names_a_target_of_each_kind_and_cr_open_echoes_it_by_name() {
    let dir = fresh_dir("every-kind");
    let ops = [
        "{\"op\":\"init\"}\n",
        "{\"op\":\"context_add\",\"name\":\"core\"}\n",
        "{\"op\":\"contract_put\",\"context\":\"core\",\"name\":\"rules\",\"title\":\"Rules\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a value is returned\"}]}\n",
        "{\"op\":\"interface_put\",\"context\":\"core\",\"name\":\"api\",\"source\":\"fn value();\\n\",\"items\":[{\"key\":\"value\",\"lines\":[1,1]}]}\n",
        "{\"op\":\"harness_put\",\"context\":\"core\",\"name\":\"tests\",\"source\":\"fn checks_value() {}\\n\",\"cases\":[{\"name\":\"checks_value\",\"lines\":[1,1],\"items\":[{\"interface\":\"api\",\"key\":\"value\"}],\"clauses\":[{\"contract\":\"rules\",\"label\":\"C1\"}]}]}\n",
        // 6: one of each kind, in the two lists (the Context is kept; a Case is a target)
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Everything\",\"intent\":\"change_behavior\",",
            "\"targets\":[{\"project\":true},{\"contract\":{\"context\":\"core\",\"contract\":\"rules\"}},",
            "{\"interface\":{\"context\":\"core\",\"interface\":\"api\"}},{\"case\":{\"context\":\"core\",\"harness\":\"tests\",\"name\":\"checks_value\"}},",
            "{\"item\":{\"context\":\"core\",\"interface\":\"api\",\"key\":\"value\"}}],",
            "\"keep\":[{\"context\":\"core\"},{\"harness\":{\"context\":\"core\",\"harness\":\"tests\"}},",
            "{\"clause\":{\"context\":\"core\",\"contract\":\"rules\",\"label\":\"C1\"}}]}\n"
        ),
        // 7: a reference of no kind, one that is not in the Project, and `false` for the Project
        "{\"op\":\"cr_open\",\"title\":\"Nothing\",\"intent\":\"change_behavior\",\"targets\":[{\"nothing\":true}]}\n",
        "{\"op\":\"cr_open\",\"title\":\"Missing\",\"intent\":\"change_behavior\",\"targets\":[{\"harness\":{\"context\":\"core\",\"harness\":\"gone\"}}]}\n",
        "{\"op\":\"cr_open\",\"title\":\"Twice\",\"intent\":\"change_behavior\",\"targets\":[{\"context\":\"core\"}],\"keep\":[{\"context\":\"core\"}]}\n",
        "{\"op\":\"cr_open\",\"title\":\"No project\",\"intent\":\"change_behavior\",\"targets\":[{\"project\":false}]}\n",
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [7, 8, 9, 10], "{results:#?}");

    let core = |extra: Value| {
        let mut base = serde_json::json!({ "context": "core" });
        for (k, v) in extra.as_object().expect("an object") {
            base[k] = v.clone();
        }
        base
    };
    // in the order of the targets: the Project, the Context, assets, then what they hold
    assert_eq!(
        at(&results, 6)["expects"],
        serde_json::json!([
            { "kind": "target", "target": { "project": true }, "recorded": 2 },
            { "kind": "keep", "target": { "context": "core" }, "recorded": 4 },
            { "kind": "target", "target": { "contract": core(serde_json::json!({ "contract": "rules" })) }, "recorded": 1 },
            { "kind": "target", "target": { "interface": core(serde_json::json!({ "interface": "api" })) }, "recorded": 1 },
            { "kind": "keep", "target": { "harness": core(serde_json::json!({ "harness": "tests" })) }, "recorded": 1 },
            { "kind": "keep", "target": { "clause": core(serde_json::json!({ "contract": "rules", "label": "C1" })) }, "recorded": 1 },
            { "kind": "target", "target": { "item": core(serde_json::json!({ "interface": "api", "key": "value" })) }, "recorded": 1 },
            { "kind": "target", "target": { "case": core(serde_json::json!({ "harness": "tests", "name": "checks_value" })) }, "recorded": 1 }
        ])
    );
    assert!(
        error_at(&results, 8).contains("gone"),
        "{}",
        error_at(&results, 8)
    );
    assert_eq!(
        at(&results, 9)["diagnostics"][0]["code"],
        "cr.target-twice",
        "{}",
        error_at(&results, 9)
    );
    assert!(
        error_at(&results, 10).contains("project"),
        "{}",
        error_at(&results, 10)
    );
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

    let (results, code) = exec(&dir, &first, &store);
    assert_eq!(code, Some(0), "{results:#?}");

    // the project is there: `init` is refused, and the Context the first run added takes a Contract
    let (results, code) = exec(&dir, &second, &store);
    assert_eq!(code, Some(1), "{results:#?}");
    assert_eq!(at(&results, 1)["ok"], false);
    assert!(message(at(&results, 1)).contains("already"));
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
        "moments=['check','edit']\nreads=['contracts']\ngate=false\n",
        "params = { strict = true }\n[checks.\"ext.contract\".params]\nstrict = false\n"
    );
    // The project is made before the program is registered: adding the Context is an edit, which the program
    // registered at `put` would be run for.
    std::fs::write(dir.join("chidev.toml"), "").unwrap();
    let (_, code, err) = exec_in(
        &dir,
        "{\"op\":\"init\"}\n{\"op\":\"context_add\",\"name\":\"core\"}\n",
    );
    assert_eq!(code, Some(0), "{err}");
    std::fs::write(dir.join("chidev.toml"), registration).unwrap();
    let (results, code, err) = exec_in(
        &dir,
        concat!(
            "{\"op\":\"status\"}\n{\"op\":\"check\"}\n{\"op\":\"check_list\"}\n",
            "{\"op\":\"check_run\"}\n{\"op\":\"status\"}\n"
        ),
    );
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert_eq!(results[0]["external"][0]["state"], "unrun");
    assert!(
        results[2]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == "ext.contract" && r["executor"] == "external")
    );
    assert_eq!(results[3]["external"][0]["findings"][0]["level"], "warning");
    assert_eq!(results[4]["external"][0]["state"], "current");
    assert_eq!(std::fs::read_to_string(dir.join("runs")).unwrap(), "run\n");
    let input: Value =
        serde_json::from_slice(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    // The request is version 2 and names the moment; the program answers in version 1, which is accepted.
    assert_eq!(input["protocol"], 2);
    assert_eq!(input["moment"], "check");
    assert!(input.get("hook").is_none());
    assert_eq!(input["params"]["strict"], false);
    assert!(input["model"]["contexts"][0].get("interfaces").is_none());
    let (results, code, err) = exec_in(&dir, "{\"op\":\"status\"}\n");
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(results[0]["external"][0]["state"], "current");
    assert_eq!(std::fs::read_to_string(dir.join("runs")).unwrap(), "run\n");

    std::fs::write(
        dir.join("chidev.toml"),
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
        dir.join("chidev.toml"),
        "[external.\"ext.bad\"]\ncommand='anything'\nmoments=['check']\nreads=['unknown']\n",
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
        dir.join("chidev.toml"),
        "[checks.\"context.\"]\nlevel = \"off\"\n\n[checks.\"pin.unpinned\"]\nlevel = \"error\"\n",
    )
    .expect("a file");
    std::fs::write(
        dir.join(".chidev/chidev.local.toml"),
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
    assert_eq!(listed["checks"].as_array().expect("checks").len(), 32);
    assert_eq!(entry("cr.resolved-without-work")["level"], "warning");
    assert_eq!(entry("cr.targets-untouched")["level"], "note");
    assert_eq!(
        entry("cr.resolved-without-work")["moments"],
        serde_json::json!(["resolve", "check"])
    );
    assert!(
        listed["checks"]
            .as_array()
            .expect("checks")
            .iter()
            .all(|c| c["id"] != "cr.unapplied")
    );
    assert_eq!(entry("crux.policy-line-gone")["level"], "note");
    assert_eq!(
        entry("crux.policy-line-gone")["moments"],
        serde_json::json!(["edit", "check"])
    );
    assert_eq!(entry("context.no-harness")["level"], "off");
    assert_eq!(entry("context.no-harness")["default_level"], "note");
    // the person's layer comes after the project's
    assert_eq!(entry("pin.unpinned")["level"], "warning");
    assert_eq!(entry("cite.stale")["level"], "warning");
    assert_eq!(entry("cite.stale")["mandatory"], false);
    assert_eq!(entry("cite.stale")["executor"], "builtin");
    // what a check speaks about is listed beside where it runs
    assert_eq!(entry("cite.stale")["kind"], "change");
    assert_eq!(entry("cite.uncited-clause")["kind"], "state");
    assert!(listed["policy"].as_str().expect("policy").len() == 64);
}

#[test]
fn a_policy_that_names_no_check_stops_the_run_before_any_operation() {
    let dir = fresh_dir("bad-policy");
    std::fs::write(
        dir.join("chidev.toml"),
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
moments = ["edit", "check"]
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
            "{{\"op\":\"check_rules\",\"moment\":\"pin\"}}\n",
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
    assert_eq!(rule["moments"], serde_json::json!(["edit", "check"]));
    let model = &rules["model"];
    assert_eq!(model["version"], 5);
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
    assert!(message(at(&results, 13)).contains("not judged"));
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
        dir.join("chidev.toml"),
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
        json!({"op":"interface_put","context":"core","name":"api","source":"fn value();\n","items":[{"key":"value","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"tests","source":"fn checks_value() {}\n","cases":[{"name":"checks_value","lines":[1,1],"items":[{"interface":"api","key":"value"}],"clauses":[{"contract":"rules","label":"C1"}]}]}),
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
    let item = json!({"context":"core","interface":"api","key":"value"});
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
        r#"{"rule":"future.rule","subject":{"item":{"context":"core","interface":"api","key":"value","extra":null}}}"#,
        r#"{"rule":"future.rule","subject":{"clause":{"context":"core","contract":"rules","label":"C1","extra":null}}}"#,
        r#"{"rule":"future.rule","subject":null,"item":null}"#,
        r#"{"rule":"future.rule","subject":null,"clause":null}"#,
        r#"{"rule":"future.rule","item":null,"clause":null}"#,
        r#"{"rule":"future.rule","subject":{"context":"core"},"item":null}"#,
        r#"{"rule":"future.rule","subject":null,"item":{"context":"core","interface":"api","key":"value"}}"#,
        r#"{"rule":"future.rule","subject":null,"subject":null}"#,
        r#"{"rule":"future.rule","item":null,"item":null}"#,
        r#"{"rule":"future.rule","clause":null,"clause":null}"#,
        r#"{"rule":"future.rule","rule":"future.other"}"#,
        r#"{"rule":"future.rule","item":{"context":"core","interface":"api","key":"value","extra":null}}"#,
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
        let code = diag(result)["code"].as_str().unwrap();
        assert!(code.starts_with("input."), "{result}");
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
        json!({"item":{"context":"core","interface":"api","key":"missing"}}),
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
            .replace("moments = [\"edit\", \"check\"]", "moments = [ \"check\" ]")
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
        dir.join("chidev.toml"),
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

#[test]
fn a_retired_asset_leaves_the_model_but_its_pin_its_request_and_its_name_stay() {
    let dir = fresh_dir("retire");
    let ops = [
        "{\"op\":\"init\"}\n",
        "{\"op\":\"context_add\",\"name\":\"core\"}\n",
        "{\"op\":\"context_add\",\"name\":\"scratch\"}\n",
        "{\"op\":\"contract_put\",\"context\":\"core\",\"name\":\"rules\",\"title\":\"Rules\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a value is returned\"}]}\n",
        "{\"op\":\"interface_put\",\"context\":\"core\",\"name\":\"api\",\"source\":\"fn value();\\n\",\"items\":[{\"key\":\"value\",\"lines\":[1,1]}]}\n",
        "{\"op\":\"harness_put\",\"context\":\"core\",\"name\":\"tests\",\"source\":\"fn checks_value() {}\\n\",\"cases\":[{\"name\":\"checks_value\",\"lines\":[1,1],\"items\":[{\"interface\":\"api\",\"key\":\"value\"}],\"clauses\":[{\"contract\":\"rules\",\"label\":\"C1\"}]}]}\n",
        "{\"op\":\"pin\",\"context\":\"core\",\"interface\":\"api\",\"code\":\"fn value() {}\"}\n",
        // 8: the request asks for the Interface to go
        "{\"op\":\"cr_open\",\"title\":\"Drop api\",\"intent\":\"change_behavior\",\"targets\":[{\"interface\":{\"context\":\"core\",\"interface\":\"api\"}}]}\n",
        // 9: a Case that stays cites the Clause: refused, naming the Case and the Clause
        "{\"op\":\"contract_retire\",\"context\":\"core\",\"contract\":\"rules\"}\n",
        // 10: the Interface is not gone yet: its request reads it as not touched
        "{\"op\":\"status\"}\n",
        // 11, 12: the Harness that cites goes first, then the Interface it cited
        "{\"op\":\"change_set\",\"for\":\"Drop api\",\"retires\":[{\"retire\":\"harness\",\"context\":\"core\",\"harness\":\"tests\"}]}\n",
        "{\"op\":\"change_set\",\"for\":\"Drop api\",\"retires\":[{\"retire\":\"interface\",\"context\":\"core\",\"interface\":\"api\"}]}\n",
        // 13: the retirement was made for the request: its target reads retired, and it is resolved as done
        "{\"op\":\"cr_resolve\",\"title\":\"Drop api\",\"resolution\":\"done\",\"reason\":\"the interface is retired\",\"by\":{\"person\":\"alice\"}}\n",
        // 14, 15: nothing cites the Contract any more; nothing uses `scratch`
        "{\"op\":\"contract_retire\",\"context\":\"core\",\"contract\":\"rules\"}\n",
        "{\"op\":\"context_retire\",\"context\":\"scratch\"}\n",
        // 16: the name and the id are reserved
        "{\"op\":\"interface_put\",\"context\":\"core\",\"name\":\"api\",\"source\":\"fn value();\\n\",\"items\":[]}\n",
        "{\"op\":\"status\"}\n",
        "{\"op\":\"check\"}\n",
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [9, 16], "{results:#?}");

    let refused = error_at(&results, 9);
    assert!(
        refused.contains("tests checks_value") && refused.contains("rules C1"),
        "{refused}"
    );
    assert_eq!(
        at(&results, 10)["change_requests"][0]["targets"],
        serde_json::json!([{ "target": "interface api", "progress": "not touched yet" }])
    );
    let reserved = error_at(&results, 16);
    assert!(
        reserved.contains("retired") && reserved.contains("api"),
        "{reserved}"
    );

    // what went with the Harness and with the Interface, the thing first, and the revision of what held it
    let line = |kind: &str, name: &str, last: u64, at: u64| serde_json::json!({ "kind": kind, "context": "core", "name": name, "last_revision": last, "at": at });
    assert_eq!(
        at(&results, 11)["retired"],
        serde_json::json!([
            line("harness", "tests", 1, 5),
            line("case", "tests checks_value", 1, 5)
        ])
    );
    assert_eq!(
        at(&results, 12)["retired"],
        serde_json::json!([
            line("interface", "api", 1, 6),
            line("item", "api value", 1, 6)
        ])
    );
    assert_eq!(at(&results, 13)["ok"], true);
    assert_eq!(
        at(&results, 14)["retired"],
        serde_json::json!([
            line("contract", "rules", 1, 7),
            line("clause", "rules C1", 1, 7)
        ])
    );
    // the Project goes one up when a Context is retired
    assert_eq!(
        at(&results, 15)["retired"],
        serde_json::json!([{ "kind": "context", "context": "scratch", "name": "scratch", "last_revision": 1, "at": 4 }])
    );

    let status = at(&results, 17);
    // the Pin stays, reads gone, and the Interface is named as retired
    assert_eq!(
        status["pins"],
        serde_json::json!([{ "context": "core", "interface": "api (retired)", "state": "gone", "why": [] }])
    );
    assert_eq!(status["change_requests"][0]["state"], "resolved");
    assert_eq!(
        status["change_requests"][0]["targets"],
        serde_json::json!([{ "target": "interface api (retired)", "progress": "retired" }])
    );
    let kinds: Vec<&str> = status["retired"]
        .as_array()
        .expect("retired")
        .iter()
        .map(|r| r["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(
        kinds,
        [
            "harness",
            "case",
            "interface",
            "item",
            "contract",
            "clause",
            "context"
        ]
    );
    // retiring is intentional and nothing can fix the Pin, so neither status nor check finds it stale
    for view in [status, at(&results, 18)] {
        let stale = view["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .filter(|f| f["rule"] == "pin.stale")
            .count();
        assert_eq!(stale, 0, "{view:#?}");
    }
}

/// Where the store of `dir` stands: the revision it moves one up at each commit.
fn store_revision(dir: &Path) -> u64 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    rt.block_on(async {
        let store = chidev_core::infra::sqlite::SqliteStore::open(dir.join("chidev.db"))
            .await
            .expect("a store");
        chidev_core::app::ports::Store::revision(&store)
            .await
            .expect("a revision")
            .0
    })
}

fn case_json(name: &str, line: usize, key: &str, label: &str) -> Value {
    serde_json::json!({
        "name": name,
        "lines": [line, line],
        "items": [{ "interface": "stock", "key": key }],
        "clauses": [{ "contract": "stock", "label": label }]
    })
}

fn stock_clauses(count: usize) -> Value {
    (1..=count)
        .map(|k| serde_json::json!({ "label": format!("C{k}"), "text": format!("C{k} holds") }))
        .collect()
}

#[test]
fn a_change_set_removes_cited_clauses_and_an_item_with_their_harness_in_one_commit() {
    use serde_json::json;
    let dir = fresh_dir("change-set");
    let tests_source = "fn reserve_refuses() {}\nfn release_frees() {}\nfn release_twice() {}\n";
    let (setup, code, err) = run_values(
        &dir,
        &[
            json!({ "op": "init" }),
            json!({ "op": "context_add", "name": "core" }),
            json!({ "op": "contract_put", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(8) }),
            json!({ "op": "interface_put", "context": "core", "name": "stock", "source": "fn reserve();\nfn release();\n",
                    "items": [{ "key": "reserve", "lines": [1, 1] }, { "key": "release", "lines": [2, 2] }] }),
            json!({ "op": "harness_put", "context": "core", "name": "stock-tests", "source": tests_source,
                    "cases": [
                        case_json("reserve_refuses", 1, "reserve", "C1"),
                        case_json("release_frees", 2, "release", "C7"),
                        case_json("release_twice", 3, "release", "C8"),
                    ] }),
        ],
    );
    assert_eq!(code, Some(0), "{err}\n{setup:#?}");
    let before = store_revision(&dir);

    let contract = json!({ "put": "contract", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(6) });
    let interface = json!({ "put": "interface", "context": "core", "name": "stock", "source": "fn reserve();\n",
                            "items": [{ "key": "reserve", "lines": [1, 1] }] });
    let harness = json!({ "put": "harness", "context": "core", "name": "stock-tests", "source": "fn reserve_refuses() {}\n",
                          "cases": [case_json("reserve_refuses", 1, "reserve", "C1")] });
    let (results, code, err) = run_values(
        &dir,
        &[
            // 1: a single put cannot drop C7 while a Case cites it
            json!({ "op": "contract_put", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(6) }),
            // 2: nor can a change set that leaves the Harness as it is, and it names every problem
            json!({ "op": "change_set", "puts": [contract.clone(), interface.clone()] }),
            // 3: the whole change
            json!({ "op": "change_set", "puts": [harness, contract, interface] }),
            json!({ "op": "status" }),
        ],
    );
    assert_eq!(code, Some(1), "{err}\n{results:#?}");

    let single = error_at(&results, 1);
    assert!(
        single.contains("stock-tests release_frees")
            && single.contains("stock C7")
            && single.contains("cannot reach"),
        "{single}"
    );
    let refused = error_at(&results, 2);
    for named in [
        "case stock-tests release_frees",
        "case stock-tests release_twice",
        "the clause `stock C7`",
        "the clause `stock C8`",
        "the item `stock release`",
    ] {
        assert!(refused.contains(named), "{named} not in {refused}");
    }

    let done = at(&results, 3);
    assert_eq!(done["ok"], true, "{done:#?}");
    assert_eq!(done["op"], "change_set");
    let puts: Vec<(&str, u64)> = done["puts"]
        .as_array()
        .expect("puts")
        .iter()
        .map(|p| {
            (
                p["put"].as_str().expect("kind"),
                p["revision"].as_u64().expect("revision"),
            )
        })
        .collect();
    assert_eq!(puts, [("harness", 2), ("contract", 2), ("interface", 2)]);
    let gone: Vec<&str> = done["retired"]
        .as_array()
        .expect("retired")
        .iter()
        .map(|r| r["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        gone,
        [
            "stock C7",
            "stock C8",
            "stock release",
            "stock-tests release_frees",
            "stock-tests release_twice"
        ]
    );
    // one commit for the whole change; the refusals kept nothing
    assert_eq!(store_revision(&dir), before + 1);
    assert_eq!(
        at(&results, 4)["retired"]
            .as_array()
            .expect("retired")
            .len(),
        5
    );
}

#[test]
fn a_change_set_makes_what_its_harness_cites_and_retires_in_the_same_change() {
    use serde_json::json;
    let dir = fresh_dir("change-set-new");
    let (results, code, err) = run_values(
        &dir,
        &[
            json!({ "op": "init" }),
            json!({ "op": "context_add", "name": "core" }),
            json!({ "op": "context_add", "name": "scratch" }),
            // 3: a Contract, an Interface and the Harness that cites both, none of which exists yet
            json!({ "op": "change_set", "puts": [
                { "put": "harness", "context": "core", "name": "stock-tests", "source": "fn reserve_refuses() {}\n",
                  "cases": [case_json("reserve_refuses", 1, "reserve", "C1")] },
                { "put": "contract", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(1) },
                { "put": "interface", "context": "core", "name": "stock", "source": "fn reserve();\n",
                  "items": [{ "key": "reserve", "lines": [1, 1] }] },
                { "put": "base_on_contract", "context": "core" }
            ], "retires": [{ "retire": "context", "context": "scratch" }] }),
            // 4: a thing named twice is refused
            json!({ "op": "change_set", "puts": [
                { "put": "contract", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(2) },
                { "put": "contract", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(3) }
            ] }),
            // 5: an empty change set is not an operation
            json!({ "op": "change_set" }),
            json!({ "op": "check" }),
        ],
    );
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let done = at(&results, 4);
    assert_eq!(done["ok"], true, "{done:#?}");
    let kinds: Vec<&str> = done["puts"]
        .as_array()
        .expect("puts")
        .iter()
        .map(|p| p["put"].as_str().expect("kind"))
        .collect();
    assert_eq!(
        kinds,
        ["harness", "contract", "interface", "base_on_contract"]
    );
    assert_eq!(done["retired"][0]["kind"], "context");
    assert!(error_at(&results, 5).contains("names the contract `stock` twice"));
    assert!(error_at(&results, 6).contains("nothing to put or retire"));
    assert_eq!(at(&results, 7)["ok"], true);
}

/// A change set adds a Context with its first assets, so a request that names the Project as a target reads it
/// revised by the set made `for` it, and not by a Context that a set for nothing added.
#[test]
fn a_change_set_for_a_request_that_adds_a_context_meets_the_change_of_the_project() {
    use serde_json::json;
    let dir = fresh_dir("context-put");
    let context = |name: &str, uses: Vec<&str>| json!({"put":"context","name":name,"uses":uses});
    let (results, code, err) = run_values(
        &dir,
        &[
            json!({"op":"init"}),
            json!({"op":"cr_open","title":"Add billing","intent":"change_behavior",
                "targets":[{"project":true}]}),
            // 3: a Context added by a set for nothing moves the Project, for no request
            json!({"op":"change_set","puts":[context("audit", vec![])]}),
            // 4: so the request has no work yet, and its target reads not touched
            json!({"op":"status"}),
            // 5: the set for the request adds a Context, another that it uses (listed after it), and puts into both
            json!({"op":"change_set","for":"Add billing","puts":[
                context("billing", vec!["ledger"]),
                context("ledger", vec![]),
                {"put":"contract","context":"billing","name":"invoice","title":"Invoice",
                 "clauses":[{"label":"C1","text":"an invoice has a total"}]},
                {"put":"interface","context":"ledger","name":"entries","source":"fn post()\n",
                 "items":[{"key":"post","lines":[1,1]}]},
                {"put":"harness","context":"billing","name":"invoice-tests",
                 "source":"fn totals() {}\n",
                 "cases":[{"name":"totals","lines":[1,1],
                    "items":[{"context":"ledger","interface":"entries","key":"post"}],
                    "clauses":[{"contract":"invoice","label":"C1"}]}]}
            ]}),
            // 6: now it reads revised, and it is resolved as done
            json!({"op":"cr_resolve","title":"Add billing","resolution":"done",
                "reason":"billing is added","by":{"person":"alice"}}),
            // 7: a Context that exists is not added again
            json!({"op":"change_set","puts":[context("billing", vec![])]}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [7], "{results:#?}");
    assert_eq!(
        at(&results, 4)["change_requests"][0]["targets"],
        json!([{ "target": "project", "progress": "not touched yet" }])
    );
    assert_eq!(
        at(&results, 8)["change_requests"][0]["targets"],
        json!([{ "target": "project", "progress": "revised" }])
    );
    let kinds: Vec<&str> = at(&results, 5)["puts"]
        .as_array()
        .expect("puts")
        .iter()
        .map(|p| p["put"].as_str().expect("kind"))
        .collect();
    assert_eq!(
        kinds,
        ["context", "context", "contract", "interface", "harness"]
    );
    assert_eq!(at(&results, 6)["ok"], true, "{:#?}", at(&results, 6));
}

/// The Contract, Interface and Harness of the `source_file` tests, with the files they read written under `dir`.
fn file_project(dir: &Path) {
    for (path, text) in [
        ("src/api.rs", "fn value();\n"),
        ("tests/api_tests.rs", "fn checks_value() {}\n"),
        ("code/api_impl.rs", "fn value() {}\n"),
    ] {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("a directory");
        std::fs::write(path, text).expect("a file");
    }
}

fn file_setup() -> Vec<Value> {
    use serde_json::json;
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"contract_put","context":"core","name":"rules","title":"Rules","clauses":[{"label":"C1","text":"a value is returned"}]}),
    ]
}

fn api_from_file(file: &str) -> Value {
    serde_json::json!({"op":"interface_put","context":"core","name":"api","source_file":file,"items":[{"key":"value","lines":[1,1]}]})
}

fn tests_from_file(file: &str) -> Value {
    serde_json::json!({"op":"harness_put","context":"core","name":"tests","source_file":file,"cases":[{"name":"checks_value","lines":[1,1],"items":[{"interface":"api","key":"value"}],"clauses":[{"contract":"rules","label":"C1"}]}]})
}

#[test]
fn an_interface_a_harness_and_a_pin_take_their_text_from_files_and_the_model_keeps_the_text() {
    use serde_json::json;
    let dir = fresh_dir("from-files");
    file_project(&dir);
    let mut ops = file_setup();
    ops.push(api_from_file("src/api.rs"));
    // a `./` path is a path under the root too
    ops.push(tests_from_file("./tests/api_tests.rs"));
    ops.push(json!({"op":"pin","context":"core","interface":"api","code_file":"code/api_impl.rs"}));
    ops.push(json!({"op":"status"}));
    let (results, code, err) = run_values(&dir, &ops);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert!(results.iter().all(|r| r["ok"] == true), "{results:#?}");
    assert_eq!(at(&results, 4)["revision"], 1);
    assert_eq!(at(&results, 5)["revision"], 1);
    assert_eq!(pin_states(at(&results, 7)), states(&[("api", "fresh")]));

    // the files change, and nothing is put again: the model holds the text it was given, not the path
    std::fs::write(dir.join("src/api.rs"), "fn value();\nfn other();\n").expect("a file");
    std::fs::write(dir.join("code/api_impl.rs"), "fn value() { 1 }\n").expect("a file");
    let (after, code, err) = run_values(&dir, &[json!({"op":"status"})]);
    assert_eq!(code, Some(0), "{err}\n{after:#?}");
    assert_eq!(pin_states(&after[0]), states(&[("api", "fresh")]));

    // put again from the file: now the model moves, and the pin falls behind
    let (again, code, err) =
        run_values(&dir, &[api_from_file("src/api.rs"), json!({"op":"status"})]);
    assert_eq!(code, Some(0), "{err}\n{again:#?}");
    assert_eq!(again[0]["revision"], 2);
    assert_ne!(pin_states(&again[1]), states(&[("api", "fresh")]));
}

#[test]
fn a_change_set_takes_the_text_of_its_interface_and_harness_entries_from_files() {
    use serde_json::json;
    let dir = fresh_dir("change-set-files");
    file_project(&dir);
    let (results, code, err) = run_values(
        &dir,
        &[
            json!({"op":"init"}),
            json!({"op":"change_set","puts":[
                {"put":"context","name":"core"},
                {"put":"contract","context":"core","name":"rules","title":"Rules","clauses":[{"label":"C1","text":"a value is returned"}]},
                {"put":"interface","context":"core","name":"api","source_file":"src/api.rs","items":[{"key":"value","lines":[1,1]}]},
                {"put":"harness","context":"core","name":"tests","source_file":"tests/api_tests.rs","cases":[{"name":"checks_value","lines":[1,1],"items":[{"interface":"api","key":"value"}],"clauses":[{"contract":"rules","label":"C1"}]}]}]}),
            json!({"op":"pin","context":"core","interface":"api","code_file":"code/api_impl.rs"}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert!(results.iter().all(|r| r["ok"] == true), "{results:#?}");
    let kinds: Vec<&str> = at(&results, 2)["puts"]
        .as_array()
        .expect("puts")
        .iter()
        .map(|p| p["put"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, ["context", "contract", "interface", "harness"]);
    assert_eq!(pin_states(at(&results, 4)), states(&[("api", "fresh")]));
}

/// The error of a refused result, which is not ok.
fn refusal(result: &Value) -> String {
    message(result)
}

#[test]
fn a_file_that_is_not_a_path_under_the_root_or_not_text_or_given_twice_is_refused_by_name() {
    use serde_json::json;
    let dir = fresh_dir("file-refusals");
    file_project(&dir);
    std::fs::write(dir.join("src/binary.rs"), [0xff_u8, 0xfe, 0x00]).expect("a file");
    let mut ops = file_setup();
    ops.extend([
        // 4: an absolute path, a path that goes up, a path that goes up after going down
        api_from_file("/etc/hosts"),
        api_from_file("../outside.rs"),
        api_from_file("src/../../outside.rs"),
        // 7: a file that is not there, a file that is not UTF-8, a directory
        api_from_file("src/missing.rs"),
        api_from_file("src/binary.rs"),
        api_from_file("src"),
        // 10: both given, neither given
        json!({"op":"interface_put","context":"core","name":"api","source":"fn value();\n","source_file":"src/api.rs","items":[]}),
        json!({"op":"interface_put","context":"core","name":"api","items":[]}),
        // 12: the same for a harness and for a pin
        tests_from_file("/etc/hosts"),
        json!({"op":"harness_put","context":"core","name":"tests","source":"x","source_file":"src/api.rs","cases":[]}),
        json!({"op":"pin","context":"core","interface":"api","code_file":"../outside.rs"}),
        json!({"op":"pin","context":"core","interface":"api","code":"x","code_file":"code/api_impl.rs"}),
        json!({"op":"pin","context":"core","interface":"api"}),
        // 17: nothing was put by any of them
        json!({"op":"status"}),
    ]);
    let (results, code, err) = run_values(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");

    let error = |n: u64| refusal(at(&results, n));
    assert!(
        error(4).contains("absolute") && error(4).contains("/etc/hosts"),
        "{}",
        error(4)
    );
    assert!(
        error(5).contains("..") && error(5).contains("../outside.rs"),
        "{}",
        error(5)
    );
    assert!(error(6).contains("..") && error(6).contains("src/../../outside.rs"));
    assert!(
        error(7).contains("cannot read") && error(7).contains("src/missing.rs"),
        "{}",
        error(7)
    );
    assert!(
        error(8).contains("not UTF-8") && error(8).contains("src/binary.rs"),
        "{}",
        error(8)
    );
    assert!(error(9).contains("cannot read") && error(9).contains("`src`"));
    assert!(
        error(10).contains("`source` or `source_file`, not both"),
        "{}",
        error(10)
    );
    assert!(
        error(11).contains("give `source` or `source_file`") && !error(11).contains("not both"),
        "{}",
        error(11)
    );
    assert!(error(12).contains("absolute"), "{}", error(12));
    assert!(error(13).contains("not both"), "{}", error(13));
    assert!(error(14).contains("..") && error(14).contains("code_file"));
    assert!(error(15).contains("`code` or `code_file`, not both"));
    assert!(error(16).contains("give `code` or `code_file`"));
    // the refused puts left no Interface and no Harness behind
    let status = at(&results, 17);
    assert_eq!(status["ok"], true, "{status:#?}");
    let (put, _, _) = run_values(&dir, &[api_from_file("src/api.rs")]);
    assert_eq!(put[0]["revision"], 1, "{put:#?}");
}

#[test]
fn a_change_set_with_a_bad_file_is_refused_whole_and_names_every_bad_entry() {
    use serde_json::json;
    let dir = fresh_dir("change-set-file-refusals");
    file_project(&dir);
    let mut ops = file_setup();
    ops.extend([
        json!({"op":"change_set","puts":[
            {"put":"contract","context":"core","name":"extra","title":"Extra","clauses":[{"label":"C1","text":"x"}]},
            {"put":"interface","context":"core","name":"api","source_file":"src/missing.rs","items":[]},
            {"put":"harness","context":"core","name":"tests","source_file":"../outside.rs","cases":[]},
            {"put":"interface","context":"core","name":"both","source":"x","source_file":"src/api.rs","items":[]}]}),
        // the contract of the refused set was not made: this is its first revision
        json!({"op":"contract_put","context":"core","name":"extra","title":"Extra","clauses":[{"label":"C1","text":"x"}]}),
    ]);
    let (results, code, err) = run_values(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let error = refusal(at(&results, 4));
    assert_eq!(
        at(&results, 4)["diagnostics"]
            .as_array()
            .expect("list")
            .len(),
        3,
        "{error}"
    );
    assert!(error.contains("src/missing.rs"), "{error}");
    assert!(error.contains("../outside.rs"), "{error}");
    assert!(error.contains("not both"), "{error}");
    assert_eq!(at(&results, 5)["revision"], 1, "{results:#?}");
}

/// `chidev exec` of the operations `ops` (written to `ops.jsonl` in `dir`) run from `cwd`, with `--store` when `store`
/// is given: the results, the exit code, standard error.
fn exec_from(
    cwd: &Path,
    dir: &Path,
    ops: &[Value],
    store: Option<&str>,
) -> (Vec<Value>, Option<i32>, String) {
    let file = dir.join("ops.jsonl");
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(&file, text).expect("a file");
    let mut command = Command::new(env!("CARGO_BIN_EXE_chidev"));
    command.current_dir(cwd).arg("exec").arg(&file);
    if let Some(store) = store {
        command.arg("--store").arg(store);
    }
    let out = command.output().expect("chidev runs");
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
fn from_a_sub_directory_the_root_is_the_nearest_chidev_toml_and_holds_the_files_and_the_store() {
    let dir = fresh_dir("root-above");
    file_project(&dir);
    let mut ops = file_setup();
    ops.push(api_from_file("src/api.rs"));
    ops.push(serde_json::json!({"op":"status"}));
    // `src` has no chidev.toml of its own: the root is the directory above it
    let (results, code, err) = exec_from(&dir.join("src"), &dir, &ops, None);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert!(results.iter().all(|r| r["ok"] == true), "{results:#?}");
    assert_eq!(at(&results, 4)["revision"], 1);
    assert!(dir.join(".chidev/chidev.db").is_file());
    assert!(!dir.join("src/.chidev").exists());
}

#[test]
fn without_a_chidev_toml_anywhere_up_the_run_stops_before_any_operation() {
    let dir = std::env::temp_dir().join(format!("chidev-no-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    if dir.ancestors().any(|d| d.join("chidev.toml").is_file()) {
        eprintln!("skipped: a chidev.toml above the temporary directory marks a root");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let (results, code, err) = exec_from(&dir, &dir, &file_setup(), None);
    assert_eq!(code, Some(2), "{err}\n{results:#?}");
    assert!(results.is_empty(), "{results:#?}");
    let here = dir.canonicalize().expect("a real path");
    assert!(
        err.contains(&format!(
            "no chidev.toml in {} or any parent directory",
            here.display()
        )) && err.contains("marks the project's root"),
        "{err}"
    );
    assert!(!dir.join(".chidev").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_store_flag_wins_over_the_store_of_the_settings_and_is_relative_to_where_it_runs() {
    let dir = fresh_dir("store-flag");
    file_project(&dir);
    std::fs::write(
        dir.join("chidev.toml"),
        "[store]\npath = \"var/from-settings.db\"\n",
    )
    .expect("a file");
    let ops = [serde_json::json!({"op":"init"})];
    // without the flag, the settings say, from the root whatever the directory it runs in
    let (results, code, err) = exec_from(&dir.join("src"), &dir, &ops, None);
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert!(dir.join("var/from-settings.db").is_file());
    // with it, the flag does, from the directory it runs in
    let (results, code, err) = exec_from(&dir.join("src"), &dir, &ops, Some("from-flag.db"));
    assert_eq!(code, Some(0), "{err}\n{results:#?}");
    assert!(dir.join("src/from-flag.db").is_file());
    assert!(!dir.join("from-flag.db").exists());
}

/// The operations that used to keep what they changed without a word from the checks say what the checks said.
#[test]
fn the_operations_that_were_kept_without_a_check_answer_with_findings() {
    let dir = fresh_dir("kept-with-findings");
    let ops = [
        "{\"op\":\"init\"}",
        "{\"op\":\"context_add\",\"name\":\"base\"}",
        "{\"op\":\"context_add\",\"name\":\"core\"}",
        "{\"op\":\"context_uses\",\"context\":\"core\",\"uses\":[\"base\"]}",
        "{\"op\":\"contract_put\",\"context\":\"core\",\"name\":\"stack\",\"title\":\"Stack\",\"clauses\":[{\"label\":\"C1\",\"text\":\"push then pop\"}]}",
        "{\"op\":\"base_on_contract\",\"context\":\"core\"}",
        "{\"op\":\"cr_open\",\"title\":\"q\",\"intent\":\"keep_behavior\"}",
        "{\"op\":\"cr_crux_raise\",\"in\":\"q\",\"question\":\"Is it kept?\",\"kind\":\"ambiguity\",\"options\":[{\"choice\":\"keep\"}]}",
        "{\"op\":\"cr_assume\",\"title\":\"q\",\"statement\":\"a stack is a stack\",\"reason\":\"it is how it is used\"}",
        "{\"op\":\"cr_crux_decide\",\"crux\":\"@8.crux\",\"option\":\"@8.option(keep)\",\"basis\":{\"arbitrary\":\"nothing derives it\"},\"by\":{\"person\":\"alice\"}}",
        "{\"op\":\"cr_resolve\",\"title\":\"q\",\"resolution\":\"done\",\"reason\":\"settled\",\"by\":{\"person\":\"alice\"}}",
        "{\"op\":\"cr_reopen\",\"title\":\"q\",\"reason\":\"it came back\",\"by\":{\"person\":\"alice\"}}",
    ]
    .map(str::to_string);
    let (results, code) = exec_each(&dir, &dir.join("chidev.db"), &ops);
    assert_eq!(code, Some(0), "{results:#?}");
    for (n, op) in ops.iter().enumerate().skip(1) {
        let result = &results[n];
        assert_eq!(result["ok"], true, "{op}: {result:#?}");
        assert!(
            result["findings"].is_array(),
            "{op} has no findings: {result:#?}"
        );
    }
    // `init` makes the Project and runs no check, so it has nothing to say.
    assert!(results[0].get("findings").is_none());
}

/// A registered gate sees the edits that were not asked about, and `init`, which is a record, is not asked.
#[test]
#[cfg(unix)]
fn a_registered_gate_refuses_adding_a_context_and_is_not_asked_about_init() {
    use std::os::unix::fs::PermissionsExt;
    let dir = fresh_dir("gate-context-add");
    let checker = dir.join("checker");
    std::fs::write(
        &checker,
        concat!(
            "#!/bin/sh\ncat > request.json\necho run >> runs\n",
            "printf '%s' '{\"protocol\":2,\"findings\":[{\"rule\":\"ext.contract\",\"message\":\"no new contexts\"}]}'\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&checker, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join("chidev.toml"),
        "[external.\"ext.contract\"]\ncommand='./checker'\nmoments=['edit']\nreads=['contracts']\ngate=true\n",
    )
    .unwrap();
    let (results, code, err) = exec_in(
        &dir,
        "{\"op\":\"init\"}\n{\"op\":\"context_add\",\"name\":\"core\"}\n{\"op\":\"status\"}\n",
    );
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    assert_eq!(results[0]["ok"], true, "{results:#?}");
    assert_eq!(results[1]["ok"], false, "{results:#?}");
    assert_eq!(results[1]["external"][0]["findings"][0]["level"], "error");
    assert!(
        message(&results[1]).contains("no new contexts"),
        "{results:#?}"
    );
    // the program was asked once, for the edit and not for `init`
    assert_eq!(std::fs::read_to_string(dir.join("runs")).unwrap(), "run\n");
    let asked: Value =
        serde_json::from_slice(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    assert_eq!(
        (asked["protocol"].clone(), asked["moment"].clone()),
        (2.into(), "edit".into())
    );
    // and nothing was kept
    assert_eq!(results[2]["ok"], true);
    assert_eq!(results[2]["external"][0]["state"], "unrun");
}

/// The words the first version used are refused, each with the word it is now.
#[test]
fn an_old_word_for_a_moment_is_refused_with_the_new_one() {
    // the key of a registration, and the word `put` in it
    for (name, registration, expect) in [
        (
            "key",
            "[external.\"ext.old\"]\ncommand='x'\nhooks=['check']\nreads=['contracts']\n",
            "`hooks` is now `moments`",
        ),
        (
            "key-and-word",
            "[external.\"ext.old\"]\ncommand='x'\nhooks=['put']\nreads=['contracts']\n",
            "`hooks` is now `moments`",
        ),
        (
            "word",
            "[external.\"ext.old\"]\ncommand='x'\nmoments=['put']\nreads=['contracts']\n",
            "`put` is now `edit`",
        ),
    ] {
        let dir = fresh_dir(&format!("old-word-external-{name}"));
        std::fs::write(dir.join("chidev.toml"), registration).unwrap();
        let (results, code, err) = exec_in(&dir, "{\"op\":\"init\"}\n");
        assert_eq!(code, Some(2), "{name}");
        assert!(results.is_empty(), "{name}");
        assert!(err.contains(expect), "{name}: {err}");
    }
    // the same in a judged rule
    for (name, text, expect) in [
        (
            "key",
            VAGUE_RULE.replace(
                "moments = [\"edit\", \"check\"]",
                "hooks = [\"edit\", \"check\"]",
            ),
            "`hooks` is now `moments`",
        ),
        (
            "word",
            VAGUE_RULE.replace("\"edit\"", "\"put\""),
            "`put` is now `edit`",
        ),
    ] {
        let dir = fresh_dir(&format!("old-word-judged-{name}"));
        std::fs::create_dir_all(dir.join("checks/judged")).unwrap();
        std::fs::write(dir.join("checks/judged/vague.toml"), text).unwrap();
        let (results, code, err) = exec_in(&dir, "{\"op\":\"init\"}\n");
        assert_eq!(code, Some(2), "{name}");
        assert!(results.is_empty(), "{name}");
        assert!(err.contains(expect), "{name}: {err}");
    }
    // and in the argument of `check_rules`, where an argument left unread would have asked for every rule
    let dir = fresh_dir("old-word-check-rules");
    std::fs::create_dir_all(dir.join("checks/judged")).unwrap();
    std::fs::write(dir.join("checks/judged/vague-clause.toml"), VAGUE_RULE).unwrap();
    let (results, code, err) = exec_in(
        &dir,
        concat!(
            "{\"op\":\"init\"}\n",
            "{\"op\":\"check_rules\",\"hook\":\"pin\"}\n",
            "{\"op\":\"check_rules\",\"moment\":\"put\"}\n",
            "{\"op\":\"check_rules\",\"moment\":\"save\"}\n",
            "{\"op\":\"check_rules\",\"moment\":\"edit\"}\n",
            "{\"op\":\"check_rules\",\"moment\":\"pin\"}\n",
        ),
    );
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    assert!(
        message(&results[1]).contains("`hook` is now `moment`"),
        "{results:#?}"
    );
    assert!(
        message(&results[2]).contains("`put` is now `edit`"),
        "{results:#?}"
    );
    assert!(
        message(&results[3]).contains("`save` is not a moment"),
        "{results:#?}"
    );
    assert_eq!(results[4]["ok"], true, "{results:#?}");
    assert_eq!(results[4]["rules"].as_array().unwrap().len(), 1);
    assert_eq!(results[5]["rules"], serde_json::json!([]));
}

#[test]
fn the_schemas_of_the_operations_and_of_the_view_have_the_beyond_of_a_decision() {
    for name in ["ops", "view"] {
        let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
            .args(["schema", name])
            .output()
            .expect("chidev runs");
        assert!(out.status.success(), "{out:#?}");
        let text = String::from_utf8(out.stdout).expect("utf-8");
        for word in ["\"beyond\"", "\"requests\"", "\"declined\""] {
            assert!(text.contains(word), "{name}: no {word} in {text}");
        }
    }
}

#[test]
fn what_an_answer_adds_outside_its_request_goes_to_a_request_or_is_declined_and_status_and_view_show_it()
 {
    use serde_json::json;
    let dir = fresh_dir("beyond");
    let tax_c1 = json!({"clause": {"context": "pricing", "contract": "tax", "label": "C1"}});
    let raise = |choice: &str| {
        format!(
            "{}\n",
            json!({"op": "cr_crux_raise", "in": "Round", "question": "How is a total made?",
                "kind": "ambiguity", "options": [{"choice": choice, "targets": [tax_c1.clone()]}, {"choice": "skip"}]})
        )
    };
    let decide = |line: u64, choice: &str, beyond: Option<Value>| {
        let mut op = json!({"op": "cr_crux_decide", "crux": format!("@{line}.crux"),
            "option": format!("@{line}.option({choice})"),
            "basis": {"arbitrary": "settled"}, "by": {"person": "alice"}});
        if let Some(beyond) = beyond {
            op["beyond"] = beyond;
        }
        format!("{op}\n")
    };
    let ops = [
        "{\"op\":\"init\"}\n".to_string(),
        "{\"op\":\"context_add\",\"name\":\"pricing\"}\n".to_string(),
        discount_put(
            "a half rounds up",
            "a refused discount is an InvalidDiscount",
        ),
        // 4: the Contract the first request is not about
        concat!(
            "{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"tax\",\"title\":\"Tax\",",
            "\"clauses\":[{\"label\":\"C1\",\"text\":\"a tax is summed\"}]}\n"
        )
        .to_string(),
        // 5, 6: a request about each Contract
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Round\",\"intent\":\"change_behavior\",",
            "\"targets\":[{\"contract\":{\"context\":\"pricing\",\"contract\":\"discount\"}}]}\n"
        )
        .to_string(),
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Tax work\",\"intent\":\"change_behavior\",",
            "\"targets\":[{\"contract\":{\"context\":\"pricing\",\"contract\":\"tax\"}}]}\n"
        )
        .to_string(),
        // 7: a question whose first option asks a Clause of the other Contract, which `Round` does not frame
        raise("sum"),
        // 8: it cannot be chosen without saying where that goes; 9: it goes to the request that frames it
        decide(7, "sum", None),
        decide(7, "sum", Some(json!({"requests": ["Tax work"]}))),
        // 10, 11: the same question, and the Clause declined
        raise("sum"),
        decide(10, "sum", Some(json!({"declined": "tax waits"}))),
        "{\"op\":\"status\"}\n".to_string(),
        "{\"op\":\"view\",\"sections\":[\"change_requests\",\"cruxes\"],\"contexts\":[]}\n"
            .to_string(),
        // 14: a form `beyond` does not have is refused as an operation that cannot be read
        decide(10, "sum", Some(json!({"request": ["Tax work"]}))),
    ];
    let (results, _) = exec_each(&dir, &dir.join("chidev.db"), &ops);

    let outside = diag(at(&results, 8));
    assert_eq!(outside["code"], "cr.crux-answer-outside");
    let suggested: Vec<&str> = outside["suggestions"]
        .as_array()
        .expect("suggestions")
        .iter()
        .map(|s| s["op"]["op"].as_str().expect("an operation"))
        .collect();
    assert_eq!(suggested, ["cr_open", "cr_crux_decide", "cr_crux_decide"]);
    assert_eq!(outside["suggestions"][0]["op"]["targets"], json!([tax_c1]));
    assert_eq!(at(&results, 9)["ok"], true, "{results:#?}");
    assert_eq!(at(&results, 11)["ok"], true, "{results:#?}");

    // status gives the titles of the requests, view their ids, and the reason as it was written
    let first = at(&results, 7)["crux"]
        .as_str()
        .expect("the id of the Crux");
    let second = at(&results, 10)["crux"]
        .as_str()
        .expect("the id of the Crux");
    let answer_of = |list: &Value, id: &str| -> Value {
        list.as_array()
            .expect("cruxes")
            .iter()
            .find(|q| q["id"] == id)
            .unwrap_or_else(|| panic!("no crux {id}"))["answer"]
            .clone()
    };
    let status = at(&results, 12);
    assert_eq!(
        answer_of(&status["cruxes"], first)["beyond"],
        json!({"requests": ["Tax work"]})
    );
    assert_eq!(
        answer_of(&status["cruxes"], second)["beyond"],
        json!({"declined": "tax waits"})
    );
    let view = &at(&results, 13)["view"];
    let tax_work = view["change_requests"]
        .as_array()
        .expect("change requests")
        .iter()
        .find(|c| c["title"] == "Tax work")
        .expect("the request")["id"]
        .clone();
    assert_eq!(
        answer_of(&view["cruxes"], first)["beyond"],
        json!({"requests": [tax_work]})
    );
    assert_eq!(
        answer_of(&view["cruxes"], second)["beyond"],
        json!({"declined": "tax waits"})
    );

    let typo = diag(at(&results, 14));
    assert_eq!(typo["code"], "input.invalid");
    assert!(
        typo["did_you_mean"]
            .as_array()
            .expect("did_you_mean")
            .contains(&json!("requests")),
        "{typo:#?}"
    );
}
