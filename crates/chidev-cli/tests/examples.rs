//! A Harness of kind `examples`, end to end through the `chidev` binary: the examples of a directory (or each Case's
//! own `example`) are read when the operation runs and kept in the model, so that a change of one text is a new
//! revision of its Case and of the Harness; the view shows them; and a put that is not the shape of examples is
//! refused with every problem named.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// A directory of its own under cargo's temporary one, with an empty `chidev.toml` (it is a project's root) and a
/// directory `acceptance` of two whole examples.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("acceptance")).expect("a temporary directory");
    std::fs::write(dir.join("chidev.toml"), "").expect("a file");
    write_example(&dir, "a1", "", "+ a\n", "1\n#exit 0\n");
    write_example(&dir, "a2", "-n 2", "+ a\n+ b\n", "2\n#exit 0\n");
    dir
}

fn write_example(dir: &Path, name: &str, args: &str, stdin: &str, expected: &str) {
    for (ext, text) in [("args", args), ("stdin", stdin), ("expected", expected)] {
        std::fs::write(dir.join("acceptance").join(format!("{name}.{ext}")), text).expect("a file");
    }
}

/// `chidev exec` with `dir` as the project's root, on a store inside it: the result line of each operation, and
/// what the program wrote to its standard error.
fn run_ops(dir: &Path, ops: &[Value]) -> Vec<Value> {
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
    assert!(
        lines.len() == ops.len(),
        "every operation gave a result: {lines:#?}\nstderr: {stderr}"
    );
    lines
}

/// As [`run_ops`], and every operation succeeded.
fn exec(dir: &Path, ops: &[Value]) -> Vec<Value> {
    let lines = run_ops(dir, ops);
    assert!(lines.iter().all(|r| r["ok"] == true), "{lines:#?}");
    lines
}

/// The ops that make the Context `core` with a Contract of two Clauses and the Interface the program is built from.
fn model() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"contract_put","context":"core","name":"tally","title":"Tally",
               "clauses":[{"label":"C1","text":"a line adds one to the count"},
                          {"label":"C2","text":"-n gives the count of the first n lines"}]}),
        json!({"op":"interface_put","context":"core","name":"api","source":"tally\n",
               "items":[{"key":"tally","lines":[1,1]}]}),
    ]
}

fn clause(label: &str) -> Value {
    json!({"contract":"tally","label":label})
}

/// A Harness of examples that reads `acceptance`, one Case for each example.
fn from_dir() -> Value {
    json!({"op":"harness_put","context":"core","name":"acceptance","kind":"examples","builds":["api"],
           "examples_dir":"acceptance",
           "cases":[{"name":"a1","clauses":[clause("C1")]},{"name":"a2","clauses":[clause("C2")]}]})
}

/// The same Harness, each Case holding its example.
fn inline(a2_expected: &str) -> Value {
    json!({"op":"harness_put","context":"core","name":"acceptance","kind":"examples","builds":["api"],
           "cases":[
               {"name":"a1","clauses":[clause("C1")],
                "example":{"args":"","stdin":"+ a\n","expected":"1\n#exit 0\n"}},
               {"name":"a2","clauses":[clause("C2")],
                "example":{"args":"-n 2","stdin":"+ a\n+ b\n","expected":a2_expected,"stderr":"warn\n"}}]})
}

fn harness_view(dir: &Path) -> Value {
    let r = exec(
        dir,
        &[json!({"op":"view","sections":["harnesses"],"contexts":["core"]})],
    );
    r[0]["view"]["contexts"][0]["harnesses"][0].clone()
}

fn revisions(harness: &Value) -> (u64, Vec<u64>) {
    (
        harness["revision"].as_u64().expect("a revision"),
        harness["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .map(|c| c["revision"].as_u64().expect("a revision"))
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

/// The messages of the diagnostics of a refused result.
fn messages(result: &Value) -> Vec<String> {
    assert_eq!(result["ok"], false, "{result:#?}");
    result["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .map(|d| d["message"].as_str().expect("a message").to_string())
        .collect()
}

#[test]
fn the_examples_of_a_directory_are_kept_in_the_model_and_the_view_shows_them() {
    let dir = fresh_dir("examples-dir");
    let mut ops = model();
    ops.push(from_dir());
    let put = exec(&dir, &ops);
    assert_eq!(put[4]["revision"], 1, "{:#?}", put[4]);

    let h = harness_view(&dir);
    assert_eq!(h["kind"], "examples");
    assert_eq!(h["name"], "acceptance");
    assert!(h.get("source").is_none(), "{h:#}");
    let cases = h["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 2);
    for case in cases {
        assert!(case.get("lines").is_none(), "{case:#}");
        assert!(case["cites"]["items"].as_array().expect("items").is_empty());
        assert_eq!(
            case["cites"]["clauses"].as_array().expect("clauses").len(),
            1
        );
    }
    let a2 = cases
        .iter()
        .find(|c| c["name"] == "a2")
        .expect("the case a2");
    assert_eq!(
        a2["example"],
        json!({"args":"-n 2","stdin":"+ a\n+ b\n","expected":"2\n#exit 0\n"})
    );
    assert_eq!(revisions(&h), (1, vec![1, 1]));

    // the model keeps the texts, not the directory: the files can go and the example stays
    std::fs::remove_dir_all(dir.join("acceptance")).expect("the directory goes");
    let again = harness_view(&dir);
    assert_eq!(again["cases"], h["cases"]);
}

#[test]
fn one_changed_file_moves_its_case_and_the_harness_and_the_same_files_again_move_nothing() {
    let dir = fresh_dir("examples-revision");
    let mut ops = model();
    ops.push(from_dir());
    exec(&dir, &ops);
    exec(&dir, &[from_dir()]);
    assert_eq!(revisions(&harness_view(&dir)), (1, vec![1, 1]));

    write_example(&dir, "a2", "-n 2", "+ a\n+ b\n", "2\n#exit 3\n");
    exec(&dir, &[from_dir()]);
    let h = harness_view(&dir);
    assert_eq!(revisions(&h), (2, vec![1, 2]));
    let a2 = &h["cases"][1];
    assert_eq!(a2["name"], "a2");
    assert_eq!(a2["example"]["expected"], "2\n#exit 3\n");
}

#[test]
fn each_case_may_hold_its_example_and_that_is_the_same_harness_as_the_directory() {
    let dir = fresh_dir("examples-inline");
    let mut ops = model();
    ops.push(inline("2\n#exit 0\n"));
    exec(&dir, &ops);
    let h = harness_view(&dir);
    assert_eq!(h["kind"], "examples");
    assert_eq!(h["cases"][1]["example"]["stderr"], "warn\n");
    assert!(h["cases"][0]["example"].get("stderr").is_none());
    assert_eq!(revisions(&h), (1, vec![1, 1]));
    exec(&dir, &[inline("2\n#exit 0\n")]);
    assert_eq!(revisions(&harness_view(&dir)), (1, vec![1, 1]));
    exec(&dir, &[inline("3\n#exit 0\n")]);
    assert_eq!(revisions(&harness_view(&dir)), (2, vec![1, 2]));
}

#[test]
fn a_change_set_makes_a_contract_and_a_harness_of_examples_that_cites_its_clause() {
    let dir = fresh_dir("examples-change-set");
    exec(
        &dir,
        &[
            json!({"op":"init"}),
            json!({"op":"context_add","name":"core"}),
        ],
    );
    let set = json!({"op":"change_set","puts":[
        {"put":"contract","context":"core","name":"tally","title":"Tally",
         "clauses":[{"label":"C1","text":"a line adds one to the count"},
                    {"label":"C2","text":"-n gives the count of the first n lines"}]},
        {"put":"interface","context":"core","name":"api","source":"tally\n",
         "items":[{"key":"tally","lines":[1,1]}]},
        {"put":"harness","context":"core","name":"acceptance","kind":"examples","builds":["api"],
         "examples_dir":"acceptance",
         "cases":[{"name":"a1","clauses":[clause("C1")]},{"name":"a2","clauses":[clause("C2")]}]}]});
    let done = exec(&dir, &[set]);
    assert_eq!(done[0]["op"], "change_set", "{:#?}", done[0]);
    let h = harness_view(&dir);
    assert_eq!(h["kind"], "examples");
    assert_eq!(revisions(&h), (1, vec![1, 1]));
    // the Clause the Case cites is the one the same set made
    let view = exec(
        &dir,
        &[json!({"op":"view","sections":["contracts"],"contexts":["core"]})],
    );
    let c1 = &view[0]["view"]["contexts"][0]["contracts"][0]["clauses"][0];
    assert_eq!(c1["label"], "C1");
    assert_eq!(h["cases"][0]["cites"]["clauses"][0]["id"], c1["id"]);
}

#[test]
fn a_put_that_is_not_the_shape_of_examples_is_refused_with_every_problem_named() {
    let dir = fresh_dir("examples-refused");
    exec(&dir, &model());

    // code and lines on a Harness of examples: every one is named
    let bad = json!({"op":"harness_put","context":"core","name":"acceptance","kind":"examples","builds":["api"],
        "source":"fn t() {}\n","examples_dir":"acceptance",
        "cases":[{"name":"a1","lines":[1,1],"clauses":[clause("C1")]},
                 {"name":"a2","clauses":[clause("C2")],
                  "items":[{"interface":"api","key":"tally"}]}]});
    let r = run_ops(&dir, &[bad]);
    let said = messages(&r[0]);
    assert_eq!(said.len(), 3, "{said:#?}");
    assert!(
        said.iter().any(|m| m.contains("has no source")),
        "{said:#?}"
    );
    assert!(
        said.iter().any(|m| m.contains("`a1` has `lines`")),
        "{said:#?}"
    );
    assert!(
        said.iter().any(|m| m.contains("`a2` cites items")),
        "{said:#?}"
    );

    // the texts come from the directory or from each Case, not both
    let mut both: Value = inline("2\n#exit 0\n");
    both["examples_dir"] = json!("acceptance");
    let r = run_ops(&dir, &[both]);
    let said = messages(&r[0]);
    assert_eq!(said.len(), 2, "{said:#?}");
    assert!(
        said.iter().all(|m| m.contains("use one of the two")),
        "{said:#?}"
    );

    // neither
    let neither = json!({"op":"harness_put","context":"core","name":"acceptance","kind":"examples","builds":["api"],
        "cases":[{"name":"a1","clauses":[clause("C1")]}]});
    let r = run_ops(&dir, &[neither]);
    assert!(
        messages(&r[0])[0].contains("give each case an `example`, or `examples_dir`"),
        "{:#?}",
        r[0]
    );

    // nothing was made
    let status = exec(
        &dir,
        &[json!({"op":"view","sections":["harnesses"],"contexts":["core"]})],
    );
    assert_eq!(
        status[0]["view"]["contexts"][0]["harnesses"],
        json!([]),
        "{status:#?}"
    );
}

#[test]
fn code_without_lines_and_an_example_directory_for_tests_are_refused() {
    let dir = fresh_dir("examples-tests-refused");
    exec(&dir, &model());
    let no_lines = json!({"op":"harness_put","context":"core","name":"t","source":"fn t() {}\n",
        "cases":[{"name":"t","items":[{"interface":"api","key":"tally"}]}]});
    let r = run_ops(&dir, &[no_lines]);
    assert!(
        messages(&r[0])[0].contains("the case `t` has no `lines`"),
        "{:#?}",
        r[0]
    );
    let with_dir = json!({"op":"harness_put","context":"core","name":"t","source":"fn t() {}\n",
        "examples_dir":"acceptance",
        "cases":[{"name":"t","lines":[1,1],"items":[{"interface":"api","key":"tally"}],
                  "example":{"args":"","stdin":"","expected":"#exit 0\n"}}]});
    let r = run_ops(&dir, &[with_dir]);
    let said = messages(&r[0]);
    assert_eq!(said.len(), 2, "{said:#?}");
    assert!(
        said.iter()
            .any(|m| m.contains("`examples_dir` is for a harness of examples")),
        "{said:#?}"
    );
    assert!(
        said.iter()
            .any(|m| m.contains("which only a harness of examples has")),
        "{said:#?}"
    );
    // a kind that is not one lists the kinds there are
    let odd = json!({"op":"harness_put","context":"core","name":"t","kind":"fuzz","source":"x","cases":[]});
    let r = run_ops(&dir, &[odd]);
    assert!(
        messages(&r[0])[0].ends_with("it is tests, sim or examples"),
        "{:#?}",
        r[0]
    );
}

#[test]
fn an_example_with_no_case_a_case_with_no_example_and_a_missing_file_are_refused_naming_them() {
    let dir = fresh_dir("examples-directory-refused");
    exec(&dir, &model());
    // an example no Case is named for
    write_example(&dir, "a3", "", "", "#exit 0\n");
    let r = run_ops(&dir, &[from_dir()]);
    let said = messages(&r[0]);
    assert_eq!(said.len(), 1, "{said:#?}");
    assert!(
        said[0].contains("the example `a3` in `acceptance` has no case"),
        "{said:#?}"
    );
    std::fs::remove_file(dir.join("acceptance/a3.args")).expect("a file goes");
    std::fs::remove_file(dir.join("acceptance/a3.stdin")).expect("a file goes");
    std::fs::remove_file(dir.join("acceptance/a3.expected")).expect("a file goes");

    // a Case no example is for
    let more = json!({"op":"harness_put","context":"core","name":"acceptance","kind":"examples","builds":["api"],
        "examples_dir":"acceptance",
        "cases":[{"name":"a1","clauses":[clause("C1")]},{"name":"a2","clauses":[clause("C2")]},
                 {"name":"a4","clauses":[clause("C2")]}]});
    let r = run_ops(&dir, &[more]);
    assert!(
        messages(&r[0])[0].contains("the case `a4` has no example in `acceptance`"),
        "{:#?}",
        r[0]
    );

    // a file the example needs is missing
    std::fs::remove_file(dir.join("acceptance/a2.stdin")).expect("a file goes");
    let r = run_ops(&dir, &[from_dir()]);
    let said = messages(&r[0]);
    assert_eq!(said.len(), 1, "{said:#?}");
    assert!(said[0].contains("missing a2.stdin"), "{said:#?}");
    assert!(said[0].contains("`examples_dir` `acceptance`"), "{said:#?}");

    // a file that is not text
    std::fs::write(dir.join("acceptance/a2.stdin"), [0xffu8, 0xfe]).expect("a file");
    let r = run_ops(&dir, &[from_dir()]);
    assert!(
        messages(&r[0])[0].contains("a2.stdin is not UTF-8"),
        "{:#?}",
        r[0]
    );

    // a directory that stays under the project's root, and one that is there
    for bad in ["../acceptance", "/acceptance", "nowhere"] {
        let mut put = from_dir();
        put["examples_dir"] = json!(bad);
        let r = run_ops(&dir, &[put]);
        assert_eq!(r[0]["ok"], false, "{bad}: {:#?}", r[0]);
    }
}

#[test]
fn what_the_domain_refuses_is_a_violation_of_its_rule() {
    let dir = fresh_dir("examples-rules");
    let mut ops = model();
    ops.push(from_dir());
    exec(&dir, &ops);

    // an expected output with no exit line
    let mut no_exit = inline("2\n");
    no_exit["cases"][1]["example"]["expected"] = json!("2\n");
    let r = run_ops(&dir, &[no_exit]);
    let d = one(&r[0]);
    assert_eq!(d["code"], "examples.expected-exit", "{d:#}");
    assert_eq!(d["subjects"][0]["name"], "case acceptance a2");

    // an example that cites no Clause
    let mut uncited = inline("2\n#exit 0\n");
    uncited["cases"][0]["clauses"] = json!([]);
    let r = run_ops(&dir, &[uncited]);
    assert_eq!(one(&r[0])["code"], "examples.cites-none");

    // a Harness that builds nothing
    let mut unbuilt = inline("2\n#exit 0\n");
    unbuilt["builds"] = json!([]);
    let r = run_ops(&dir, &[unbuilt]);
    assert_eq!(one(&r[0])["code"], "examples.builds-none");

    // a name that is no file name, and a name twice
    let mut slash = inline("2\n#exit 0\n");
    slash["cases"][0]["name"] = json!("a/1");
    let r = run_ops(&dir, &[slash]);
    let d = one(&r[0]);
    assert_eq!(d["code"], "examples.case-name");
    assert_eq!(d["values"]["reason"], "not-a-name");
    // in a new Harness, and in this one, where a Case named `a1` already has an id that both are given
    let mut twice = inline("2\n#exit 0\n");
    twice["name"] = json!("second");
    twice["cases"][1]["name"] = json!("a1");
    let r = run_ops(&dir, &[twice.clone()]);
    assert_eq!(one(&r[0])["values"]["reason"], "named-twice");
    twice["name"] = json!("acceptance");
    let r = run_ops(&dir, &[twice]);
    let d = one(&r[0]);
    assert_eq!(d["code"], "examples.case-name");
    assert_eq!(d["values"]["reason"], "named-twice");

    // code put to a Harness of examples without saying it is code
    let code = json!({"op":"harness_put","context":"core","name":"acceptance","source":"fn t() {}\n",
        "cases":[{"name":"t","lines":[1,1],"items":[{"interface":"api","key":"tally"}]}]});
    let r = run_ops(&dir, std::slice::from_ref(&code));
    assert_eq!(one(&r[0])["code"], "examples.kind-content");
    // and it can be said, which makes it a Harness of tests (a new revision of the Harness)
    let mut tests = code;
    tests["kind"] = json!("tests");
    exec(&dir, &[tests]);
    let h = harness_view(&dir);
    assert_eq!(h["kind"], "tests");
    assert!(h["source"].is_string());
    assert_eq!(h["cases"][0]["lines"], json!([1, 1]));
    assert!(h["cases"][0].get("example").is_none());
    assert_eq!(h["revision"], 2);
}

#[test]
fn the_new_rules_can_be_explained() {
    let dir = fresh_dir("examples-explain");
    let ids = [
        "examples.kind-content",
        "examples.cites-item",
        "examples.cites-none",
        "examples.case-name",
        "examples.builds-none",
        "examples.too-large",
        "examples.expected-exit",
    ];
    let ops: Vec<Value> = ids
        .iter()
        .map(|id| json!({"op":"explain","rule":id}))
        .collect();
    let r = exec(&dir, &ops);
    for (result, id) in r.iter().zip(ids) {
        assert_eq!(result["id"], id, "{result:#}");
        assert_eq!(result["kind"], "invariant", "{result:#}");
    }
}

#[test]
fn a_tests_harness_is_stored_as_before_and_a_harness_of_examples_survives_the_store() {
    let dir = fresh_dir("examples-store");
    let mut ops = model();
    ops.push(from_dir());
    ops.push(
        json!({"op":"harness_put","context":"core","name":"t","source":"fn t() {}\n",
        "cases":[{"name":"t","lines":[1,1],"items":[{"interface":"api","key":"tally"}]}]}),
    );
    exec(&dir, &ops);
    // a second process reads what the first one kept
    let r = exec(
        &dir,
        &[json!({"op":"view","sections":["harnesses"],"contexts":["core"]})],
    );
    let harnesses = r[0]["view"]["contexts"][0]["harnesses"]
        .as_array()
        .expect("harnesses");
    assert_eq!(harnesses.len(), 2);
    let by = |name: &str| {
        harnesses
            .iter()
            .find(|h| h["name"] == name)
            .expect("a harness")
    };
    assert_eq!(by("acceptance")["kind"], "examples");
    assert_eq!(by("acceptance")["cases"][0]["example"]["stdin"], "+ a\n");
    assert_eq!(by("t")["kind"], "tests");
    assert_eq!(by("t")["cases"][0]["lines"], json!([1, 1]));
}
