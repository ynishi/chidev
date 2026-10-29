//! `text_find` and `record_correct` through `chidev exec`: a text is found where the records keep it, replaced by a
//! mark there, and shown nowhere: not in an answer, not in a refusal, and not in the result line of the operation that
//! was given it.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// The text that must not stay.
const TOKEN: &str = "tok-SECRET-123";

/// A directory of its own under cargo's temporary one, with an empty `chidev.toml` (it is a project's root) and
/// nothing else, so that a run starts from no store.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("chidev.toml"), "").expect("a file");
    dir
}

/// One operation through a `chidev exec` of its own on `store`: the line it printed, as it was printed, and read.
fn exec(dir: &Path, store: &Path, op: &Value) -> (String, Value) {
    let file = dir.join("each.jsonl");
    std::fs::write(&file, format!("{op}\n")).expect("a file");
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .current_dir(dir)
        .arg("exec")
        .arg(&file)
        .arg("--store")
        .arg(store)
        .output()
        .expect("chidev runs");
    let raw = String::from_utf8(out.stdout).expect("utf-8");
    let line = raw.lines().next().expect("one result line").to_string();
    let result = serde_json::from_str(&line).expect("one JSON object per line");
    (line, result)
}

/// The change request in the `status` result titled `title`.
fn titled<'a>(status: &'a Value, title: &str) -> &'a Value {
    status["change_requests"]
        .as_array()
        .expect("change requests")
        .iter()
        .find(|c| c["title"] == title)
        .unwrap_or_else(|| panic!("no change request titled {title} in {status}"))
}

#[test]
fn a_text_is_found_and_corrected_and_the_result_lines_show_none_of_it() {
    let dir = fresh_dir("correct");
    let store = dir.join("chidev.db");
    let ok = |op: Value| {
        let (line, result) = exec(&dir, &store, &op);
        assert_eq!(result["ok"], true, "{line}");
        (line, result)
    };

    ok(json!({"op":"init"}));
    ok(json!({"op":"context_add","name":"core"}));
    ok(
        json!({"op":"cr_open","title":"plain","intent":"keep_behavior",
        "situation":format!("it holds {TOKEN}"),
        "assumptions":[{"statement":"it is kept","reason":format!("because {TOKEN}")}]}),
    );
    ok(
        json!({"op":"cr_open","title":format!("{TOKEN} request"),"intent":"keep_behavior",
        "situation":"nothing secret here"}),
    );
    ok(
        json!({"op":"cr_crux_raise","in":"plain","question":"Is it kept?","kind":"ambiguity",
        "options":[{"choice":"keep","recommended":{"reason":format!("{TOKEN} is why"),"confidence":60}}]}),
    );

    // find: each place is a record by id and a field, and the result holds no text
    let (line, found) = ok(json!({"op":"text_find","text":TOKEN}));
    assert!(!line.contains(TOKEN), "{line}");
    let places = found["places"].as_array().expect("places").clone();
    let kinds: Vec<(&str, Value)> = places
        .iter()
        .map(|p| {
            let (kind, place) = p.as_object().unwrap().iter().next().unwrap();
            (kind.as_str(), place["field"].clone())
        })
        .collect();
    assert_eq!(places.len(), 4, "{found}");
    for want in [
        ("change_request", json!("situation")),
        ("change_request", json!({"assumption_reason": 0})),
        ("change_request", json!("title")),
        ("crux", json!("recommendation_reason")),
    ] {
        assert!(kinds.contains(&want), "{want:?} is not in {kinds:?}");
    }

    // a refusal about the request whose title holds the text names it by its id, and shows neither the title nor
    // the text
    let titled_place = places
        .iter()
        .find(|p| p["change_request"]["field"] == "title")
        .expect("the title")
        .clone();
    let id = titled_place["change_request"]["id"].as_str().unwrap();
    let (line, refused) = exec(
        &dir,
        &store,
        &json!({"op":"record_correct","text":TOKEN,"reason":"a token was pasted",
            "by":{"person":"carol"},
            "places":[{"change_request":{"id":id,"field":"situation"}}]}),
    );
    assert_eq!(refused["ok"], false, "{line}");
    assert_eq!(refused["diagnostics"][0]["code"], "correct.text-absent");
    assert_eq!(refused["diagnostics"][0]["values"]["id"], id);
    assert!(!line.contains(TOKEN), "a refusal shows the text: {line}");
    // nor does the refusal of an unreadable place
    let (line, refused) = exec(
        &dir,
        &store,
        &json!({"op":"record_correct","text":TOKEN,"reason":"a token was pasted",
            "by":{"person":"carol"},
            "places":[{"change_request":{"id":"not-a-uuid","field":"situation"}}]}),
    );
    assert_eq!(refused["ok"], false, "{line}");
    assert!(!line.contains(TOKEN), "{line}");

    // correct the places find listed: the result line has the places, never the text
    let (line, done) = ok(
        json!({"op":"record_correct","text":TOKEN,"reason":"a token was pasted",
        "by":{"person":"carol"},"places":places}),
    );
    assert!(!line.contains(TOKEN), "the result shows the text: {line}");
    assert_eq!(done["op"], "record_correct");
    assert_eq!(done["corrected"].as_array().unwrap().len(), 4, "{done}");
    assert_eq!(done["remaining"], json!([]));
    assert_eq!(done["cleared"], "done");

    // status shows the mark and the step that says what was done
    let (line, status) = ok(json!({"op":"status"}));
    assert!(!line.contains(TOKEN), "status shows the text: {line}");
    let plain = titled(&status, "plain");
    assert_eq!(plain["situation"], "it holds [redacted]");
    assert_eq!(plain["assumptions"][0]["reason"], "because [redacted]");
    let step = plain["history"]
        .as_array()
        .unwrap()
        .last()
        .expect("a corrected step");
    assert_eq!(step["step"], "corrected");
    assert_eq!(step["fields"], json!(["situation", "assumption[0].reason"]));
    assert_eq!(step["reason"], "a token was pasted");
    assert_eq!(step["by"], json!({"kind":"person","name":"carol"}));
    // a corrected step has no resolution to name
    assert!(step.get("resolution").is_none() && step.get("of").is_none());
    titled(&status, "[redacted] request");
    let crux = &status["cruxes"][0];
    assert_eq!(
        crux["corrections"][0]["fields"],
        json!(["recommendation.reason"])
    );
    assert_eq!(crux["corrections"][0]["by"]["name"], "carol");

    // nothing is left to find, and a text that cannot be sought is refused
    let (_, again) = ok(json!({"op":"text_find","text":TOKEN}));
    assert_eq!(again["places"], json!([]));
    let (line, refused) = exec(&dir, &store, &json!({"op":"text_find","text":"red"}));
    assert_eq!(refused["ok"], false, "{line}");
    assert_eq!(refused["diagnostics"][0]["code"], "correct.text-in-mark");

    // the file keeps nothing of it
    for suffix in ["", "-journal", "-wal"] {
        let mut name = store.as_os_str().to_os_string();
        name.push(suffix);
        if let Ok(bytes) = std::fs::read(&name) {
            assert!(
                !bytes.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()),
                "the database{suffix} holds the text"
            );
        }
    }
}

#[test]
fn the_schema_of_the_operations_has_the_two_that_correct_records() {
    let out = Command::new(env!("CARGO_BIN_EXE_chidev"))
        .arg("schema")
        .arg("ops")
        .output()
        .expect("chidev runs");
    assert!(out.status.success(), "{out:#?}");
    let schema: Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    let alternatives = schema["oneOf"].as_array().expect("one alternative each");
    for name in ["text_find", "record_correct"] {
        let one = alternatives
            .iter()
            .find(|a| a["properties"]["op"]["const"] == name)
            .unwrap_or_else(|| panic!("no alternative for {name}"));
        assert!(one["properties"]["text"].is_object(), "{one}");
    }
    let correct = alternatives
        .iter()
        .find(|a| a["properties"]["op"]["const"] == "record_correct")
        .unwrap();
    for field in ["reason", "by", "places"] {
        assert!(correct["properties"][field].is_object(), "{field}");
    }
}
