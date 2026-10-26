//! The Items of an Interface found by a provider, end to end through the `chidev` binary: `interface_put` without
//! `items` starts the command of `[measure.items]` for the language its layout gives, and the answer becomes the
//! Items. The provider here is a fake: a small shell script, written by the test, that answers a fixed JSON (or
//! fails, or is slow) according to a marker in the code it is sent. The Rust provider is not used.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// Answers by what the code says: `BADLINES` has lines past its end, `BOOM` exits with a failure, `SLOW` does not
/// answer in time, `NOTJSON` is not a reply; any other code gets `answer.json`. The request is kept.
const PROVIDER: &str = r#"#!/bin/sh
cat > request.json
if grep -q BADLINES request.json; then
  printf '%s' '{"protocol":1,"items":[{"key":"a","lines":[9,9]}]}'
elif grep -q BOOM request.json; then
  echo 'cannot parse the code' >&2
  exit 3
elif grep -q SLOW request.json; then
  sleep 30
elif grep -q NOTJSON request.json; then
  printf '%s' 'not json'
else
  cat answer.json
fi
"#;

/// A directory of its own under cargo's temporary one, with an empty `chidev.toml` (it is a project's root).
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("chidev.toml"), "").expect("a file");
    dir
}

fn write(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).expect("a file");
}

/// A project whose `[measure]` has a provider of items for the language `fake`, and the Interfaces `api`
/// (`fake`), `bare` (no language) and `other` (a language only `mutants` has a command for) in the Context `core`.
fn project(name: &str, timeout: u64) -> PathBuf {
    let dir = fresh_dir(name);
    write(&dir, "items.sh", PROVIDER);
    std::fs::set_permissions(dir.join("items.sh"), std::fs::Permissions::from_mode(0o755))
        .expect("executable");
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"a","lines":[1,1]}]}"#,
    );
    write(
        &dir,
        "chidev.toml",
        &format!(
            r#"[measure]
build = ["true"]
timeout = {timeout}

[measure.items]
fake = ["./items.sh"]

[measure.mutants]
other = ["true"]

[[measure.layout]]
context = "core"
interface = "api"
lang = "fake"
path = "api.txt"

[[measure.layout]]
context = "core"
interface = "bare"
path = "bare.txt"

[[measure.layout]]
context = "core"
interface = "other"
lang = "other"
path = "other.txt"
"#
        ),
    );
    dir
}

/// `chidev exec` with `dir` as the project's root, on a store inside it: the result of each operation.
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
    assert_eq!(lines.len(), ops.len(), "{lines:#?}\nstderr: {stderr}");
    lines
}

fn setup() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
    ]
}

fn put(name: &str, source: &str) -> Value {
    json!({"op":"interface_put","context":"core","name":name,"source":source})
}

/// The Items of the Interface `name` as the model shows them: (key, lines, id, revision) and the Interface's
/// revision.
fn items_of(dir: &Path, name: &str) -> (u64, Vec<(String, Value, String, u64)>) {
    let view = &exec(dir, &[json!({"op":"check_rules"})])[0];
    let interface = view["model"]["contexts"][0]["interfaces"]
        .as_array()
        .expect("interfaces")
        .iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no interface {name}: {view:#?}"));
    (
        interface["revision"].as_u64().unwrap(),
        interface["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["key"].as_str().unwrap().to_string(),
                    i["lines"].clone(),
                    i["id"].as_str().unwrap().to_string(),
                    i["revision"].as_u64().unwrap(),
                )
            })
            .collect(),
    )
}

/// The one diagnostic of a refused result.
fn one(result: &Value) -> &Value {
    assert_eq!(result["ok"], false, "{result:#?}");
    let all = result["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(all.len(), 1, "{result:#?}");
    &all[0]
}

#[test]
fn an_interface_put_without_items_is_given_those_the_provider_found_and_a_rename_keeps_the_id() {
    let dir = project("items-found", 10);
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"price","lines":[1,3]},{"key":"tax","lines":[4,4]}]}"#,
    );
    let source = "pub fn price(\n    total: u64,\n) -> u64;\nfn tax() -> u64;\n";
    let mut ops = setup();
    ops.push(put("api", source));
    let results = exec(&dir, &ops);
    assert_eq!(results[2]["ok"], true, "{results:#?}");
    assert_eq!(results[2]["revision"], 1);
    // the provider got the protocol's request, in the project's root
    let asked: Value =
        serde_json::from_slice(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    assert_eq!(asked, json!({"protocol":1,"source":source}));
    let (revision, before) = items_of(&dir, "api");
    assert_eq!(revision, 1);
    assert_eq!(
        before
            .iter()
            .map(|i| (i.0.as_str(), i.1.clone()))
            .collect::<Vec<_>>(),
        [("price", json!([1, 3])), ("tax", json!([4, 4]))]
    );

    // the provider now names the first Item by a path; `renames` says it is the same Item
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"pricing::price","lines":[1,3]},{"key":"tax","lines":[4,4]}]}"#,
    );
    let mut again = put("api", source);
    again["renames"] = json!([{"from":"price","to":"pricing::price"}]);
    let results = exec(&dir, &[again]);
    assert_eq!(results[0]["ok"], true, "{results:#?}");
    assert_eq!(results[0]["revision"], 1, "a rename moves no revision");
    let (revision, after) = items_of(&dir, "api");
    assert_eq!(revision, 1);
    assert_eq!(after[0].0, "pricing::price");
    assert_eq!((&after[0].2, after[0].3), (&before[0].2, before[0].3));
    assert_eq!(after[1], before[1]);

    // without it, the path is a key the Interface did not have: a new Item with a new id
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"pricing::price","lines":[1,3]},{"key":"pricing::tax","lines":[4,4]}]}"#,
    );
    let results = exec(&dir, &[put("api", source)]);
    assert_eq!(results[0]["ok"], true, "{results:#?}");
    let (_, last) = items_of(&dir, "api");
    assert_eq!(last[0], after[0]);
    assert_ne!(last[1].2, after[1].2);
}

#[test]
fn items_that_are_written_start_no_provider() {
    let dir = project("items-written", 10);
    let mut ops = setup();
    let mut with_items = put("api", "fn a();\n// BOOM would fail the provider\n");
    with_items["items"] = json!([{"key":"written","lines":[1,1]}]);
    ops.push(with_items);
    let results = exec(&dir, &ops);
    assert_eq!(results[2]["ok"], true, "{results:#?}");
    assert!(!dir.join("request.json").exists(), "no provider was asked");
    // and the same in a change set
    let results = exec(
        &dir,
        &[json!({"op":"change_set","puts":[
            {"put":"interface","context":"core","name":"api",
             "source":"fn a();\n// BOOM\n","items":[{"key":"written","lines":[1,1]}]}]})],
    );
    assert_eq!(results[0]["ok"], true, "{results:#?}");
    assert!(!dir.join("request.json").exists());
}

#[test]
fn a_change_set_finds_the_items_of_its_interfaces_too_before_it_plans() {
    let dir = project("items-change-set", 10);
    let mut ops = setup();
    ops.push(json!({"op":"change_set","puts":[
        {"put":"interface","context":"core","name":"api","source":"fn a();\n"}]}));
    let results = exec(&dir, &ops);
    assert_eq!(results[2]["ok"], true, "{results:#?}");
    assert_eq!(items_of(&dir, "api").1[0].0, "a");
    // one that cannot be found refuses the whole set, and the other put is not kept
    let results = exec(
        &dir,
        &[json!({"op":"change_set","puts":[
            {"put":"interface","context":"core","name":"api","source":"fn b();\n","items":[{"key":"b","lines":[1,1]}]},
            {"put":"interface","context":"core","name":"other","source":"fn c();\n"}]})],
    );
    assert_eq!(one(&results[0])["code"], "items.no-provider");
    assert_eq!(
        items_of(&dir, "api").1[0].0,
        "a",
        "the change set kept nothing"
    );
}

#[test]
fn every_way_the_items_cannot_be_found_is_a_rule_and_leaves_the_model_as_it_was() {
    let dir = project("items-rules", 2);
    let mut ops = setup();
    ops.push(put("api", "fn a();\n"));
    let results = exec(&dir, &ops);
    assert_eq!(results[2]["ok"], true, "{results:#?}");
    let (revision, before) = items_of(&dir, "api");

    let results = exec(
        &dir,
        &[
            // a layout with no language
            put("bare", "fn a();\n"),
            // an Interface that is not laid out at all
            put("nowhere", "fn a();\n"),
            // a language that only `mutants` has a command for
            put("other", "fn a();\n"),
            // the provider disagrees with the code, is not a reply, fails, or does not answer
            put("api", "fn a();\n// BADLINES\n"),
            put("api", "fn a();\n// NOTJSON\n"),
            put("api", "fn a();\n// BOOM\n"),
            put("api", "fn a();\n// SLOW\n"),
        ],
    );
    let d = one(&results[0]);
    assert_eq!(d["code"], "input.items-required");
    assert_eq!(d["subjects"][1]["name"], "bare");
    assert_eq!(one(&results[1])["code"], "input.items-required");
    let d = one(&results[2]);
    assert_eq!(d["code"], "items.no-provider");
    assert_eq!(d["values"]["lang"], "other");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("`[measure.items]` has no command for `other`"),
        "{d:#?}"
    );
    let d = one(&results[3]);
    assert_eq!(d["code"], "items.protocol");
    assert_eq!(d["values"]["lang"], "fake");
    assert!(
        d["values"]["detail"]
            .as_str()
            .unwrap()
            .contains("past the last")
    );
    let d = one(&results[4]);
    assert_eq!(d["code"], "items.protocol");
    assert!(
        d["values"]["detail"]
            .as_str()
            .unwrap()
            .contains("not a chidev-items reply")
    );
    let d = one(&results[5]);
    assert_eq!(d["code"], "items.failed");
    assert!(
        d["values"]["detail"]
            .as_str()
            .unwrap()
            .contains("cannot parse the code")
    );
    let d = one(&results[6]);
    assert_eq!(d["code"], "items.failed");
    assert!(d["values"]["detail"].as_str().unwrap().contains("in time"));

    // nothing was kept: the model is as it was
    assert_eq!(items_of(&dir, "api"), (revision, before));
}

#[test]
fn a_project_without_a_measure_table_cannot_leave_the_items_out() {
    let dir = fresh_dir("items-no-measure");
    let mut ops = setup();
    ops.push(put("api", "fn a();\n"));
    let results = exec(&dir, &ops);
    let d = one(&results[2]);
    assert_eq!(d["code"], "items.no-provider");
    assert!(d["values"]["lang"].is_null(), "{d:#?}");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("no `[measure]` table")
    );
    // a chidev.toml with other tables is the same
    write(&dir, "chidev.toml", "[checks]\n");
    let results = exec(&dir, &[put("api", "fn a();\n")]);
    assert_eq!(one(&results[0])["code"], "items.no-provider");
}

#[test]
fn the_renames_that_cannot_be_carried_out_are_refused_by_name() {
    let dir = project("items-renames", 10);
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"x","lines":[1,1]},{"key":"y","lines":[2,2]}]}"#,
    );
    let mut ops = setup();
    ops.push(put("api", "fn x();\nfn y();\n"));
    assert_eq!(exec(&dir, &ops)[2]["ok"], true);
    let (revision, before) = items_of(&dir, "api");

    let with = |renames: Value| {
        let mut op = put("api", "fn x();\nfn y();\n");
        op["renames"] = renames;
        op
    };
    let results = exec(
        &dir,
        &[
            with(json!([{"from":"gone","to":"x"}])),
            with(json!([{"from":"x","to":"nothing"}])),
            with(json!([{"from":"x","to":"x"},{"from":"x","to":"y"}])),
            with(json!([{"from":"x","to":"x"},{"from":"y","to":"x"}])),
            with(json!([{"from":"y","to":"x"}])),
            {
                let mut op = with(json!([{"from":"x","to":"y"}]));
                op["items"] = json!([{"key":"x","lines":[1,1]}]);
                op
            },
        ],
    );
    let d = one(&results[0]);
    assert_eq!(d["code"], "input.rename-unknown");
    assert_eq!(
        (&d["values"]["key"], &d["values"]["field"]),
        (&json!("gone"), &json!("from"))
    );
    let d = one(&results[1]);
    assert_eq!(d["code"], "input.rename-unknown");
    assert_eq!(
        (&d["values"]["key"], &d["values"]["field"]),
        (&json!("nothing"), &json!("to"))
    );
    let d = one(&results[2]);
    assert_eq!(d["code"], "input.rename-conflict");
    assert_eq!(
        (&d["values"]["key"], &d["values"]["side"]),
        (&json!("x"), &json!("from"))
    );
    let d = one(&results[3]);
    assert_eq!(d["code"], "input.rename-conflict");
    assert_eq!(
        (&d["values"]["key"], &d["values"]["side"]),
        (&json!("x"), &json!("to"))
    );
    let d = one(&results[4]);
    assert_eq!(d["code"], "input.replaces-kept");
    assert_eq!(d["values"]["key"], "y");
    assert_eq!(one(&results[5])["code"], "input.renames-with-items");
    assert_eq!(items_of(&dir, "api"), (revision, before));
}

#[test]
fn the_examples_of_the_rules_of_renames_are_refused_by_those_rules() {
    let dir = project("items-examples", 10);
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"a","lines":[1,1]}]}"#,
    );
    let ids = ["input.rename-unknown", "input.renames-with-items"];
    let ops: Vec<Value> = ids
        .iter()
        .map(|id| json!({"op":"explain","rule":id}))
        .collect();
    let examples: Vec<Value> = exec(&dir, &ops)
        .iter()
        .map(|e| serde_json::from_str(e["example"].as_str().unwrap()).expect("an operation"))
        .collect();
    let mut all = setup();
    all.extend(examples);
    let results = exec(&dir, &all);
    for (id, result) in ids.iter().zip(&results[2..]) {
        assert_eq!(one(result)["code"], *id, "{result:#?}");
    }
    // the example of the rule for a project with no provider is refused by it where there is no `[measure]`
    let bare = fresh_dir("items-examples-bare");
    let ops = [json!({"op":"explain","rule":"items.no-provider"})];
    let example: Value =
        serde_json::from_str(exec(&bare, &ops)[0]["example"].as_str().unwrap()).unwrap();
    let mut all = setup();
    all.push(example);
    assert_eq!(one(&exec(&bare, &all)[2])["code"], "items.no-provider");
}

fn put_items(name: &str, source: &str, items: Value) -> Value {
    json!({"op":"interface_put","context":"core","name":name,"source":source,"items":items})
}

fn rekey(name: &str) -> Value {
    json!({"op":"items_rekey","context":"core","interface":name})
}

#[test]
fn items_rekey_gives_the_provider_keys_and_keeps_the_ids_and_the_revisions() {
    let dir = project("rekey-keys", 10);
    let source = "pub fn price(total: u64) -> u64;\nfn tax() -> u64;\n";
    let mut ops = setup();
    ops.push(put_items(
        "api",
        source,
        json!([
            {"key":"pub fn price(total: u64) -> u64","lines":[1,1]},
            {"key":"fn tax() -> u64","lines":[2,2]},
        ]),
    ));
    assert_eq!(exec(&dir, &ops)[2]["ok"], true);
    let (revision, before) = items_of(&dir, "api");

    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"price","lines":[1,1]},{"key":"tax","lines":[2,2]}]}"#,
    );
    let results = exec(&dir, &[rekey("api")]);
    let r = &results[0];
    assert_eq!(r["ok"], true, "{r:#?}");
    assert_eq!(r["op"], "items_rekey");
    assert_eq!(
        r["rekeyed"],
        json!([
            {"from":"pub fn price(total: u64) -> u64","to":"price"},
            {"from":"fn tax() -> u64","to":"tax"},
        ])
    );
    assert_eq!(r["revised"], json!([]));
    assert_eq!(r["unmatched"], json!([]));
    assert_eq!(r["revision"], revision);
    // a rekey is an edit, and says what its checks said
    assert_eq!(r["findings"], json!([]));
    // the provider was sent the code the Interface keeps
    let asked: Value =
        serde_json::from_slice(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
    assert_eq!(asked, json!({"protocol":1,"source":source}));
    let (now, after) = items_of(&dir, "api");
    assert_eq!(now, revision);
    assert_eq!(
        after
            .iter()
            .map(|i| (i.0.as_str(), i.2.as_str(), i.3))
            .collect::<Vec<_>>(),
        [
            ("price", before[0].2.as_str(), 1),
            ("tax", before[1].2.as_str(), 1)
        ]
    );

    // again: nothing is left to change
    let again = &exec(&dir, &[rekey("api")])[0];
    assert_eq!(again["rekeyed"], json!([]));
    assert_eq!(items_of(&dir, "api"), (now, after));
}

#[test]
fn items_rekey_revises_an_item_whose_new_lines_declare_something_else_and_lists_it() {
    let dir = project("rekey-revised", 10);
    let source = "fn price(\n    total: u64,\n) -> u64;\nfn tax() -> u64;\n";
    let mut ops = setup();
    ops.push(put_items(
        "api",
        source,
        json!([
            {"key":"price","lines":[1,1]},
            {"key":"tax","lines":[4,4]},
        ]),
    ));
    assert_eq!(exec(&dir, &ops)[2]["ok"], true);
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"price","lines":[1,3]},{"key":"tax","lines":[4,4]}]}"#,
    );
    let r = &exec(&dir, &[rekey("api")])[0];
    assert_eq!(r["ok"], true, "{r:#?}");
    assert_eq!(r["rekeyed"], json!([]));
    assert_eq!(r["revised"], json!([{"key":"price","from":1,"to":2}]));
    assert_eq!(r["revision"], 2);
    let (revision, items) = items_of(&dir, "api");
    assert_eq!(revision, 2);
    assert_eq!(
        items.iter().map(|i| (i.1.clone(), i.3)).collect::<Vec<_>>(),
        [(json!([1, 3]), 2), (json!([4, 4]), 1)]
    );
}

#[test]
fn items_rekey_leaves_an_item_with_no_single_declaration_as_it_is_and_lists_it() {
    let dir = project("rekey-many", 10);
    let mut ops = setup();
    // one Item over the lines of two declarations
    ops.push(put_items(
        "api",
        "fn a();\nfn b();\nfn c();\n",
        json!([{"key":"a and b","lines":[1,2]}, {"key":"c","lines":[3,3]}]),
    ));
    assert_eq!(exec(&dir, &ops)[2]["ok"], true);
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"a","lines":[1,1]},{"key":"b","lines":[2,2]},{"key":"C","lines":[3,3]}]}"#,
    );
    let (_, before) = items_of(&dir, "api");
    let r = &exec(&dir, &[rekey("api")])[0];
    assert_eq!(r["ok"], true, "{r:#?}");
    assert_eq!(r["rekeyed"], json!([{"from":"c","to":"C"}]));
    assert_eq!(r["unmatched"], json!(["a and b"]));
    let (_, after) = items_of(&dir, "api");
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1].0, "C");
}

#[test]
fn items_rekey_that_would_give_one_key_twice_or_has_no_provider_changes_nothing() {
    let dir = project("rekey-refused", 2);
    let mut ops = setup();
    ops.push(put_items(
        "api",
        "fn a();\nfn b();\n",
        json!([{"key":"a","lines":[1,1]},{"key":"b","lines":[2,2]}]),
    ));
    ops.push(put_items(
        "bare",
        "fn a();\n",
        json!([{"key":"a","lines":[1,1]}]),
    ));
    ops.push(put_items(
        "other",
        "fn a();\n",
        json!([{"key":"a","lines":[1,1]}]),
    ));
    ops.push(put_items(
        "nowhere",
        "fn a();\n",
        json!([{"key":"a","lines":[1,1]}]),
    ));
    let results = exec(&dir, &ops);
    assert!(results[2..].iter().all(|r| r["ok"] == true), "{results:#?}");
    let (revision, before) = items_of(&dir, "api");
    write(
        &dir,
        "answer.json",
        r#"{"protocol":1,"items":[{"key":"x","lines":[1,1]},{"key":"x","lines":[2,2]}]}"#,
    );
    let results = exec(
        &dir,
        &[
            rekey("api"),
            rekey("bare"),
            rekey("other"),
            rekey("nowhere"),
            rekey("missing"),
        ],
    );
    let d = one(&results[0]);
    assert_eq!(d["code"], "interface.key-taken");
    assert_eq!(items_of(&dir, "api"), (revision, before));
    assert_eq!(one(&results[1])["code"], "input.items-required");
    assert_eq!(one(&results[2])["code"], "items.no-provider");
    assert_eq!(one(&results[3])["code"], "input.items-required");
    // an Interface the Context does not have
    one(&results[4]);

    // a provider that fails on the code the Interface keeps
    let results = exec(
        &dir,
        &[
            put_items(
                "api",
                "fn a();\n// BOOM\n",
                json!([{"key":"a","lines":[1,1]}]),
            ),
            rekey("api"),
        ],
    );
    assert_eq!(results[0]["ok"], true, "{results:#?}");
    let d = one(&results[1]);
    assert_eq!(d["code"], "items.failed");
    assert_eq!(d["values"]["lang"], "fake");
    assert_eq!(items_of(&dir, "api").1[0].0, "a");
}

/// A project whose Interfaces in the Context `core` are laid out as Rust, with the Rust provider of this very
/// binary (`chidev provider items-rust`) as the provider of items.
fn rust_project(name: &str) -> PathBuf {
    let dir = fresh_dir(name);
    write(
        &dir,
        "chidev.toml",
        &format!(
            r#"[measure]
build = ["true"]
timeout = 20

[measure.items]
rust = ['{bin}', "provider", "items-rust"]

[[measure.layout]]
context = "core"
interface = "api"
lang = "rust"
path = "api.rs"
"#,
            bin = env!("CARGO_BIN_EXE_chidev")
        ),
    );
    dir
}

const API: &str = "\
/// The price of a basket.
pub fn price(total: u64) -> u64;
pub trait Clock {
    fn now(&self) -> u64;
}
impl<T> Stock<T> {
    pub fn new() -> Stock<T> {
        Stock::default()
    }
}
";

#[test]
fn the_rust_provider_finds_the_items_of_an_interface_put_without_them_end_to_end() {
    let dir = rust_project("rust-found");
    let mut ops = setup();
    ops.push(put("api", API));
    let results = exec(&dir, &ops);
    assert_eq!(results[2]["ok"], true, "{results:#?}");
    let (_, items) = items_of(&dir, "api");
    assert_eq!(
        items
            .iter()
            .map(|i| (i.0.as_str(), i.1.clone()))
            .collect::<Vec<_>>(),
        [
            ("price", json!([2, 2])),
            ("Clock", json!([3, 3])),
            ("Clock::now", json!([4, 4])),
            ("Stock<T>::new", json!([7, 7])),
        ]
    );
}

/// `chidev provider items-rust` with `input` on its standard input: its exit code, standard output and error.
fn items_rust(input: &str) -> (Option<i32>, String, String) {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .args(["provider", "items-rust"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("chidev runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("the request is written");
    let out = child.wait_with_output().expect("chidev ends");
    (
        out.status.code(),
        String::from_utf8(out.stdout).expect("utf-8"),
        String::from_utf8(out.stderr).expect("utf-8"),
    )
}

#[test]
fn the_items_rust_provider_answers_one_reply_on_standard_output_and_a_failure_with_an_exit_code() {
    let (code, out, _) = items_rust(r#"{"protocol":1,"source":"fn price(total: u64) -> u64;\n"}"#);
    assert_eq!(code, Some(0));
    assert_eq!(
        out,
        r#"{"protocol":1,"items":[{"key":"price","lines":[1,1]}]}"#
    );
    for (input, said) in [
        ("not json", "not a chidev-items request"),
        (r#"{"protocol":2,"source":""}"#, "protocol 2"),
        (r#"{"protocol":1,"source":"fn ("}"#, "does not parse"),
    ] {
        let (code, out, err) = items_rust(input);
        assert_eq!(code, Some(2), "{input}");
        assert_eq!(out, "", "no half reply: {input}");
        assert!(
            err.contains("chidev provider items-rust:") && err.contains(said),
            "{err}"
        );
    }
}

#[test]
fn items_rekey_with_the_rust_provider_replaces_the_keys_written_by_hand() {
    let dir = rust_project("rust-rekey");
    let mut ops = setup();
    // the Items as someone wrote them before a provider found them: the signature as the key
    ops.push(put_items(
        "api",
        API,
        json!([
            {"key":"pub fn price(total: u64) -> u64","lines":[2,2]},
            {"key":"pub trait Clock","lines":[3,5]},
            {"key":"pub fn new() -> Stock<T>","lines":[7,7]},
        ]),
    ));
    assert_eq!(exec(&dir, &ops)[2]["ok"], true);
    let (revision, before) = items_of(&dir, "api");
    let r = &exec(&dir, &[rekey("api")])[0];
    assert_eq!(r["ok"], true, "{r:#?}");
    assert_eq!(
        r["rekeyed"],
        json!([
            {"from":"pub fn price(total: u64) -> u64","to":"price"},
            {"from":"pub fn new() -> Stock<T>","to":"Stock<T>::new"},
        ])
    );
    // `trait Clock` was written over the lines of two declarations (the trait and `now`): it is not paired
    assert_eq!(r["unmatched"], json!(["pub trait Clock"]));
    // the lines of `new` were the declaration's already
    assert_eq!(r["revised"], json!([]));
    let (now, after) = items_of(&dir, "api");
    assert_eq!(now, revision);
    assert_eq!(
        after.iter().map(|i| i.0.as_str()).collect::<Vec<_>>(),
        ["price", "pub trait Clock", "Stock<T>::new"]
    );
    assert_eq!(
        after.iter().map(|i| i.2.as_str()).collect::<Vec<_>>(),
        before.iter().map(|i| i.2.as_str()).collect::<Vec<_>>(),
        "the ids are the ones the Items had"
    );
}
