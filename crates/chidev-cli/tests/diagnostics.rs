//! A refused operation prints `{"ok": false, "line": n, "diagnostics": [...]}`: the rule that was broken (`code`),
//! what it is about, the field of the input, the typed `values`, the rule's `help`, `suggestions` that mend it and
//! `did_you_mean` names. These tests read the structure, not the sentences, wherever the structure says it.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// A directory of its own under cargo's temporary one, with an empty `chidev.toml` (it is a project's root) and
/// nothing else, so that a run starts from no store.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("chidev.toml"), "").expect("a file");
    dir
}

/// Runs `ops` through `chidev exec` in a project of its own; the result of each, in order.
fn run(name: &str, ops: &[Value]) -> Vec<Value> {
    let dir = fresh_dir(name);
    let file = dir.join("ops.jsonl");
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(&file, text).expect("a file");
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(&dir)
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

/// A project with the Context `core`, a Contract `rounding` (C1), an Interface `api` and a Harness `tests` whose
/// Case cites C1 and the Item `price`.
fn setup() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"contract_put","context":"core","name":"rounding","title":"Rounding",
            "clauses":[{"label":"C1","text":"a half rounds up"}]}),
        json!({"op":"interface_put","context":"core","name":"api","source":"fn price()\n",
            "items":[{"key":"price","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"tests","source":"fn t() {}\n",
            "cases":[{"name":"t","lines":[1,1],
                "items":[{"interface":"api","key":"price"}],
                "clauses":[{"contract":"rounding","label":"C1"}]}]}),
    ]
}

/// The setup, then `more`; the results of `more`.
fn after_setup(name: &str, more: &[Value]) -> Vec<Value> {
    let mut ops = setup();
    let skip = ops.len();
    ops.extend(more.iter().cloned());
    let results = run(name, &ops);
    assert!(
        results[..skip].iter().all(|r| r["ok"] == true),
        "{results:#?}"
    );
    results[skip..].to_vec()
}

fn cr_open(title: &str, parent: Option<&str>) -> Value {
    let mut op = json!({"op":"cr_open","title":title,"intent":"change_behavior"});
    if let Some(parent) = parent {
        op["parent"] = json!(parent);
    }
    op
}

fn by_alice() -> Value {
    json!({"person":"alice"})
}

/// The one diagnostic of a refused result.
fn one(result: &Value) -> &Value {
    assert_eq!(result["ok"], false, "{result:#?}");
    let all = result["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(all.len(), 1, "{result:#?}");
    &all[0]
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array()
        .expect("a list")
        .iter()
        .map(|s| s.as_str().expect("a string"))
        .collect()
}

#[test]
fn a_parent_with_open_parts_names_the_parts_and_the_operation_that_resolves_each() {
    let r = after_setup(
        "diag-parent-open",
        &[
            cr_open("Parent", None),
            cr_open("Part one", Some("Parent")),
            cr_open("Part two", Some("Parent")),
            json!({"op":"cr_resolve","title":"Parent","resolution":"done","reason":"all done","by":by_alice()}),
        ],
    );
    let d = one(&r[3]);
    assert_eq!(d["code"], "cr.parent-open");
    assert_eq!(d["level"], "error");
    assert_eq!(
        d["message"],
        "the change request `Parent` has 2 part(s) that are not resolved: `Part one`, `Part two`"
    );
    assert_eq!(d["values"]["count"], 2);
    let subjects: Vec<&str> = d["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, ["Parent", "Part one", "Part two"]);
    assert_eq!(d["subjects"][1]["kind"], "change_request");
    assert!(d["help"].as_str().unwrap().contains("Resolve each part"));
    let ops: Vec<(&str, &str)> = d["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            assert_eq!(s["op"]["op"], "cr_resolve");
            assert_eq!(s["applicability"], "placeholder");
            (
                s["op"]["title"].as_str().unwrap(),
                s["op"]["resolution"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(ops, [("Part one", "done"), ("Part two", "done")]);
    assert_eq!(r[3]["line"], 9);
}

#[test]
fn done_with_an_unanswered_crux_suggests_deciding_it() {
    let r = after_setup(
        "diag-crux-unanswered",
        &[
            json!({"op":"cr_open","title":"Choose rounding","intent":"change_behavior",
                "crux":{"kind":"ambiguity","options":[{"choice":"up"},{"choice":"even"}]}}),
            json!({"op":"cr_resolve","title":"Choose rounding","resolution":"done","reason":"r","by":by_alice()}),
            json!({"op":"cr_decide","title":"Choose rounding","choice":"up","basis":{"arbitrary":"x"},"by":by_alice()}),
            json!({"op":"cr_decide","title":"Choose rounding","choice":"even","basis":{"arbitrary":"y"},"by":by_alice()}),
        ],
    );
    let d = one(&r[1]);
    assert_eq!(d["code"], "cr.crux-unanswered");
    assert_eq!(d["subjects"][0]["name"], "Choose rounding");
    assert_eq!(d["suggestions"][0]["op"]["op"], "cr_decide");
    assert_eq!(d["suggestions"][0]["op"]["title"], "Choose rounding");
    assert_eq!(d["suggestions"][0]["applicability"], "placeholder");
    // once answered, a second answer is refused, and a new change request that supersedes it is suggested
    assert_eq!(r[2]["ok"], true, "{r:#?}");
    let again = one(&r[3]);
    assert_eq!(again["code"], "cr.crux-answered");
    assert_eq!(again["suggestions"][0]["op"]["op"], "cr_open");
    assert_eq!(
        again["suggestions"][0]["op"]["crux"]["supersedes"],
        "Choose rounding"
    );
}

#[test]
fn a_change_set_for_a_change_request_that_is_not_open_suggests_the_close_title_and_opening_it() {
    let r = after_setup(
        "diag-for-unknown",
        &[
            cr_open("Round halves up", None),
            json!({"op":"change_set","for":"Round halfs up","puts":[{"put":"context","name":"extra"}]}),
            json!({"op":"change_set","for":"Brand new","puts":[{"put":"context","name":"extra"}]}),
        ],
    );
    let near = one(&r[1]);
    assert_eq!(near["code"], "input.unknown-name");
    assert_eq!(near["field"], "for");
    assert_eq!(
        near["values"],
        json!({"kind":"change request","name":"Round halfs up"})
    );
    assert_eq!(strings(&near["did_you_mean"]), ["Round halves up"]);
    let make = &near["suggestions"][0];
    assert_eq!(
        make["op"],
        json!({"op":"cr_open","title":"Round halfs up","intent":"change_behavior"})
    );
    assert_eq!(make["applicability"], "review");

    let far = one(&r[2]);
    assert_eq!(far["field"], "for");
    assert!(far["did_you_mean"].as_array().unwrap().is_empty());
    assert_eq!(far["suggestions"][0]["op"]["title"], "Brand new");
}

#[test]
fn retiring_a_cited_contract_names_the_citing_case_and_the_harness_to_retire_first() {
    let r = after_setup(
        "diag-retire-cited",
        &[json!({"op":"contract_retire","context":"core","contract":"rounding"})],
    );
    let d = one(&r[0]);
    assert_eq!(d["code"], "project.retire-cited");
    assert_eq!(d["subjects"][0]["kind"], "case");
    assert_eq!(d["subjects"][0]["name"], "case tests t");
    assert_eq!(strings(&d["values"]["cited"]), ["rounding C1"]);
    assert_eq!(
        d["message"],
        "cannot retire: `case tests t` cite `rounding C1`"
    );
    let fix = &d["suggestions"][0];
    assert_eq!(
        fix["op"],
        json!({"op":"harness_retire","context":"core","harness":"tests"})
    );
    assert_eq!(fix["applicability"], "review");
}

#[test]
fn a_citation_that_cannot_be_reached_names_the_case_and_the_clause() {
    let r = after_setup(
        "diag-cite-unreachable",
        &[
            json!({"op":"contract_put","context":"core","name":"rounding","title":"Rounding",
            "clauses":[{"label":"C2","text":"another"}]}),
        ],
    );
    let d = one(&r[0]);
    assert_eq!(d["code"], "project.cite-unreachable");
    assert_eq!(d["values"]["kind"], "clause");
    let subjects: Vec<(&str, &str)> = d["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["kind"].as_str().unwrap(), s["name"].as_str().unwrap()))
        .collect();
    assert_eq!(
        subjects,
        [
            ("case", "case tests t"),
            ("clause", "rounding C1"),
            ("context", "context core")
        ]
    );
    assert!(d["message"].as_str().unwrap().contains("`rounding C1`"));
    assert!(d["message"].as_str().unwrap().contains("`case tests t`"));
    let remedies = d["suggestions"].as_array().unwrap();
    assert_eq!(remedies[0]["op"]["op"], "harness_put");
    assert_eq!(remedies[0]["op"]["name"], "tests");
    assert_eq!(remedies[0]["applicability"], "placeholder");
    assert!(remedies[1]["op"].is_string());
}

#[test]
fn an_input_that_cannot_be_read_is_a_rule_of_its_own_with_the_names_that_would_be_right() {
    let r = after_setup(
        "diag-unreadable",
        &[
            json!({"op":"cr_opn","title":"x"}),
            json!({"op":"context_add"}),
            json!({"op":"context_add","name":["core"]}),
            json!({"op":"cr_open","title":"T","intent":"change_behavior","situaton":"x"}),
            json!({"op":"contract_put","context":"cor","name":"x","title":"X","clauses":[]}),
            json!({"op":"harness_put","context":"core","name":"h","source":"x\n",
                "cases":[{"name":"c","lines":[1,1],"items":[{"interface":"ap","key":"price"}]}]}),
            json!({"op":"harness_put","context":"core","name":"h","source":"x\n",
                "cases":[{"name":"c","lines":[1,1],"items":[{"interface":"api","key":"pric"}]}]}),
            json!({"op":"cr_decide","title":"Nobody","choice":"a","basis":{"arbitrary":"x"},"by":by_alice()}),
        ],
    );
    let op = one(&r[0]);
    assert_eq!(op["code"], "input.unknown-op");
    assert_eq!(op["field"], "op");
    assert_eq!(strings(&op["did_you_mean"]), ["cr_open"]);

    let missing = one(&r[1]);
    assert_eq!(missing["code"], "input.missing-field");
    assert_eq!(missing["values"]["name"], "name");

    assert_eq!(one(&r[2])["code"], "input.wrong-type");

    let field = one(&r[3]);
    assert_eq!(field["code"], "input.unknown-field");
    assert_eq!(strings(&field["did_you_mean"]), ["situation"]);
    assert_eq!(field["field"], "situaton");

    let context = one(&r[4]);
    assert_eq!(context["code"], "input.unknown-name");
    assert_eq!(context["values"]["kind"], "context");
    assert_eq!(context["field"], "context");
    assert_eq!(strings(&context["did_you_mean"]), ["core"]);
    assert_eq!(
        context["suggestions"][0]["op"],
        json!({"op":"context_add","name":"cor"})
    );

    let interface = one(&r[5]);
    assert_eq!(interface["values"]["kind"], "interface");
    assert_eq!(interface["field"], "cases[0].items[0].interface");
    assert_eq!(strings(&interface["did_you_mean"]), ["api"]);

    let key = one(&r[6]);
    assert_eq!(key["values"]["kind"], "item");
    assert_eq!(strings(&key["did_you_mean"]), ["price"]);
    assert_eq!(key["subjects"][0]["name"], "api");

    let title = one(&r[7]);
    assert_eq!(title["values"]["kind"], "change request");
    assert_eq!(title["field"], "title");
}

#[test]
fn the_rules_are_listed_and_each_is_explained() {
    let r = run(
        "diag-rules",
        &[
            json!({"op":"rules"}),
            json!({"op":"explain","rule":"cr.parent-open"}),
            json!({"op":"explain","rule":"cite.stale"}),
            json!({"op":"explain","rule":"cr.parent-opn"}),
        ],
    );
    assert_eq!(r[0]["ok"], true);
    let rules = r[0]["rules"].as_array().unwrap();
    let find = |id: &str| {
        rules
            .iter()
            .find(|x| x["id"] == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    assert_eq!(find("cr.parent-open")["kind"], "invariant");
    assert_eq!(find("cr.parent-open")["level"], "error");
    assert_eq!(find("input.unknown-name")["kind"], "input");
    assert_eq!(find("cite.stale")["kind"], "check");
    assert!(!find("cite.stale")["summary"].as_str().unwrap().is_empty());
    assert!(rules.iter().any(|x| x["id"] == "domain.unmapped"));

    let e = &r[1];
    assert_eq!(e["ok"], true);
    assert_eq!(e["op"], "explain");
    assert_eq!(e["id"], "cr.parent-open");
    assert_eq!(e["kind"], "invariant");
    assert_eq!(e["values"], json!(["count"]));
    assert!(e["help"].as_str().unwrap().contains("Resolve each part"));
    assert!(e["example"].as_str().unwrap().contains("cr_resolve"));

    assert_eq!(r[2]["kind"], "check");
    assert_eq!(r[2]["executor"], "builtin");

    let missing = one(&r[3]);
    assert_eq!(missing["code"], "input.unknown-name");
    assert_eq!(strings(&missing["did_you_mean"]), ["cr.parent-open"]);
}

#[test]
fn the_example_of_each_rule_is_an_operation_chidev_reads_or_refuses_by_its_rule() {
    let all = run("diag-rules-list", &[json!({"op":"rules"})]);
    let ids: Vec<String> = all[0]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["kind"] != "check")
        .map(|x| x["id"].as_str().unwrap().to_string())
        .collect();
    let ops: Vec<Value> = ids
        .iter()
        .map(|id| json!({"op":"explain","rule":id}))
        .collect();
    let explained = run("diag-rules-explain", &ops);
    for e in &explained {
        let example: Value = serde_json::from_str(e["example"].as_str().unwrap())
            .unwrap_or_else(|err| panic!("{}: {err}", e["id"]));
        assert!(example["op"].is_string(), "{}", e["id"]);
        // An invariant's example is an operation chidev reads; an input rule's example is the one it cannot.
        let tried = run("diag-rules-try", &[example]);
        if e["kind"] == "invariant" {
            let code = tried[0]["diagnostics"][0]["code"].as_str().unwrap_or("");
            assert!(
                !code.starts_with("input.unknown-op") && !code.starts_with("input.unknown-field"),
                "{}: its example is not read: {tried:#?}",
                e["id"]
            );
        }
    }
}

#[test]
fn the_example_of_each_rule_of_the_items_of_an_interface_is_refused_by_that_rule() {
    let ids = [
        "interface.key-taken",
        "interface.blank-item",
        "input.replaces-kept",
        "input.rename-conflict",
    ];
    let ops: Vec<Value> = ids
        .iter()
        .map(|id| json!({"op":"explain","rule":id}))
        .collect();
    let examples: Vec<Value> = run("diag-item-rules-explain", &ops)
        .iter()
        .map(|e| serde_json::from_str(e["example"].as_str().unwrap()).expect("an operation"))
        .collect();
    let refused = after_setup("diag-item-rules", &examples);
    for (id, result) in ids.iter().zip(&refused) {
        assert_eq!(one(result)["code"], *id, "{result:#?}");
    }
    // the sentences name the key and the Interface, not an id
    let kept = one(&refused[2]);
    assert_eq!(kept["values"]["key"], "b");
    assert_eq!(kept["subjects"][1]["name"], "api");
    let taken = one(&refused[0]);
    assert_eq!(taken["subjects"][0]["name"], "api a");
}

#[test]
fn a_finding_is_a_diagnostic_too_with_its_rule_as_the_code_and_the_help_of_the_rule() {
    let r = after_setup("diag-finding", &[json!({"op":"check"})]);
    let findings = r[0]["findings"].as_array().unwrap();
    assert!(!findings.is_empty(), "{r:#?}");
    for f in findings {
        assert_eq!(f["code"], f["rule"]);
        assert!(f["level"].is_string() && f["message"].is_string());
        assert!(!f["help"].as_str().unwrap().is_empty(), "{f}");
        assert!(f["state"].is_string());
        assert!(f["suggestions"].is_array() && f["did_you_mean"].is_array());
    }
}
