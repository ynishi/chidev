//! The copies of the placed assets in the working tree, end to end through the binary: a pinned file that is changed
//! on disk is found by `chidev check` with nothing put again, the question `check_run` keeps the same finding, and
//! `status` lists what moved and what no layout places. No program is run: the settings only lay the files out.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

/// The code as the Pin accepts it, and the source of the Harness.
const CODE: &str = "pub fn push(x: u8) -> u8 { x }\n";
const SOURCE: &str = "fn pushes() {}\n";

/// A project directory of its own under cargo's temporary one, with the settings that lay out the Interface `api` in
/// `code.txt` and the Harness `tests` in `tests.txt`, and the files as the model has them.
fn project(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(
        dir.join("chidev.toml"),
        r#"[measure]
build = ["sh", "-c", "true"]
env = {}

[[measure.layout]]
context = "core"
interface = "api"
path = "code.txt"

[[measure.layout]]
context = "core"
harness = "tests"
path = "tests.txt"
"#,
    )
    .expect("chidev.toml");
    std::fs::write(dir.join("code.txt"), CODE).expect("code.txt");
    std::fs::write(dir.join("tests.txt"), SOURCE).expect("tests.txt");
    dir
}

/// `chidev` with `args` in `dir`.
fn chidev(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("chidev runs")
}

/// `chidev exec` of `ops` in `dir`, every one of which succeeded: the result line of each.
fn exec(dir: &Path, ops: &[Value]) -> Vec<Value> {
    let file = dir.join("ops.jsonl");
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(&file, text).expect("a file");
    let out = chidev(dir, &["exec", file.to_str().expect("utf-8")]);
    let lines: Vec<Value> = String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON object per line"))
        .collect();
    assert_eq!(lines.len(), ops.len(), "{lines:#?}");
    assert!(lines.iter().all(|r| r["ok"] == true), "{lines:#?}");
    lines
}

/// What `chidev check tree.` printed, and its exit code.
fn check_tree(dir: &Path) -> (Value, i32) {
    let out = chidev(dir, &["check", "tree."]);
    let text = String::from_utf8(out.stdout.clone()).expect("utf-8");
    (
        serde_json::from_str(&text).expect("a JSON object"),
        out.status.code().expect("an exit code"),
    )
}

/// The findings of a result as `(rule, level, about)`.
fn found(result: &Value) -> Vec<(String, String, String)> {
    result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["rule"].as_str().expect("rule").starts_with("tree."))
        .map(|f| {
            (
                f["rule"].as_str().expect("rule").to_string(),
                f["level"].as_str().expect("level").to_string(),
                f["about"].as_str().expect("about").to_string(),
            )
        })
        .collect()
}

/// The model: a Contract, the Interfaces `api` (pinned) and `lib` (laid out nowhere), and the Harness `tests`.
fn model() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"contract_put","context":"core","name":"stack","title":"Stack","clauses":[
            {"label":"C1","text":"push then pop"}]}),
        json!({"op":"interface_put","context":"core","name":"api","source":"fn push();\n",
               "items":[{"key":"push","lines":[1,1]}]}),
        json!({"op":"interface_put","context":"core","name":"lib","source":"fn other();\n",
               "items":[{"key":"other","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"tests","source":SOURCE,"cases":[
            {"name":"pushes","lines":[1,1],"items":[{"interface":"api","key":"push"}],
             "clauses":[{"contract":"stack","label":"C1"}]}]}),
        json!({"op":"pin","context":"core","interface":"api","code":CODE}),
    ]
}

#[test]
fn a_pinned_file_changed_on_disk_is_found_by_check_with_nothing_put_again() {
    let dir = project("tree-e2e-moved");
    exec(&dir, &model());

    // the files are what the model holds: nothing is found
    let (same, code) = check_tree(&dir);
    assert_eq!(found(&same), [], "{same:#?}");
    assert_eq!(code, 0);
    let status = &exec(&dir, &[json!({"op":"status"})])[0]["tree"];
    assert_eq!(status["moved"], json!([]));
    assert_eq!(status["unpinned"], json!([]));
    // `lib` is laid out nowhere, and is only counted
    assert_eq!(
        status["unplaced"],
        json!({"interfaces":[{"context":"core","name":"lib"}],"harnesses":[]})
    );

    // the pinned file is changed on disk (a signature), and nothing is put again
    std::fs::write(
        dir.join("code.txt"),
        "pub fn push(x: u8, y: u8) -> u8 { x }\n",
    )
    .expect("code.txt");
    let (moved, code) = check_tree(&dir);
    assert_eq!(
        found(&moved),
        [(
            "tree.code-moved".to_string(),
            "warning".to_string(),
            "interface api".to_string()
        )],
        "{moved:#?}"
    );
    // a warning refuses nothing
    assert_eq!(code, 0);

    // `status` lists what moved, and says the Items of the copy are not looked at
    let results = exec(
        &dir,
        &[
            json!({"op":"status"}),
            json!({"op":"check_run"}),
            json!({"op":"status"}),
        ],
    );
    assert_eq!(
        results[0]["tree"]["moved"],
        json!([{"context":"core","interface":"api","declared":"off"}])
    );
    // the question sees the same copies and keeps the same finding
    assert_eq!(found(&results[1]), found(&moved), "{:#?}", results[1]);
    assert_eq!(found(&results[2]), found(&moved));

    // the same bytes again are the same
    std::fs::write(dir.join("code.txt"), CODE).expect("code.txt");
    assert_eq!(found(&check_tree(&dir).0), []);
}

#[test]
fn a_changed_harness_file_and_a_file_that_is_gone_are_found_each_by_its_rule() {
    let dir = project("tree-e2e-harness");
    exec(&dir, &model());
    std::fs::write(dir.join("tests.txt"), "fn pushes() { assert!(true) }\n").expect("tests.txt");
    std::fs::remove_file(dir.join("code.txt")).expect("remove code.txt");
    let (result, code) = check_tree(&dir);
    assert_eq!(
        found(&result),
        [
            (
                "tree.harness-moved".to_string(),
                "warning".to_string(),
                "case pushes".to_string()
            ),
            (
                "tree.unread".to_string(),
                "warning".to_string(),
                "interface api".to_string()
            ),
        ],
        "{result:#?}"
    );
    assert_eq!(code, 0);
    let message = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|f| f["rule"] == "tree.unread")
        .expect("unread")["message"]
        .as_str()
        .expect("message")
        .to_string();
    assert!(message.ends_with("is missing"), "{message}");
}

#[test]
fn a_project_with_no_measure_table_places_nothing_and_finds_nothing() {
    let dir = project("tree-e2e-none");
    std::fs::write(dir.join("chidev.toml"), "").expect("chidev.toml");
    exec(&dir, &model());
    std::fs::write(dir.join("code.txt"), "something else\n").expect("code.txt");
    let (result, _) = check_tree(&dir);
    assert_eq!(found(&result), []);
    let status = &exec(&dir, &[json!({"op":"status"})])[0]["tree"];
    assert_eq!(status["moved"], json!([]));
    assert_eq!(
        status["unplaced"]["interfaces"]
            .as_array()
            .expect("list")
            .len(),
        2
    );
    assert_eq!(
        status["unplaced"]["harnesses"]
            .as_array()
            .expect("list")
            .len(),
        1
    );
}
