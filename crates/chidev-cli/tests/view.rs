//! The operation `view`, which reads the part of the model that is asked for, and `chidev schema`, which prints what
//! an agent may write and what it is given back.

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

/// A project with the Contexts `core` and `extra`; `core` has a Contract, an Interface, a Harness whose Case cites
/// both, and a change request is open.
fn setup() -> Vec<Value> {
    vec![
        json!({"op":"init"}),
        json!({"op":"context_add","name":"core"}),
        json!({"op":"context_add","name":"extra"}),
        json!({"op":"contract_put","context":"core","name":"rounding","title":"Rounding",
            "clauses":[{"label":"C1","text":"a half rounds up"}]}),
        json!({"op":"interface_put","context":"core","name":"api","source":"fn price()\n",
            "items":[{"key":"price","lines":[1,1]}]}),
        json!({"op":"harness_put","context":"core","name":"tests","source":"fn t() {}\n",
            "cases":[{"name":"t","lines":[1,1],
                "items":[{"interface":"api","key":"price"}],
                "clauses":[{"contract":"rounding","label":"C1"}]}]}),
        json!({"op":"cr_open","title":"Keep the price","intent":"keep_behavior",
            "keep":[{"item":{"interface":"api","key":"price","context":"core"}}]}),
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

/// The one diagnostic of a refused result.
fn one(result: &Value) -> &Value {
    assert_eq!(result["ok"], false, "{result:#?}");
    let all = result["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(all.len(), 1, "{result:#?}");
    &all[0]
}

/// The names of the Contexts of a result of `view`, in order.
fn context_names(result: &Value) -> Vec<&str> {
    result["view"]["contexts"]
        .as_array()
        .expect("contexts")
        .iter()
        .map(|c| c["name"].as_str().expect("a name"))
        .collect()
}

#[test]
fn a_view_gives_the_sections_of_the_contexts_asked_for_and_the_revision_it_was_read_at() {
    let r = after_setup(
        "view-ok",
        &[
            json!({"op":"view","sections":["contracts","interfaces"],"contexts":["core"]}),
            json!({"op":"status"}),
        ],
    );
    let v = &r[0];
    assert_eq!(v["ok"], true, "{v:#?}");
    assert_eq!(v["op"], "view");
    assert!(v["store_revision"].is_u64());
    assert_eq!(v["view"]["version"], 3);
    assert_eq!(context_names(v), ["core"]);
    let core = &v["view"]["contexts"][0];
    for key in ["id", "name", "basis", "uses"] {
        assert!(core.get(key).is_some(), "{key}");
    }
    assert_eq!(core["contracts"][0]["name"], "rounding");
    assert_eq!(core["contracts"][0]["clauses"][0]["label"], "C1");
    assert_eq!(core["interfaces"][0]["items"][0]["key"], "price");
    assert!(core.get("harnesses").is_none(), "{core}");
    assert!(v["view"].get("change_requests").is_none());
    // a role kept from the tests is given these two sections: nothing of the Harnesses is anywhere in what it reads
    assert!(!v.to_string().contains("harnesses"), "{v}");
    assert!(!v.to_string().contains("fn t()"), "{v}");
}

#[test]
fn leaving_contexts_out_reads_every_context_and_an_empty_list_reads_none() {
    let r = after_setup(
        "view-contexts",
        &[
            json!({"op":"view","sections":["contracts","change_requests"]}),
            json!({"op":"view","sections":["contracts","change_requests"],"contexts":[]}),
        ],
    );
    assert_eq!(context_names(&r[0]), ["core", "extra"]);
    assert_eq!(r[0]["view"]["contexts"][1]["contracts"], json!([]));
    assert_eq!(r[1]["ok"], true, "{:#?}", r[1]);
    assert_eq!(r[1]["view"]["contexts"], json!([]));
    // the change requests belong to the Project, not to a Context
    assert_eq!(
        r[1]["view"]["change_requests"][0]["title"],
        "Keep the price"
    );
}

#[test]
fn a_view_without_sections_or_with_none_or_an_unknown_one_is_refused_by_its_rule() {
    let r = after_setup(
        "view-refused",
        &[
            json!({"op":"view"}),
            json!({"op":"view","sections":[]}),
            json!({"op":"view","sections":["contract"]}),
            json!({"op":"view","sections":["contracts"],"contexts":["cor"]}),
            json!({"op":"view","sections":["contracts"],"context":["core"]}),
        ],
    );
    let missing = one(&r[0]);
    assert_eq!(missing["code"], "input.missing-field");
    assert_eq!(missing["values"]["name"], "sections");

    let empty = one(&r[1]);
    assert_eq!(empty["code"], "input.view-sections-empty");
    assert_eq!(empty["field"], "sections");

    let unknown = one(&r[2]);
    assert_eq!(unknown["code"], "input.invalid");
    assert_eq!(unknown["field"], "sections[0]");
    assert_eq!(unknown["did_you_mean"], json!(["contracts"]));

    let context = one(&r[3]);
    assert_eq!(context["code"], "input.unknown-name");
    assert_eq!(context["values"]["kind"], "context");
    assert_eq!(context["did_you_mean"], json!(["core"]));

    // a misspelled `contexts` is refused, not read as "every Context"
    let misspelled = one(&r[4]);
    assert_eq!(misspelled["code"], "input.unknown-field");
    assert_eq!(misspelled["did_you_mean"], json!(["contexts"]));
}

#[test]
fn the_example_of_the_rule_for_a_view_of_no_section_is_refused_by_that_rule() {
    let explained = run(
        "view-explain",
        &[json!({"op":"explain","rule":"input.view-sections-empty"})],
    );
    let example: Value =
        serde_json::from_str(explained[0]["example"].as_str().expect("an example")).unwrap();
    let r = after_setup("view-example", &[example]);
    assert_eq!(one(&r[0])["code"], "input.view-sections-empty", "{r:#?}");
}

/// What `chidev schema` prints for `args`.
fn schema(args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .arg("schema")
        .args(args)
        .output()
        .expect("chidev runs");
    assert!(out.status.success(), "{out:#?}");
    serde_json::from_slice(&out.stdout).expect("one JSON document")
}

/// The schema `reference` (`#/$defs/Name`) points at.
fn resolved<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let path = reference.strip_prefix("#/").expect("a local reference");
            path.split('/')
                .try_fold(root, |at, key| at.get(key))
                .unwrap_or_else(|| panic!("{reference} is not in the schema"))
        }
        None => schema,
    }
}

/// Whether `value` is in the shape `schema` describes, as far as the keys go: every key of an object is one the
/// schema lists, every key it requires is there, and the same holds for what is inside. Types and formats are not
/// checked, and no crate of JSON Schema is needed to say that the view and its schema do not part ways.
fn keys_fit(value: &Value, schema: &Value, root: &Value, at: &str) {
    let schema = resolved(schema, root);
    match value {
        Value::Object(fields) => {
            let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                return;
            };
            for (key, inner) in fields {
                let listed = properties
                    .get(key)
                    .unwrap_or_else(|| panic!("{at}.{key} is not in the schema"));
                keys_fit(inner, listed, root, &format!("{at}.{key}"));
            }
            for key in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let key = key.as_str().expect("a name");
                assert!(fields.contains_key(key), "{at}.{key} is required");
            }
        }
        Value::Array(items) => {
            if let Some(each) = schema.get("items") {
                for (k, inner) in items.iter().enumerate() {
                    keys_fit(inner, each, root, &format!("{at}[{k}]"));
                }
            }
        }
        _ => {}
    }
}

#[test]
fn the_schema_of_the_operations_is_json_schema_2020_12_named_by_its_protocol() {
    let schema = schema(&["ops"]);
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["title"], "chidev-exec 1");
    assert_eq!(schema["$id"], "urn:chidev:schema:ops:1");
    // one alternative for each operation, told apart by `op`; `view` needs `sections`
    let view = schema["oneOf"]
        .as_array()
        .expect("one alternative for each operation")
        .iter()
        .find(|alternative| alternative["properties"]["op"]["const"] == "view")
        .expect("the operation `view`");
    assert!(view["properties"]["sections"].is_object(), "{view}");
    assert!(view["properties"]["contexts"].is_object(), "{view}");
    assert!(
        view["required"]
            .as_array()
            .unwrap()
            .contains(&json!("sections"))
    );
    // `check` may be narrowed to a Context, and need not be
    let check = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|alternative| alternative["properties"]["op"]["const"] == "check")
        .expect("the operation `check`");
    assert!(check["properties"]["context"].is_object(), "{check}");
    assert!(
        !check["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("context"))),
        "{check}"
    );
    // `check_rules` filters by `moment`; the argument it was called before is refused and is not offered
    let rules = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|alternative| alternative["properties"]["op"]["const"] == "check_rules")
        .expect("the operation `check_rules`");
    assert!(rules["properties"]["moment"].is_object(), "{rules}");
    assert!(rules["properties"].get("hook").is_none(), "{rules}");
    // a type that is read through another (`try_from`) is described by the one it is read through
    let address = &schema["$defs"]["AddressDto"]["properties"];
    for field in ["rule", "subject", "item", "clause"] {
        assert!(address[field].is_object(), "{field}: {address}");
    }
    // every operation the setup writes has its alternative
    let alternatives = schema["oneOf"].as_array().unwrap();
    for op in setup() {
        let name = op["op"].as_str().unwrap();
        assert!(
            alternatives
                .iter()
                .any(|a| a["properties"]["op"]["const"] == name),
            "{name}"
        );
    }
}

#[test]
fn the_schema_of_the_view_describes_the_view_an_operation_gives() {
    let schema = schema(&["view"]);
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["title"], "chidev-view 3");
    assert_eq!(schema["$id"], "urn:chidev:schema:view:3");

    let r = after_setup(
        "view-schema",
        &[
            json!({"op":"view","sections":["contracts","interfaces","harnesses","change_requests"]}),
            json!({"op":"view","sections":["contracts"],"contexts":["extra"]}),
            json!({"op":"view","sections":["change_requests"],"contexts":[]}),
        ],
    );
    for result in &r {
        assert_eq!(result["ok"], true, "{result:#?}");
        keys_fit(&result["view"], &schema, &schema, "view");
    }
    // a key the schema does not list is caught by the same check
    let mut stray = r[0]["view"].clone();
    stray["contexts"][0]["secrets"] = json!([]);
    let caught = std::panic::catch_unwind(|| keys_fit(&stray, &schema, &schema, "view"));
    assert!(caught.is_err());
}

#[test]
fn without_a_name_the_schemas_there_are_are_listed() {
    let listed = schema(&[]);
    let schemas = listed["schemas"].as_array().expect("a list");
    let names: Vec<&str> = schemas
        .iter()
        .map(|s| s["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["ops", "view", "external-call", "external-reply"]);
    // what a name prints carries the title and the id the list gives
    for s in schemas {
        let printed = schema(&[s["name"].as_str().unwrap()]);
        assert_eq!(printed["title"], s["title"]);
        assert_eq!(printed["$id"], s["$id"]);
    }
}

#[test]
fn an_unknown_name_is_refused_by_the_parser() {
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .args(["schema", "results"])
        .output()
        .expect("chidev runs");
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
#[cfg(unix)]
fn the_schemas_of_an_external_check_describe_the_request_it_is_sent_and_the_reply_it_gives() {
    use std::os::unix::fs::PermissionsExt;
    let call = schema(&["external-call"]);
    let reply = schema(&["external-reply"]);
    assert_eq!(call["title"], "chidev-external-call 2");
    assert_eq!(call["$id"], "urn:chidev:schema:external-call:2");
    assert_eq!(reply["title"], "chidev-external-reply 2");
    assert_eq!(reply["$id"], "urn:chidev:schema:external-reply:2");
    // the words a reply may give are the ones it is read with
    assert_eq!(reply["properties"]["protocol"]["enum"], json!([1, 2]));
    let finding = &reply["$defs"]["ExternalFinding"]["properties"];
    assert_eq!(
        finding["level"]["enum"],
        json!(["note", "warning", "error", null])
    );
    assert_eq!(
        reply["$defs"]["ExternalSubject"]["properties"]["kind"]["enum"],
        json!([
            "change_request",
            "context",
            "interface",
            "item",
            "clause",
            "case"
        ])
    );
    assert_eq!(
        call["properties"]["moment"]["enum"],
        json!(["edit", "pin", "resolve", "check"])
    );

    // a program that keeps what it is sent, and answers with a finding
    let dir = fresh_dir("external-schemas");
    let answer =
        json!({"protocol":2,"findings":[{"rule":"ext.keep","message":"kept","level":"note"}]});
    let checker = dir.join("checker");
    std::fs::write(
        &checker,
        format!("#!/bin/sh\ncat > request.json\nprintf '%s' '{answer}'\n"),
    )
    .expect("a file");
    std::fs::set_permissions(&checker, std::fs::Permissions::from_mode(0o755)).expect("executable");
    std::fs::write(
        dir.join("chidev.toml"),
        "[external.\"ext.keep\"]\ncommand='./checker'\nmoments=['check']\nreads=['contracts','change_requests']\n",
    )
    .expect("a file");
    let mut ops: Vec<Value> = setup();
    ops.push(json!({"op":"check_run"}));
    let text: String = ops.iter().map(|op| format!("{op}\n")).collect();
    std::fs::write(dir.join("ops.jsonl"), text).expect("a file");
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(&dir)
        .arg("exec")
        .arg("ops.jsonl")
        .output()
        .expect("chidev runs");
    assert!(out.status.success(), "{out:#?}");
    let last: Value = serde_json::from_str(
        String::from_utf8(out.stdout)
            .expect("utf-8")
            .lines()
            .last()
            .expect("a result"),
    )
    .expect("one JSON object");
    assert_eq!(
        last["external"][0]["findings"][0]["message"], "kept",
        "{last:#?}"
    );

    let request: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("request.json")).expect("the request"),
    )
    .expect("one JSON document");
    keys_fit(&request, &call, &call, "request");
    keys_fit(&answer, &reply, &reply, "reply");
    let with_subject = json!({"protocol":1,"findings":[{"rule":"ext.keep","message":"m",
        "subject":{"kind":"context","id":"01a17790-2c62-72dd-8b3a-58ac6f178fab"}}]});
    keys_fit(&with_subject, &reply, &reply, "reply");
}
