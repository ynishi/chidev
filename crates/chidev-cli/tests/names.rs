//! The name of an asset (a Contract, an Interface, a Harness) and the name of a Context follow one rule: not empty, and
//! only `a-z`, `0-9`, `-` and `_`. A name that breaks it is refused where it is read, by a put and by a reference, and
//! the message names the kind of name and the first character not allowed. Nothing is kept for the refused line.

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

/// Runs `ops` through `chidev exec` in a fresh project; the result of each, in order.
fn exec(name: &str, ops: &[Value]) -> Vec<Value> {
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

/// The message of the one diagnostic of a refused line.
fn refusal(result: &Value) -> String {
    assert_eq!(result["ok"], false, "{result:#?}");
    let diagnostics = result["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(diagnostics.len(), 1, "{result:#?}");
    diagnostics[0]["message"]
        .as_str()
        .expect("a message")
        .to_string()
}

/// A project with a Context `core`, and nothing in it.
fn project() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
    ]
}

#[test]
fn a_put_of_an_asset_with_a_name_that_breaks_the_rule_is_refused_naming_the_character() {
    let mut ops = project();
    ops.extend([
        json!({"op":"contract_put","context":"core","name":"Stock","title":"Stock","clauses":[
            {"label":"C1","text":"a unit is reserved"}]}),
        json!({"op":"interface_put","context":"core","name":"price list","source":"fn price()\n",
            "items":[{"key":"price","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"tests.rs","source":"fn a() {}\n",
            "cases":[{"name":"a","lines":[1,1],"items":[{"interface":"api","key":"price"}]}]}),
        json!({"op":"clauses_put","context":"core","contract":"a#b",
            "clauses":[{"label":"C1","text":"x"}]}),
        json!({"op":"contract_retire","context":"core","contract":"a/b"}),
        json!({"op":"contract_put","context":"core","name":"","title":"Stock","clauses":[
            {"label":"C1","text":"a unit is reserved"}]}),
        json!({"op":"view","sections":["contracts","interfaces","harnesses"]}),
    ]);
    let results = exec("names-put", &ops);
    assert!(results[..2].iter().all(|r| r["ok"] == true), "{results:#?}");
    let said: Vec<String> = results[2..8].iter().map(refusal).collect();
    for (message, ch) in said.iter().zip(['S', ' ', '.', '#', '/']) {
        assert!(message.contains("an asset name"), "{message}");
        assert!(message.contains(&format!("`{ch}`")), "{message}");
    }
    assert!(said[5].contains("an asset name is empty"), "{}", said[5]);
    // nothing was made
    let view = &results[8]["view"]["contexts"][0];
    for section in ["contracts", "interfaces", "harnesses"] {
        assert_eq!(view[section], json!([]), "{section}: {view:#?}");
    }
}

#[test]
fn an_entry_of_a_change_set_with_a_name_that_breaks_the_rule_is_refused_and_the_set_is_not_applied()
{
    let mut ops = project();
    ops.extend([
        json!({"op":"change_set","puts":[
            {"put":"contract","context":"core","name":"stock","title":"Stock",
             "clauses":[{"label":"C1","text":"x"}]},
            {"put":"interface","context":"core","name":"Api","source":"fn price()\n",
             "items":[{"key":"price","lines":[1,1]}]}]}),
        json!({"op":"change_set","puts":[
            {"put":"harness","context":"core","name":"tests","builds":["a b"],"source":"fn a() {}\n",
             "cases":[]}]}),
        json!({"op":"change_set","retires":[{"retire":"harness","context":"core","harness":"x:y"}]}),
        json!({"op":"view","sections":["contracts"]}),
    ]);
    let results = exec("names-change-set", &ops);
    assert!(results[..2].iter().all(|r| r["ok"] == true), "{results:#?}");
    for (result, ch) in results[2..5].iter().zip(['A', ' ', ':']) {
        let message = refusal(result);
        assert!(message.contains("an asset name"), "{message}");
        assert!(message.contains(&format!("`{ch}`")), "{message}");
    }
    assert_eq!(
        results[5]["view"]["contexts"][0]["contracts"],
        json!([]),
        "{:#?}",
        results[5]
    );
}

#[test]
fn a_reference_with_a_name_that_breaks_the_rule_is_refused_naming_the_character() {
    let mut ops = project();
    ops.extend([
        json!({"op":"cr_open","title":"T","intent":"change_behavior",
            "targets":[{"contract":{"context":"core","contract":"Stock"}}]}),
        json!({"op":"cr_open","title":"T","intent":"change_behavior",
            "targets":[{"context":"Core"}]}),
        json!({"op":"cr_open","title":"T","intent":"change_behavior",
            "keep":[{"clause":{"context":"core","contract":"stock","label":"C1"}},
                    {"item":{"interface":"a.b","key":"k"}}]}),
        json!({"op":"cr_open","title":"T","intent":"change_behavior",
            "addresses":[{"rule":"cite.uncited-clause",
                "subject":{"interface":{"context":"core","interface":"a b"}}}]}),
        json!({"op":"harness_put","context":"core","name":"tests","source":"fn a() {}\n",
            "cases":[{"name":"a","lines":[1,1],"items":[{"interface":"Api","key":"price"}]}]}),
    ]);
    let results = exec("names-reference", &ops);
    assert!(results[..2].iter().all(|r| r["ok"] == true), "{results:#?}");
    let said: Vec<String> = results[2..].iter().map(refusal).collect();
    for (message, (kind, ch)) in said.iter().zip([
        ("an asset name", 'S'),
        ("a context name", 'C'),
        ("an asset name", '.'),
        ("an asset name", ' '),
        ("an asset name", 'A'),
    ]) {
        assert!(message.contains(kind), "{message}");
        assert!(message.contains(&format!("`{ch}`")), "{message}");
    }
}
