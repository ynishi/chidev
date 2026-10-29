//! The operation `view`, which reads the part of the model that is asked for, and `chidev schema`, which prints what
//! an agent may write and what it is given back.

use std::path::{Path, PathBuf};
use std::process::Command;

use chidev_core::app::ports::external::{REPLY_PROTOCOLS, finding_levels, subject_kinds};
use chidev_core::domain::check::Moment;
use chidev_core::domain::vo::crux::{CruxKind, Importance};
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
    exec_in(&fresh_dir(name), ops)
}

/// Runs `ops` through `chidev exec` in the project `dir` as it is, so that a run can go on from the ids an earlier one
/// gave back.
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
    after_setup_in(name, more).1
}

/// [`after_setup`], and the directory of the project, to go on in.
fn after_setup_in(name: &str, more: &[Value]) -> (PathBuf, Vec<Value>) {
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

#[test]
fn a_change_request_in_the_view_names_its_parent_but_not_the_words_it_was_asked_in() {
    let view = json!({"op":"view","sections":["change_requests"]});
    let r = after_setup(
        "view-cr-structure",
        &[
            json!({"op":"cr_open","title":"First version","intent":"change_behavior",
                "situation":"the requirements, in the requester's words","issuer":"the requester",
                "assumptions":[{"statement":"a reading","reason":"why",
                    "evidence":[{"knowledge":{"reference":"general"}}]}]}),
            json!({"op":"cr_open","title":"How a half rounds","intent":"change_behavior",
                "parent":"First version","targets":[{"context":"core"}]}),
            view,
        ],
    );
    assert!(r.iter().all(|x| x["ok"] == true), "{r:#?}");
    let requests = r[2]["view"]["change_requests"]
        .as_array()
        .expect("the change requests");
    let by_title = |title: &str| {
        requests
            .iter()
            .find(|c| c["title"] == title)
            .unwrap_or_else(|| panic!("{title}"))
    };
    let parent = by_title("First version");
    let part = by_title("How a half rounds");
    assert_eq!(part["parent"], parent["id"]);
    // a request that is no part has no `parent`, as it had none before; and none holds a Crux, which is a record of
    // its own
    assert!(parent.get("parent").is_none(), "{parent}");
    for c in requests {
        assert!(c.get("crux").is_none(), "{c}");
        // the words it was asked in, and their readings, are not given
        for key in ["situation", "issuer", "assumptions"] {
            assert!(c.get(key).is_none(), "{key}: {c}");
        }
    }
    assert!(r[2]["view"].get("cruxes").is_none(), "{}", r[2]["view"]);
    let schema = schema(&["view"]);
    keys_fit(&r[2]["view"], &schema, &schema, "view");
}

#[test]
fn the_cruxes_of_the_view_are_a_section_of_their_own_with_their_options_and_decisions() {
    let view = json!({"op":"view","sections":["change_requests","cruxes"],"contexts":[]});
    let raise = |kind: &str, question: &str| {
        json!({"op":"cr_crux_raise","in":"How a half rounds","kind":kind,"question":question,
            "options":[{"choice":"up"},{"choice":"even"}]})
    };
    let (dir, r) = after_setup_in(
        "view-cruxes",
        &[
            json!({"op":"cr_open","title":"First version","intent":"change_behavior"}),
            json!({"op":"cr_open","title":"How a half rounds","intent":"change_behavior",
                "parent":"First version","targets":[{"context":"core"}]}),
            json!({"op":"cr_crux_raise","in":"How a half rounds","kind":"ambiguity","importance":"normal",
                "question":"Which way does a half round?",
                "findings":[{"rule":"cite.uncited-clause","clause":{"contract":"rounding","label":"C1","context":"core"}},
                    {"rule":"context.no-harness"}],
                "options":[{"choice":"up","consequence":"0.5 gives 1",
                        "recommended":{"reason":"the usual way","confidence":60}},
                    {"choice":"even","wording":"C1: a half rounds to even",
                        "targets":[{"clause":{"context":"core","contract":"rounding","label":"C1"}}]}]}),
            raise("model", "Is a Cart a Basket?"),
            raise("conflict", "Which promise holds?"),
            raise("disagreement", "Does the test read the promise?"),
            view.clone(),
        ],
    );
    assert!(r.iter().all(|x| x["ok"] == true), "{r:#?}");
    let id = |n: usize| r[n]["crux"].as_str().expect("the id of a Crux").to_string();
    let option = |n: usize, k: usize| {
        r[n]["options"][k]["id"]
            .as_str()
            .expect("an option id")
            .to_string()
    };
    let open = &r[6]["view"];
    let cruxes = open["cruxes"].as_array().expect("the cruxes");
    assert_eq!(cruxes.len(), 4);
    let find = |view: &'_ Value, id: &str| -> Value {
        view["cruxes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no crux {id}"))
    };
    let asked = find(open, &id(2));
    let part = open["change_requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["title"] == "How a half rounds")
        .expect("the part");
    assert_eq!(asked["raised_in"], part["id"]);
    assert_eq!(asked["kind"], "ambiguity");
    assert_eq!(asked["importance"], "normal");
    assert_eq!(asked["question"], "Which way does a half round?");
    assert_eq!(asked["findings"][0]["rule"], "cite.uncited-clause");
    assert_eq!(asked["findings"][0]["subject"]["kind"], "clause");
    assert_eq!(asked["findings"][1]["subject"], Value::Null);
    assert_eq!(asked["options"][0]["id"], option(2, 0));
    assert_eq!(asked["options"][0]["choice"], "up");
    assert_eq!(asked["options"][0]["consequence"], "0.5 gives 1");
    assert_eq!(asked["options"][1]["wording"], "C1: a half rounds to even");
    assert_eq!(asked["options"][1]["expects"][0]["target"], "clause");
    assert_eq!(asked["options"][1]["expects"][0]["kind"], "target");
    assert_eq!(asked["recommended"]["option"], option(2, 0));
    assert_eq!(asked["recommended"]["reason"], "the usual way");
    assert_eq!(asked["recommended"]["confidence"], 60);
    assert_eq!(asked["supersedes"], Value::Null);
    assert_eq!(asked["state"], "open");
    assert_eq!(asked["answer"], Value::Null);
    // the request that is stopped holds no Crux of its own in the view
    assert!(
        part.get("crux").is_none() && part.get("cruxes").is_none(),
        "{part}"
    );
    assert_eq!(part["state"], "open");

    // each kind of decision, in the words of the view
    let by = json!({"person":"alice"});
    let basis = json!({"arbitrary":"settled"});
    let more = exec_in(
        &dir,
        &[
            json!({"op":"cr_crux_decide","crux":id(2),"option":option(2, 0),"reason":"the usual way",
                "basis":basis,"by":by}),
            json!({"op":"cr_crux_decide","crux":id(3),"wording":"a Cart is a Basket","basis":basis,"by":by}),
            json!({"op":"cr_crux_decide","crux":id(4),"unspecified":["a total is whole cents"],
                "basis":basis,"by":by}),
            json!({"op":"cr_crux_decide","crux":id(5),"withdrawn":true,"basis":basis,"by":by}),
            view,
        ],
    );
    assert!(more.iter().all(|x| x["ok"] == true), "{more:#?}");
    let answered = &more[4]["view"];
    let answer = |n: usize| find(answered, &id(n))["answer"].clone();
    assert_eq!(
        answer(2)["decision"],
        json!({"chosen": {"option": option(2, 0), "reason": "the usual way"}})
    );
    assert_eq!(
        answer(3)["decision"],
        json!({"worded": {"wording": "a Cart is a Basket", "reason": null}})
    );
    assert_eq!(
        answer(4)["decision"],
        json!({"unspecified": ["a total is whole cents"]})
    );
    assert_eq!(answer(5)["decision"], json!("withdrawn"));
    for n in 2..=5 {
        assert_eq!(find(answered, &id(n))["state"], "answered");
        assert!(answer(n)["id"].is_string());
        assert_eq!(
            answer(n)["basis"],
            json!({"arbitrary": {"reason": "settled"}})
        );
        assert_eq!(answer(n)["by"], json!({"kind": "person", "name": "alice"}));
        assert!(answer(n)["at"].as_u64().expect("a time") > 0);
    }
    // every Crux has a decision, so the request is decided
    let part = answered["change_requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["title"] == "How a half rounds")
        .expect("the part");
    assert_eq!(part["state"], "decided");

    // both views are in the shape the schema of the view gives, and its words for a Crux are the domain's
    let schema = schema(&["view"]);
    for view in [open, answered] {
        keys_fit(view, &schema, &schema, "view");
    }
    let crux = &schema["$defs"]["CruxView"]["properties"];
    let kinds: Vec<&str> = CruxKind::ALL.iter().map(|k| k.word()).collect();
    assert_eq!(crux["kind"]["enum"], json!(kinds));
    let mut importances: Vec<Value> = Importance::ALL.iter().map(|i| json!(i.word())).collect();
    importances.push(Value::Null);
    assert_eq!(crux["importance"]["enum"], json!(importances));
    assert_eq!(crux["state"]["enum"], json!(["open", "answered"]));
    // the cruxes are a section the operation names, and `change_requests` can be read without them
    let alone = after_setup(
        "view-cruxes-alone",
        &[json!({"op":"view","sections":["cruxes"]})],
    );
    assert_eq!(alone[0]["view"]["cruxes"], json!([]));
    assert!(alone[0]["view"].get("change_requests").is_none());
    keys_fit(&alone[0]["view"], &schema, &schema, "view");
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
    assert_eq!(v["view"]["version"], 4);
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
    // an optional value (`anyOf` of it and `null`) is checked as the value it is when it is there
    if let Some(alternatives) = schema.get("anyOf").and_then(Value::as_array)
        && let Some(present) = alternatives
            .iter()
            .find(|a| a.get("type") != Some(&json!("null")))
    {
        return resolved(present, root);
    }
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
    // every operation the setup writes has its alternative, and the two that correct records, which no setup uses
    let alternatives = schema["oneOf"].as_array().unwrap();
    let written = setup()
        .into_iter()
        .map(|op| op["op"].as_str().unwrap().to_string());
    for name in written.chain(["text_find".to_string(), "record_correct".to_string()]) {
        assert!(
            alternatives
                .iter()
                .any(|a| a["properties"]["op"]["const"] == name.as_str()),
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
    assert_eq!(schema["title"], "chidev-view 4");
    assert_eq!(schema["$id"], "urn:chidev:schema:view:4");

    let r = after_setup(
        "view-schema",
        &[
            json!({"op":"view","sections":["contracts","interfaces","harnesses","change_requests","cruxes"]}),
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
    // the words a reply may give are the ones it is read with: the lists the reading uses
    let protocols: Vec<Value> = REPLY_PROTOCOLS.iter().map(|p| json!(p)).collect();
    assert_eq!(reply["properties"]["protocol"]["enum"], json!(protocols));
    let mut levels: Vec<Value> = finding_levels().map(|l| json!(l.word())).collect();
    levels.push(Value::Null);
    assert_eq!(
        reply["$defs"]["ExternalFinding"]["properties"]["level"]["enum"],
        json!(levels)
    );
    let kinds: Vec<&str> = subject_kinds().map(|k| k.word()).collect();
    assert_eq!(
        reply["$defs"]["ExternalSubject"]["properties"]["kind"]["enum"],
        json!(kinds)
    );
    let moments: Vec<&str> = Moment::ALL.iter().map(|m| m.word()).collect();
    assert_eq!(call["properties"]["moment"]["enum"], json!(moments));

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
    // the subject inside the optional value is checked too: a misspelled key is caught
    let misspelled = json!({"protocol":2,"findings":[{"rule":"ext.keep","message":"m",
        "subject":{"knd":"context","id":"01a17790-2c62-72dd-8b3a-58ac6f178fab"}}]});
    let caught = std::panic::catch_unwind(|| keys_fit(&misspelled, &reply, &reply, "reply"));
    assert!(caught.is_err());
}
