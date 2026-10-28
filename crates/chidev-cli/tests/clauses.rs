//! `clauses_put`, the `clauses` entry of a change set, and `replaces` in a Clause of `contract_put`, through
//! `chidev exec`: a new label moves no revision (so nothing goes stale) and is a move of its own in a request's
//! footprint, as a new key of an Item is; a partial put leaves the other Clauses as they are; and a name that cannot
//! be carried out is refused by a rule with everything the entry has wrong said at once.

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

/// Runs `ops` through `chidev exec` in the project `dir`; the result of each, in order.
fn exec_in(dir: &Path, ops: &[Value]) -> Vec<Value> {
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
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect()
}

/// A Context `core` with the Contract `stock` (C1 to C4), the Interface `api` (the Item `price`) and the Harness
/// `tests`, whose Cases `a`, `b` and `c` cite C1, C2 and C3 (and the Item).
fn setup() -> Vec<Value> {
    let case = |name: &str, label: &str| {
        json!({"name":name,"lines":[1,1],
            "items":[{"interface":"api","key":"price"}],
            "clauses":[{"contract":"stock","label":label}]})
    };
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"contract_put","context":"core","name":"stock","title":"Stock","clauses":[
            {"label":"C1","text":"a unit is reserved"},
            {"label":"C2","text":"a unit is released"},
            {"label":"C3","text":"stock is not negative"},
            {"label":"C4","text":"a reservation is not kept for ever"}]}),
        json!({"op":"interface_put","context":"core","name":"api","source":"fn price()\n",
            "items":[{"key":"price","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"tests",
            "source":"fn a() {}\nfn b() {}\nfn c() {}\n",
            "cases":[case("a","C1"),case("b","C2"),case("c","C3")]}),
    ]
}

/// The setup, then `more`; the directory of the project and the results of `more`.
fn after_setup(name: &str, more: &[Value]) -> (PathBuf, Vec<Value>) {
    let dir = fresh_dir(name);
    let mut ops = setup();
    let skip = ops.len();
    ops.extend(more.iter().cloned());
    let results = exec_in(&dir, &ops);
    assert!(
        results[..skip].iter().all(|r| r["ok"] == true),
        "{results:#?}"
    );
    (dir, results[skip..].to_vec())
}

fn rules(result: &Value) -> Vec<&str> {
    result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule"].as_str().expect("a rule"))
        .collect()
}

fn diagnostics(result: &Value) -> &Vec<Value> {
    assert_eq!(result["ok"], false, "{result:#?}");
    result["diagnostics"].as_array().expect("diagnostics")
}

fn one(result: &Value) -> &Value {
    let all = diagnostics(result);
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

/// The clauses of the Contract `stock`, as `view` shows them.
fn clauses_of(result: &Value) -> &Vec<Value> {
    assert_eq!(result["ok"], true, "{result:#?}");
    result["view"]["contexts"][0]["contracts"][0]["clauses"]
        .as_array()
        .expect("clauses")
}

fn view() -> Value {
    json!({"op":"view","sections":["contracts"]})
}

fn cr_status<'a>(status: &'a Value, title: &str) -> &'a Value {
    status["change_requests"]
        .as_array()
        .expect("change requests")
        .iter()
        .find(|c| c["title"] == title)
        .unwrap_or_else(|| panic!("no change request {title}"))
}

#[test]
fn clauses_put_rewords_one_adds_one_and_removes_one_and_the_others_keep_their_revision() {
    let (_, r) = after_setup(
        "clauses-put-partial",
        &[
            view(),
            json!({"op":"clauses_put","context":"core","contract":"stock",
                "clauses":[{"label":"C1","text":"a unit is reserved at most once"},
                           {"label":"C5","text":"a unit is counted"}],
                "remove":["C4"]}),
            view(),
        ],
    );
    let before = clauses_of(&r[0]);
    assert_eq!(r[1]["ok"], true, "{:#?}", r[1]);
    assert_eq!(r[1]["op"], "clauses_put");
    assert_eq!(r[1]["revision"], 2);
    let after = clauses_of(&r[2]);
    let labels: Vec<&str> = after.iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["C1", "C2", "C3", "C5"]);
    let revisions: Vec<u64> = after
        .iter()
        .map(|c| c["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(revisions, [2, 1, 1, 1]);
    // the ids of the Clauses that stay are the ones they had
    for (now, was) in after.iter().zip(before).take(3) {
        assert_eq!(now["id"], was["id"]);
    }
    // the Case that cites the reworded Clause is stale, and the ones that cite the others are not
    let stale = rules(&r[1]).iter().filter(|r| **r == "cite.stale").count();
    assert_eq!(stale, 1, "{:#?}", r[1]);
}

#[test]
fn a_new_label_by_replaces_keeps_the_id_and_moves_no_revision_in_either_put() {
    let (_, r) = after_setup(
        "clauses-put-replaces",
        &[
            view(),
            json!({"op":"clauses_put","context":"core","contract":"stock",
                "clauses":[{"label":"C7","text":"a unit is released","replaces":"C2"}]}),
            view(),
            // the whole put, with the Clause given its label back and another renamed
            json!({"op":"contract_put","context":"core","name":"stock","title":"Stock","clauses":[
                {"label":"C1","text":"a unit is reserved"},
                {"label":"C2","text":"a unit is released","replaces":"C7"},
                {"label":"C3","text":"stock is not negative"},
                {"label":"C9","text":"a reservation is not kept for ever","replaces":"C4"}]}),
            view(),
            json!({"op":"check"}),
        ],
    );
    let ids = |v: &Value| -> Vec<Value> { clauses_of(v).iter().map(|c| c["id"].clone()).collect() };
    let revisions = |v: &Value| -> Vec<u64> {
        clauses_of(v)
            .iter()
            .map(|c| c["revision"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(r[1]["revision"], 1, "{:#?}", r[1]);
    assert_eq!(ids(&r[2]), ids(&r[0]));
    assert_eq!(revisions(&r[2]), [1, 1, 1, 1]);
    assert_eq!(clauses_of(&r[2])[1]["label"], "C7");
    assert_eq!(r[3]["revision"], 1, "{:#?}", r[3]);
    assert_eq!(ids(&r[4]), ids(&r[0]));
    assert_eq!(revisions(&r[4]), [1, 1, 1, 1]);
    let labels: Vec<&str> = clauses_of(&r[4])
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["C1", "C2", "C3", "C9"]);
    // no citation went stale, in either put or where the whole is checked
    assert!(!rules(&r[1]).contains(&"cite.stale"), "{:#?}", r[1]);
    assert!(!rules(&r[3]).contains(&"cite.stale"), "{:#?}", r[3]);
    assert!(!rules(&r[5]).contains(&"cite.stale"), "{:#?}", r[5]);
}

#[test]
fn a_relabel_for_a_request_is_its_work_breaks_no_keep_and_is_a_move_of_the_clause_alone() {
    let clause =
        |label: &str| json!({"clause":{"context":"core","contract":"stock","label":label}});
    let (_, r) = after_setup(
        "clauses-put-relabel-request",
        &[
            // 0, 1: a request whose frame is the Clause, and one that keeps the Contract
            json!({"op":"cr_open","title":"Rename C2","intent":"change_behavior","targets":[clause("C2")]}),
            json!({"op":"cr_open","title":"Keep stock","intent":"keep_behavior",
                "keep":[{"contract":{"context":"core","contract":"stock"}}]}),
            // 2: the relabel
            json!({"op":"change_set","for":"Rename C2","puts":[{"put":"clauses","context":"core",
                "contract":"stock","clauses":[{"label":"C7","text":"a unit is released","replaces":"C2"}]}]}),
            // 3: resolved as done
            json!({"op":"cr_resolve","title":"Rename C2","resolution":"done",
                "reason":"C2 is C7 now","by":{"person":"alice"}}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(r[2]["ok"], true, "{:#?}", r[2]);
    assert!(!rules(&r[2]).contains(&"cr.out-of-frame"), "{:#?}", r[2]);
    assert!(!rules(&r[2]).contains(&"cr.keep-moved"), "{:#?}", r[2]);
    // the new name is the request's work: resolved as done, it is not said to be without any, and its target reads
    // as revised
    assert_eq!(r[3]["ok"], true, "{:#?}", r[3]);
    assert!(
        !rules(&r[3]).contains(&"cr.resolved-without-work"),
        "{:#?}",
        r[3]
    );
    assert!(!rules(&r[3]).contains(&"cr.keep-moved"), "{:#?}", r[3]);
    let status = &r[4];
    let request = cr_status(status, "Rename C2");
    assert_eq!(request["targets"][0]["progress"], "revised", "{status:#?}");
    // the work holds the new name, and no revision moved
    let work = request["work"].as_array().expect("work");
    assert_eq!(work.len(), 1, "{status:#?}");
    assert_eq!(work[0]["moved"], json!([]), "{status:#?}");
    assert_eq!(
        work[0]["renamed"],
        json!([{"target":"stock C7","from":"C2","to":"C7"}]),
        "{status:#?}"
    );
    // the keep is as it was
    assert!(
        !rules(status).contains(&"cr.keep-moved"),
        "{:#?}",
        status["findings"]
    );
}

#[test]
fn a_relabel_outside_the_frame_of_a_request_is_out_of_frame_by_the_clause_alone() {
    let (_, r) = after_setup(
        "clauses-put-relabel-outside",
        &[
            json!({"op":"cr_open","title":"Reword C1","intent":"change_behavior",
                "targets":[{"clause":{"context":"core","contract":"stock","label":"C1"}}]}),
            json!({"op":"change_set","for":"Reword C1","puts":[{"put":"clauses","context":"core",
                "contract":"stock","clauses":[{"label":"C7","text":"a unit is released","replaces":"C2"}]}]}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(r[1]["ok"], true, "{:#?}", r[1]);
    let outside: Vec<&Value> = r[1]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"] == "cr.out-of-frame")
        .collect();
    assert_eq!(outside.len(), 1, "{:#?}", r[1]);
    assert!(
        outside[0]["about"]
            .as_str()
            .is_some_and(|a| a.contains("Reword C1")),
        "{:#?}",
        r[1]
    );
    // the Clause alone is outside: the Contract holding it moved nothing
    let request = cr_status(&r[2], "Reword C1");
    assert_eq!(request["out_of_frame"], json!(["stock C7"]), "{:#?}", r[2]);
}

#[test]
fn an_item_given_a_new_key_for_a_request_is_its_work_and_its_target_reads_revised() {
    let item = |key: &str| json!({"item":{"context":"core","interface":"api","key":key}});
    let (_, r) = after_setup(
        "items-put-rekey-request",
        &[
            json!({"op":"cr_open","title":"Rename price","intent":"change_behavior","targets":[item("price")]}),
            json!({"op":"change_set","for":"Rename price","puts":[{"put":"interface","context":"core",
                "name":"api","source":"fn price()\n","items":[{"key":"cost","lines":[1,1],"replaces":"price"}]}]}),
            json!({"op":"cr_resolve","title":"Rename price","resolution":"done",
                "reason":"price is cost now","by":{"person":"alice"}}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(r[1]["ok"], true, "{:#?}", r[1]);
    assert!(!rules(&r[1]).contains(&"cr.out-of-frame"), "{:#?}", r[1]);
    assert_eq!(r[2]["ok"], true, "{:#?}", r[2]);
    assert!(
        !rules(&r[2]).contains(&"cr.resolved-without-work"),
        "{:#?}",
        r[2]
    );
    let request = cr_status(&r[3], "Rename price");
    assert_eq!(request["targets"][0]["progress"], "revised", "{:#?}", r[3]);
    let work = request["work"].as_array().expect("work");
    assert_eq!(work.len(), 1, "{:#?}", r[3]);
    assert_eq!(work[0]["moved"], json!([]), "{:#?}", r[3]);
    assert_eq!(
        work[0]["renamed"],
        json!([{"target":"api cost","from":"price","to":"cost"}]),
        "{:#?}",
        r[3]
    );
}

#[test]
fn a_clauses_entry_for_a_request_moves_the_reworded_clause_and_the_contract_only() {
    let (_, r) = after_setup(
        "clauses-put-footprint",
        &[
            json!({"op":"cr_open","title":"Reword C1","intent":"change_behavior",
                "targets":[{"contract":{"context":"core","contract":"stock"}}]}),
            json!({"op":"change_set","for":"Reword C1","puts":[{"put":"clauses","context":"core",
                "contract":"stock","clauses":[{"label":"C1","text":"a unit is reserved at most once"}]}]}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(r[1]["ok"], true, "{:#?}", r[1]);
    assert_eq!(r[1]["puts"][0]["put"], "clauses");
    assert!(!rules(&r[1]).contains(&"cr.out-of-frame"), "{:#?}", r[1]);
    let work = cr_status(&r[2], "Reword C1")["work"]
        .as_array()
        .expect("work");
    assert_eq!(work.len(), 1, "{:#?}", r[2]);
    let mut moved = strings(&work[0]["moved"]);
    moved.sort_unstable();
    assert_eq!(moved, ["contract stock", "stock C1"]);
}

#[test]
fn a_contract_and_a_clauses_entry_for_one_contract_in_one_set_are_refused() {
    let (_, r) = after_setup(
        "clauses-put-named-twice",
        &[
            json!({"op":"change_set","puts":[
                {"put":"contract","context":"core","name":"stock","title":"Stock","clauses":[
                    {"label":"C1","text":"a unit is reserved"},
                    {"label":"C2","text":"a unit is released"},
                    {"label":"C3","text":"stock is not negative"},
                    {"label":"C4","text":"a reservation is not kept for ever"}]},
                {"put":"clauses","context":"core","contract":"stock",
                 "clauses":[{"label":"C1","text":"x"}]}]}),
            view(),
        ],
    );
    assert_eq!(
        diagnostics(&r[0])[0]["code"],
        "change-set.named-twice",
        "{:#?}",
        r[0]
    );
    assert_eq!(clauses_of(&r[1]).len(), 4);
}

#[test]
fn a_harness_in_the_same_set_cites_a_label_the_clauses_entry_adds_and_one_it_renames() {
    let case = |name: &str, label: &str| {
        json!({"name":name,"lines":[1,1],
            "items":[{"interface":"api","key":"price"}],
            "clauses":[{"contract":"stock","label":label}]})
    };
    let (_, r) = after_setup(
        "clauses-put-same-set",
        &[json!({"op":"change_set","puts":[
            {"put":"clauses","context":"core","contract":"stock","clauses":[
                {"label":"C6","text":"a unit is reserved","replaces":"C1"},
                {"label":"C9","text":"a unit is counted"}]},
            {"put":"harness","context":"core","name":"tests",
             "source":"fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n",
             "cases":[case("a","C6"),case("b","C2"),case("c","C3"),case("d","C9")]}]})],
    );
    assert_eq!(r[0]["ok"], true, "{:#?}", r[0]);
    assert_eq!(r[0]["puts"][0]["put"], "clauses");
    // the added Clause moved the Contract; the new label alone did not move the Clause
    assert_eq!(r[0]["puts"][0]["revision"], 2);
}

#[test]
fn an_unknown_label_of_replaces_or_remove_is_an_unknown_name_with_the_contracts_labels_near_it() {
    let (_, r) = after_setup(
        "clauses-put-unknown",
        &[
            json!({"op":"clauses_put","context":"core","contract":"stock",
                "clauses":[{"label":"C7","text":"x","replaces":"C5"}]}),
            json!({"op":"clauses_put","context":"core","contract":"stock","remove":["C5"]}),
            json!({"op":"contract_put","context":"core","name":"stock","title":"Stock",
                "clauses":[{"label":"C7","text":"x","replaces":"C5"}]}),
            json!({"op":"clauses_put","context":"core","contract":"nope","clauses":[]}),
        ],
    );
    for (k, field) in [(0, "replaces"), (1, "remove"), (2, "replaces")] {
        let d = one(&r[k]);
        assert_eq!(d["code"], "input.unknown-name", "{d:#?}");
        assert_eq!(d["values"]["kind"], "clause");
        assert_eq!(d["values"]["name"], "C5");
        assert_eq!(d["values"]["field"], field);
        assert_eq!(d["subjects"].as_array().unwrap().len(), 1);
        assert_eq!(d["subjects"][0]["kind"], "contract");
        assert_eq!(d["subjects"][0]["name"], "stock");
        let near = strings(&d["did_you_mean"]);
        assert!(!near.is_empty(), "{d:#?}");
        assert!(near.iter().all(|n| ["C1", "C2", "C3", "C4"].contains(n)));
    }
    // a Contract that is not there is the unknown asset
    let d = one(&r[3]);
    assert_eq!(d["code"], "input.unknown-name", "{d:#?}");
    assert_eq!(d["values"]["kind"], "contract");
}

#[test]
fn a_replaces_still_given_and_a_label_named_twice_are_refused_by_their_rules() {
    let (_, r) = after_setup(
        "clauses-put-conflicts",
        &[
            // 0: the label replaced is also given
            json!({"op":"clauses_put","context":"core","contract":"stock","clauses":[
                {"label":"C1","text":"x","replaces":"C2"},{"label":"C2","text":"y"}]}),
            // 1, 2, 3: a label given twice, replaced twice, removed and given
            json!({"op":"clauses_put","context":"core","contract":"stock","clauses":[
                {"label":"C1","text":"x"},{"label":"C1","text":"y"}]}),
            json!({"op":"clauses_put","context":"core","contract":"stock","clauses":[
                {"label":"C5","text":"x","replaces":"C1"},{"label":"C6","text":"y","replaces":"C1"}]}),
            json!({"op":"clauses_put","context":"core","contract":"stock",
                "clauses":[{"label":"C1","text":"x"}],"remove":["C1"]}),
        ],
    );
    let kept = one(&r[0]);
    assert_eq!(kept["code"], "input.replaces-kept", "{kept:#?}");
    assert_eq!(kept["values"]["key"], "C2");
    assert_eq!(kept["values"]["field"], "label");
    let named: Vec<(&str, &str)> = kept["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["kind"].as_str().unwrap(), s["name"].as_str().unwrap()))
        .collect();
    assert_eq!(named, [("context", "core"), ("contract", "stock")]);
    for (k, key, side) in [
        (1, "C1", "label"),
        (2, "C1", "replaces"),
        (3, "C1", "remove"),
    ] {
        let d = one(&r[k]);
        assert_eq!(d["code"], "input.rename-conflict", "{d:#?}");
        assert_eq!(d["values"]["key"], key);
        assert_eq!(d["values"]["side"], side);
        assert_eq!(d["subjects"][1]["kind"], "contract");
        assert_eq!(d["subjects"][1]["name"], "stock");
    }
}

#[test]
fn the_problems_of_one_entry_are_said_together_and_nothing_changes() {
    let (_, r) = after_setup(
        "clauses-put-many",
        &[
            json!({"op":"clauses_put","context":"core","contract":"stock",
                "clauses":[{"label":"C1","text":"x","replaces":"C2"},{"label":"C2","text":"y"},
                           {"label":"C3","text":"z"},{"label":"C3","text":"w"}],
                "remove":["C9"]}),
            view(),
        ],
    );
    let codes: Vec<&str> = diagnostics(&r[0])
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "input.replaces-kept",
            "input.rename-conflict",
            "input.unknown-name"
        ],
        "{:#?}",
        r[0]
    );
    let texts: Vec<&str> = clauses_of(&r[1])
        .iter()
        .map(|c| c["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts[0], "a unit is reserved");
    assert_eq!(texts.len(), 4);
}

#[test]
fn the_schema_of_the_operations_has_clauses_put_and_replaces() {
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .args(["schema", "ops"])
        .output()
        .expect("chidev runs");
    assert!(out.status.success(), "{out:#?}");
    let schema: Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    let text = schema.to_string();
    assert!(text.contains("\"clauses_put\""), "{text}");
    assert!(text.contains("\"replaces\""), "{text}");
    assert!(text.contains("\"remove\""), "{text}");
}
