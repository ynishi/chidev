//! Runs `tests/flow.jsonl` through `chidev exec`: two Contexts get their Contract, Interfaces and Harnesses, are
//! pinned; a change request is opened, with a part whose Crux a person decides once, on a stated basis; the
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

    // the request is open; its part decided, with the clause still to be written
    let decided = at(&results, 16);
    let crs = &decided["change_requests"];
    assert_eq!(crs[0]["title"], "Fair totals");
    assert_eq!(crs[0]["state"], "open");
    assert_eq!(crs[0]["crux"], Value::Null);
    assert_eq!(crs[1]["parent"], "Fair totals");
    assert_eq!(crs[1]["issuer"], "designer");
    assert_eq!(crs[1]["crux"], "ambiguity");
    assert_eq!(crs[1]["state"], "decided");
    assert_eq!(crs[1]["answer"]["decision"], "half to even");
    assert_eq!(
        crs[1]["answer"]["basis"],
        serde_json::json!({ "arbitrary": { "reason": "the shop accepts the cost of fairness" } })
    );
    assert_eq!(
        crs[1]["answer"]["by"],
        serde_json::json!({ "kind": "person", "name": "alice" })
    );
    assert!(crs[1]["answer"]["at"].as_u64().expect("a time") > 0);
    assert_eq!(crs[0]["answer"], Value::Null);
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
        // 4: the settled change (C1, a line) and the question about a detail (which error; C2 for a new one)
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Refuse big discounts\",\"intent\":\"change_behavior\",",
            "\"change\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C1\"}}],",
            "\"crux\":{\"kind\":\"ambiguity\",\"options\":[",
            "{\"choice\":\"reuse InvalidDiscount\"},",
            "{\"choice\":\"new error\",\"change\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C2\"}}]}",
            "],\"recommended\":{\"choice\":\"reuse InvalidDiscount\",\"reason\":\"no new type\",\"confidence\":60},",
            "\"importance\":\"normal\"}}\n"
        )
        .to_string(),
        "{\"op\":\"status\"}\n".to_string(),
        "{\"op\":\"check\"}\n".to_string(),
        concat!(
            "{\"op\":\"cr_decide\",\"title\":\"Refuse big discounts\",\"choice\":\"reuse InvalidDiscount\",",
            "\"basis\":{\"arbitrary\":\"one error type is enough\"},\"by\":{\"person\":\"alice\"}}\n"
        )
        .to_string(),
        // 8: decided, yet C1 never changed: it does not resolve, and says which definition
        "{\"op\":\"cr_resolve\",\"title\":\"Refuse big discounts\"}\n".to_string(),
        "{\"op\":\"status\"}\n".to_string(),
        discount_put(
            "a discount above 50% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        "{\"op\":\"cr_resolve\",\"title\":\"Refuse big discounts\"}\n".to_string(),
        // 12: a question whose second option asks C2 to change
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Name the error\",\"intent\":\"change_behavior\",",
            "\"crux\":{\"kind\":\"ambiguity\",\"options\":[",
            "{\"choice\":\"reuse InvalidDiscount\"},",
            "{\"choice\":\"new error\",\"change\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C2\"}}]}",
            "]}}\n"
        )
        .to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Name the error\"}\n".to_string(),
        concat!(
            "{\"op\":\"cr_decide\",\"title\":\"Name the error\",\"choice\":\"new error\",",
            "\"basis\":{\"arbitrary\":\"callers tell the errors apart\"},\"by\":{\"person\":\"alice\"}}\n"
        )
        .to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Name the error\"}\n".to_string(),
        discount_put(
            "a discount above 50% is refused",
            "a refused discount is a DiscountTooLarge",
        ),
        "{\"op\":\"cr_resolve\",\"title\":\"Name the error\"}\n".to_string(),
        "{\"op\":\"status\"}\n".to_string(),
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [8, 13, 15], "{results:#?}");

    // cr_open echoes what it recorded, by name
    let opened = at(&results, 4);
    assert_eq!(
        opened["expects"],
        serde_json::json!([{
            "kind": "change",
            "target": { "clause": { "context": "pricing", "contract": "discount", "label": "C1" } },
            "recorded": 1
        }])
    );

    // the undecided question is visible: in status, and in check as a note; the line is behind as well
    let waiting = at(&results, 5);
    assert_eq!(
        waiting["awaiting"],
        serde_json::json!([{
            "cr": "Refuse big discounts",
            "kind": "ambiguity",
            "options": [
                { "choice": "reuse InvalidDiscount", "wording": null, "expects": [], "consequence": null },
                { "choice": "new error", "wording": null,
                  "expects": [{ "kind": "change", "target": "discount C2" }], "consequence": null }
            ],
            "recommended": { "choice": "reuse InvalidDiscount", "reason": "no new type", "confidence": 60 },
            "importance": "normal"
        }])
    );
    assert_eq!(
        waiting["policy"],
        serde_json::json!({ "revision": 1, "lines": [] })
    );
    assert_eq!(waiting["unapplied"][0]["unchanged"][0], "discount C1");
    let finding = |r: &Value, rule: &str| -> Option<Value> {
        r["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .find(|f| f["rule"] == rule)
            .cloned()
    };
    let note = finding(at(&results, 6), "cr.awaiting-decision").expect("awaiting is found");
    assert_eq!(note["level"], "note");
    assert!(finding(at(&results, 6), "cr.unapplied").is_some());

    // decided and not resolved: the error names the definition, not its id
    let refused = at(&results, 8)["error"].as_str().expect("error");
    assert!(refused.contains("discount C1"), "{refused}");
    assert!(
        !refused.contains("ClauseId") && !refused.contains("Clause("),
        "{refused}"
    );
    let decided = at(&results, 9);
    assert_eq!(decided["awaiting"], serde_json::json!([]));
    assert_eq!(decided["unapplied"][0]["decision"], "reuse InvalidDiscount");
    assert_eq!(at(&results, 11)["ok"], true, "{results:#?}");

    // the second question: awaited, refused undecided, and after the choice of the second option C2 is required
    assert_eq!(at(&results, 13)["ok"], false);
    assert!(
        at(&results, 13)["error"]
            .as_str()
            .expect("error")
            .contains("no decision")
    );
    let asked = at(&results, 15)["error"].as_str().expect("error");
    assert!(asked.contains("discount C2"), "{asked}");
    assert_eq!(at(&results, 17)["ok"], true, "{results:#?}");
    let last = at(&results, 18);
    assert_eq!(last["awaiting"], serde_json::json!([]));
    assert_eq!(last["unapplied"], serde_json::json!([]));
    assert_eq!(last["change_requests"][0]["state"], "resolved");
    assert_eq!(last["change_requests"][1]["state"], "resolved");
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
    let crux = |kind: &str, recommended: &str, importance: &str, supersedes: &str| {
        format!(
            "\"crux\":{{\"kind\":\"{kind}\",\"options\":[{{\"choice\":\"half up\"}},{{\"choice\":\"half to even\"}}]{recommended}{importance}{supersedes}}}"
        )
    };
    let ops = [
        "{\"op\":\"init\"}\n".to_string(),
        "{\"op\":\"context_add\",\"name\":\"pricing\"}\n".to_string(),
        discount_put("a half rounds up", "a refused discount is an InvalidDiscount"),
        // 4: the decision Policy
        policy_put(&[
            ("P1", "take the recommendation when its confidence is 80 or more"),
            ("P2", "when the importance is low and the options cost the same, either is fine"),
        ]),
        // 5: a Crux whose recommendation is 85 sure
        format!(
            "{{\"op\":\"cr_open\",\"title\":\"Round half to even?\",\"intent\":\"change_behavior\",{}}}\n",
            crux(
                "ambiguity",
                ",\"recommended\":{\"choice\":\"half to even\",\"reason\":\"no bias\",\"confidence\":85}",
                ",\"importance\":\"low\"",
                ""
            )
        ),
        // 6: a Crux nothing derives
        format!(
            "{{\"op\":\"cr_open\",\"title\":\"Name the error\",\"intent\":\"change_behavior\",{}}}\n",
            crux("conflict", "", ",\"importance\":\"high\"", "")
        ),
        "{\"op\":\"status\"}\n".to_string(),
        // 8: the orchestrator applies P1
        concat!(
            "{\"op\":\"cr_decide\",\"title\":\"Round half to even?\",\"choice\":\"half to even\",",
            "\"basis\":{\"policy\":\"P1\"},\"by\":{\"role\":\"orchestrator\"}}\n"
        )
        .to_string(),
        // 9: a person decides what no line derives
        concat!(
            "{\"op\":\"cr_decide\",\"title\":\"Name the error\",\"choice\":\"half up\",\"reason\":\"keep what callers know\",",
            "\"basis\":{\"arbitrary\":\"the requester wants the old behaviour kept\"},\"by\":{\"person\":\"alice\"}}\n"
        )
        .to_string(),
        "{\"op\":\"status\"}\n".to_string(),
        // 11: an answer is never rewritten
        concat!(
            "{\"op\":\"cr_decide\",\"title\":\"Round half to even?\",\"choice\":\"half up\",",
            "\"basis\":{\"arbitrary\":\"changed my mind\"},\"by\":{\"person\":\"alice\"}}\n"
        )
        .to_string(),
        // 12: a third change request replaces the first decision
        format!(
            "{{\"op\":\"cr_open\",\"title\":\"Round half up after all\",\"intent\":\"change_behavior\",{}}}\n",
            crux("ambiguity", "", "", ",\"supersedes\":\"Round half to even?\"")
        ),
        "{\"op\":\"cr_open\",\"title\":\"Plain\",\"intent\":\"keep_behavior\"}\n".to_string(),
        // 14: there is nothing to replace in a change request without an answered Crux
        format!(
            "{{\"op\":\"cr_open\",\"title\":\"Bad supersede\",\"intent\":\"change_behavior\",{}}}\n",
            crux("ambiguity", "", "", ",\"supersedes\":\"Plain\"")
        ),
        // 15: an Assumption appended to an open change request
        concat!(
            "{\"op\":\"cr_assume\",\"title\":\"Plain\",\"statement\":\"a half cent is a tie\",",
            "\"reason\":\"the rounding code treats it so\",\"evidence\":[",
            "{\"repo\":{\"path\":\"src/round.rs\",\"lines\":[10,14]}},",
            "{\"knowledge\":{\"reference\":\"IEEE 754 roundTiesToEven\"}}]}\n"
        )
        .to_string(),
        // 16: evidence that leaves the repository
        concat!(
            "{\"op\":\"cr_assume\",\"title\":\"Plain\",\"statement\":\"x\",\"reason\":\"y\",",
            "\"evidence\":[{\"repo\":{\"path\":\"../secret\"}}]}\n"
        )
        .to_string(),
        // 17: a Policy line the policy does not have
        concat!(
            "{\"op\":\"cr_decide\",\"title\":\"Round half up after all\",\"choice\":\"half up\",",
            "\"basis\":{\"policy\":\"P9\"},\"by\":{\"role\":\"harness-lead\"}}\n"
        )
        .to_string(),
        // 18: the harness lead applies P2
        concat!(
            "{\"op\":\"cr_decide\",\"title\":\"Round half up after all\",\"choice\":\"half up\",",
            "\"basis\":{\"policy\":\"P2\"},\"by\":{\"role\":\"harness-lead\"}}\n"
        )
        .to_string(),
        "{\"op\":\"status\"}\n".to_string(),
        // 20: the policy is replaced: P1 is reworded and P2 is gone
        policy_put(&[("P1", "take the recommendation when its confidence is 90 or more")]),
        "{\"op\":\"status\"}\n".to_string(),
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [11, 14, 16, 17], "{results:#?}");

    // the policy: revision 2 after the first put (1 is the empty one)
    assert_eq!(at(&results, 4)["revision"], 2);

    // both Cruxes are awaited, with what a decision maker needs to choose
    let waiting = at(&results, 7);
    let awaiting = waiting["awaiting"].as_array().expect("awaiting");
    assert_eq!(awaiting.len(), 2);
    assert_eq!(awaiting[0]["cr"], "Round half to even?");
    assert_eq!(awaiting[0]["kind"], "ambiguity");
    assert_eq!(awaiting[0]["importance"], "low");
    assert_eq!(
        awaiting[0]["recommended"],
        serde_json::json!({ "choice": "half to even", "reason": "no bias", "confidence": 85 })
    );
    assert_eq!(awaiting[0]["options"].as_array().expect("options").len(), 2);
    assert_eq!(awaiting[1]["recommended"], Value::Null);
    assert_eq!(awaiting[1]["importance"], "high");
    assert_eq!(
        waiting["policy"]["lines"][0],
        serde_json::json!({
            "id": "P1",
            "text": "take the recommendation when its confidence is 80 or more"
        })
    );
    assert_eq!(waiting["policy"]["revision"], 2);

    // status shows both answers: who decided, on what basis, and when
    let decided = at(&results, 10);
    assert_eq!(decided["awaiting"], serde_json::json!([]));
    let first = &decided["change_requests"][0]["answer"];
    assert_eq!(first["decision"], "half to even");
    assert_eq!(
        first["basis"],
        serde_json::json!({ "policy": { "line": "P1", "policy_revision": 2 } })
    );
    assert_eq!(
        first["by"],
        serde_json::json!({ "kind": "role", "name": "orchestrator" })
    );
    assert!(first["at"].as_u64().expect("a time") > 0);
    let second = &decided["change_requests"][1]["answer"];
    assert_eq!(second["decision"], "half up");
    assert_eq!(
        second["basis"],
        serde_json::json!({ "arbitrary": { "reason": "the requester wants the old behaviour kept" } })
    );
    assert_eq!(
        second["by"],
        serde_json::json!({ "kind": "person", "name": "alice" })
    );

    // the answer stands
    assert!(error_at(&results, 11).contains("answered already"));
    // nothing to replace, a Policy line that is not there, evidence outside the repository
    assert!(error_at(&results, 14).contains("no answered crux"));
    assert!(error_at(&results, 16).contains(".."));
    assert!(error_at(&results, 17).contains("no line P9"));

    // the third change request supersedes the first; the Assumption is on its change request
    let later = at(&results, 19);
    let crs = &later["change_requests"];
    assert_eq!(crs[2]["title"], "Round half up after all");
    assert_eq!(crs[2]["supersedes"], "Round half to even?");
    assert_eq!(crs[2]["state"], "decided");
    assert_eq!(
        crs[2]["answer"]["by"],
        serde_json::json!({ "kind": "role", "name": "harness-lead" })
    );
    assert_eq!(crs[0]["answer"]["decision"], "half to even");
    assert_eq!(crs[3]["title"], "Plain");
    assert_eq!(
        crs[3]["assumptions"],
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
    let replaced = at(&results, 20);
    assert_eq!(replaced["revision"], 3);
    let gone: Vec<&Value> = replaced["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"] == "crux.policy-line-gone")
        .collect();
    assert_eq!(gone.len(), 2, "{replaced:#?}");
    assert!(gone.iter().all(|f| f["level"] == "note"));
    let after = at(&results, 21);
    assert_eq!(after["change_requests"][0]["answer"], *first);
    assert_eq!(after["policy"]["lines"].as_array().expect("lines").len(), 1);
}

/// The `error` of the operation on line `n`.
fn error_at(results: &[Value], n: u64) -> &str {
    at(results, n)["error"].as_str().expect("an error")
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
        // 4: adding a Context is a change of the Project
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Add billing\",\"intent\":\"change_behavior\",",
            "\"change\":[{\"project\":true}]}\n"
        )
        .to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Add billing\"}\n".to_string(),
        "{\"op\":\"context_add\",\"name\":\"billing\"}\n".to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Add billing\"}\n".to_string(),
        // 8: removing a Clause, while the other stays as it is
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Drop C2\",\"intent\":\"change_behavior\",",
            "\"remove\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C2\"}}],",
            "\"keep\":[{\"clause\":{\"context\":\"pricing\",\"contract\":\"discount\",\"label\":\"C1\"}}]}\n"
        )
        .to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Drop C2\"}\n".to_string(),
        "{\"op\":\"contract_put\",\"context\":\"pricing\",\"name\":\"discount\",\"title\":\"Discount\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a discount above 50% is refused\"}]}\n"
            .to_string(),
        "{\"op\":\"cr_resolve\",\"title\":\"Drop C2\"}\n".to_string(),
        "{\"op\":\"status\"}\n".to_string(),
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [5, 9], "{results:#?}");

    assert_eq!(
        at(&results, 4)["expects"],
        serde_json::json!([{ "kind": "change", "target": { "project": true }, "recorded": 2 }])
    );
    // no Context was added yet: the Project did not change
    assert!(error_at(&results, 5).contains("not changed: project"));

    // the Clause is still there: it says which one, by name
    assert!(error_at(&results, 9).contains("not removed: discount C2"));
    // (two Clauses are in the order of their ids, which are not known here)
    let recorded = at(&results, 8)["expects"].as_array().expect("expects");
    assert_eq!(recorded.len(), 2);
    for (kind, label) in [("remove", "C2"), ("keep", "C1")] {
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
    assert_eq!(last["unapplied"], serde_json::json!([]));
    assert_eq!(last["keep_moved"], serde_json::json!([]));
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
        // 6: the parent waits for its part
        "{\"op\":\"cr_resolve\",\"title\":\"Umbrella\"}\n".to_string(),
        discount_put(
            "a discount above 40% is refused",
            "a refused discount is an InvalidDiscount",
        ),
        "{\"op\":\"status\"}\n".to_string(),
        "{\"op\":\"check\"}\n".to_string(),
        // 10: the clause it expected to keep moved: it cannot be resolved
        "{\"op\":\"cr_resolve\",\"title\":\"Part\"}\n".to_string(),
    ]
    .concat();
    let (results, code, err) = exec_in(&dir, &ops);
    assert_eq!(code, Some(1), "{err}\n{results:#?}");
    let failed: Vec<u64> = results
        .iter()
        .filter(|r| r["ok"] == false)
        .map(|r| r["line"].as_u64().expect("line"))
        .collect();
    assert_eq!(failed, [6, 10], "{results:#?}");

    let parent = error_at(&results, 6);
    assert!(
        parent.contains("not resolved") && parent.contains("Part"),
        "{parent}"
    );

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

    let kept = error_at(&results, 10);
    assert!(
        kept.contains("stay as they are") && kept.contains("discount C1"),
        "{kept}"
    );
}

#[test]
fn a_reference_names_a_target_of_each_kind_and_cr_open_echoes_it_by_name() {
    let dir = fresh_dir("every-kind");
    let ops = [
        "{\"op\":\"init\"}\n",
        "{\"op\":\"context_add\",\"name\":\"core\"}\n",
        "{\"op\":\"contract_put\",\"context\":\"core\",\"name\":\"rules\",\"title\":\"Rules\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a value is returned\"}]}\n",
        "{\"op\":\"interface_put\",\"context\":\"core\",\"name\":\"api\",\"source\":\"fn value();\\n\",\"items\":[{\"signature\":\"fn value()\",\"lines\":[1,1]}]}\n",
        "{\"op\":\"harness_put\",\"context\":\"core\",\"name\":\"tests\",\"source\":\"fn checks_value() {}\\n\",\"cases\":[{\"name\":\"checks_value\",\"lines\":[1,1],\"items\":[{\"interface\":\"api\",\"signature\":\"fn value()\"}],\"clauses\":[{\"contract\":\"rules\",\"label\":\"C1\"}]}]}\n",
        // 6: one of each kind, in the three lists (the Context is kept; a Case is changed)
        concat!(
            "{\"op\":\"cr_open\",\"title\":\"Everything\",\"intent\":\"change_behavior\",",
            "\"change\":[{\"project\":true},{\"contract\":{\"context\":\"core\",\"contract\":\"rules\"}},",
            "{\"interface\":{\"context\":\"core\",\"interface\":\"api\"}},{\"case\":{\"context\":\"core\",\"harness\":\"tests\",\"name\":\"checks_value\"}}],",
            "\"remove\":[{\"item\":{\"context\":\"core\",\"interface\":\"api\",\"signature\":\"fn value()\"}}],",
            "\"keep\":[{\"context\":\"core\"},{\"harness\":{\"context\":\"core\",\"harness\":\"tests\"}},",
            "{\"clause\":{\"context\":\"core\",\"contract\":\"rules\",\"label\":\"C1\"}}]}\n"
        ),
        // 7: a reference of no kind, one that is not in the Project, and `false` for the Project
        "{\"op\":\"cr_open\",\"title\":\"Nothing\",\"intent\":\"change_behavior\",\"change\":[{\"nothing\":true}]}\n",
        "{\"op\":\"cr_open\",\"title\":\"Missing\",\"intent\":\"change_behavior\",\"remove\":[{\"harness\":{\"context\":\"core\",\"harness\":\"gone\"}}]}\n",
        "{\"op\":\"cr_open\",\"title\":\"Twice\",\"intent\":\"change_behavior\",\"change\":[{\"context\":\"core\"}],\"keep\":[{\"context\":\"core\"}]}\n",
        "{\"op\":\"cr_open\",\"title\":\"No project\",\"intent\":\"change_behavior\",\"change\":[{\"project\":false}]}\n",
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
            { "kind": "change", "target": { "project": true }, "recorded": 2 },
            { "kind": "keep", "target": { "context": "core" }, "recorded": 4 },
            { "kind": "change", "target": { "contract": core(serde_json::json!({ "contract": "rules" })) }, "recorded": 1 },
            { "kind": "change", "target": { "interface": core(serde_json::json!({ "interface": "api" })) }, "recorded": 1 },
            { "kind": "keep", "target": { "harness": core(serde_json::json!({ "harness": "tests" })) }, "recorded": 1 },
            { "kind": "keep", "target": { "clause": core(serde_json::json!({ "contract": "rules", "label": "C1" })) }, "recorded": 1 },
            { "kind": "remove", "target": { "item": core(serde_json::json!({ "interface": "api", "signature": "fn value()" })) }, "recorded": 1 },
            { "kind": "change", "target": { "case": core(serde_json::json!({ "harness": "tests", "name": "checks_value" })) }, "recorded": 1 }
        ])
    );
    assert!(
        error_at(&results, 8).contains("gone"),
        "{}",
        error_at(&results, 8)
    );
    assert!(
        error_at(&results, 9).contains("given twice"),
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
    assert_eq!(listed["checks"].as_array().expect("checks").len(), 24);
    assert_eq!(entry("crux.policy-line-gone")["level"], "note");
    assert_eq!(
        entry("crux.policy-line-gone")["hooks"],
        serde_json::json!(["put", "check"])
    );
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
    assert_eq!(model["version"], 2);
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

#[test]
fn a_retired_asset_leaves_the_model_but_its_pin_its_request_and_its_name_stay() {
    let dir = fresh_dir("retire");
    let ops = [
        "{\"op\":\"init\"}\n",
        "{\"op\":\"context_add\",\"name\":\"core\"}\n",
        "{\"op\":\"context_add\",\"name\":\"scratch\"}\n",
        "{\"op\":\"contract_put\",\"context\":\"core\",\"name\":\"rules\",\"title\":\"Rules\",\"clauses\":[{\"label\":\"C1\",\"text\":\"a value is returned\"}]}\n",
        "{\"op\":\"interface_put\",\"context\":\"core\",\"name\":\"api\",\"source\":\"fn value();\\n\",\"items\":[{\"signature\":\"fn value()\",\"lines\":[1,1]}]}\n",
        "{\"op\":\"harness_put\",\"context\":\"core\",\"name\":\"tests\",\"source\":\"fn checks_value() {}\\n\",\"cases\":[{\"name\":\"checks_value\",\"lines\":[1,1],\"items\":[{\"interface\":\"api\",\"signature\":\"fn value()\"}],\"clauses\":[{\"contract\":\"rules\",\"label\":\"C1\"}]}]}\n",
        "{\"op\":\"pin\",\"context\":\"core\",\"interface\":\"api\",\"code\":\"fn value() {}\"}\n",
        // 8: the request asks for the Interface to go
        "{\"op\":\"cr_open\",\"title\":\"Drop api\",\"intent\":\"change_behavior\",\"remove\":[{\"interface\":{\"context\":\"core\",\"interface\":\"api\"}}]}\n",
        // 9: a Case that stays cites the Clause: refused, naming the Case and the Clause
        "{\"op\":\"contract_retire\",\"context\":\"core\",\"contract\":\"rules\"}\n",
        // 10: the Interface is not gone yet
        "{\"op\":\"cr_resolve\",\"title\":\"Drop api\"}\n",
        // 11, 12: the Harness that cites goes first, then the Interface it cited
        "{\"op\":\"harness_retire\",\"context\":\"core\",\"harness\":\"tests\"}\n",
        "{\"op\":\"interface_retire\",\"context\":\"core\",\"interface\":\"api\"}\n",
        // 13: a retired target meets a Remove expectation
        "{\"op\":\"cr_resolve\",\"title\":\"Drop api\"}\n",
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
    assert_eq!(failed, [9, 10, 16], "{results:#?}");

    let refused = error_at(&results, 9);
    assert!(
        refused.contains("tests checks_value") && refused.contains("rules C1"),
        "{refused}"
    );
    assert!(
        error_at(&results, 10).contains("not removed: interface api"),
        "{}",
        error_at(&results, 10)
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
            line("item", "api fn value()", 1, 6)
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
    assert_eq!(status["unapplied"], serde_json::json!([]));
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

fn case_json(name: &str, line: usize, signature: &str, label: &str) -> Value {
    serde_json::json!({
        "name": name,
        "lines": [line, line],
        "items": [{ "interface": "stock", "signature": signature }],
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
                    "items": [{ "signature": "fn reserve()", "lines": [1, 1] }, { "signature": "fn release()", "lines": [2, 2] }] }),
            json!({ "op": "harness_put", "context": "core", "name": "stock-tests", "source": tests_source,
                    "cases": [
                        case_json("reserve_refuses", 1, "fn reserve()", "C1"),
                        case_json("release_frees", 2, "fn release()", "C7"),
                        case_json("release_twice", 3, "fn release()", "C8"),
                    ] }),
        ],
    );
    assert_eq!(code, Some(0), "{err}\n{setup:#?}");
    let before = store_revision(&dir);

    let contract = json!({ "put": "contract", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(6) });
    let interface = json!({ "put": "interface", "context": "core", "name": "stock", "source": "fn reserve();\n",
                            "items": [{ "signature": "fn reserve()", "lines": [1, 1] }] });
    let harness = json!({ "put": "harness", "context": "core", "name": "stock-tests", "source": "fn reserve_refuses() {}\n",
                          "cases": [case_json("reserve_refuses", 1, "fn reserve()", "C1")] });
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
        "the clause stock C7",
        "the clause stock C8",
        "the item stock fn release()",
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
            "stock fn release()",
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
                  "cases": [case_json("reserve_refuses", 1, "fn reserve()", "C1")] },
                { "put": "contract", "context": "core", "name": "stock", "title": "Stock", "clauses": stock_clauses(1) },
                { "put": "interface", "context": "core", "name": "stock", "source": "fn reserve();\n",
                  "items": [{ "signature": "fn reserve()", "lines": [1, 1] }] },
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
