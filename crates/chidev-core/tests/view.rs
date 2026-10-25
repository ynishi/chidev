//! `view` reads the part of the model that was asked for: the sections named, in the Contexts named.

use std::collections::BTreeSet;

use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, Intent, ItemInput,
    ItemRef, TargetInput, ViewInput,
};
use chidev_core::domain::vo::model_section::ModelSection;
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::sqlite::SqliteStore;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Default)]
struct Counter(u128);

impl IdSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_u128(self.0)
    }
}

/// A project of two Contexts, `base` and `core` (which uses it), each with a Contract; `core` also has an Interface,
/// a Harness with a Case, and a change request.
async fn project(name: &str) -> App<SqliteStore, Counter> {
    let dir = std::env::temp_dir().join(format!("chidev-view-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
    let mut app = App::new(
        store,
        Counter::default(),
        FixedClock(Timestamp::from_millis(1_000)),
    );
    app.init().await.unwrap();
    app.add_context("base", &[]).await.unwrap();
    app.add_context("core", &["base".to_string()])
        .await
        .unwrap();
    let clause = |text: &str| {
        vec![ClauseInput {
            label: "C1".into(),
            text: text.into(),
        }]
    };
    app.put_contract("base", "rules", "Rules", &clause("a half rounds up"))
        .await
        .unwrap();
    app.put_contract("core", "stock", "Stock", &clause("a total is not negative"))
        .await
        .unwrap();
    app.put_interface(
        "core",
        "api",
        "fn price()\n",
        &[ItemInput {
            key: "price".into(),
            lines: (1, 1),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    app.put_harness(
        "core",
        "tests",
        "fn t() {}\n",
        &[CaseInput {
            name: "t".into(),
            lines: (1, 1),
            items: vec![ItemRef {
                context: None,
                interface: "api".into(),
                key: "price".into(),
            }],
            clauses: vec![ClauseRef {
                context: None,
                contract: "stock".into(),
                label: "C1".into(),
            }],
        }],
    )
    .await
    .unwrap();
    app.open_change_request(&ChangeRequestInput {
        title: "keep the price".into(),
        situation: None,
        intent: Intent::KeepBehavior,
        parent: None,
        issuer: None,
        targets: vec![],
        keep: vec![TargetInput::Item(ItemRef {
            context: Some("core".into()),
            interface: "api".into(),
            key: "price".into(),
        })],
        crux: None,
        assumptions: vec![],
        addresses: vec![],
    })
    .await
    .unwrap();
    app
}

fn sections(of: &[ModelSection]) -> BTreeSet<ModelSection> {
    of.iter().copied().collect()
}

/// The view `input` reads, as the JSON an agent is given.
async fn read(app: &App<SqliteStore, Counter>, input: &ViewInput) -> Value {
    serde_json::to_value(app.view(input).await.unwrap().view).unwrap()
}

/// The names of the Contexts of a view, in order.
fn context_names(view: &Value) -> Vec<&str> {
    view["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn only_the_sections_asked_for_are_in_the_view_and_the_contexts_always_name_themselves() {
    let app = project("sections").await;
    let view = read(
        &app,
        &ViewInput {
            contexts: None,
            sections: sections(&[ModelSection::Contracts]),
        },
    )
    .await;
    assert_eq!(view["version"], 3);
    assert!(view.get("change_requests").is_none());
    assert_eq!(context_names(&view), ["base", "core"]);
    for context in view["contexts"].as_array().unwrap() {
        for key in ["id", "name", "basis", "uses"] {
            assert!(context.get(key).is_some(), "{key}: {context}");
        }
        assert!(context["contracts"][0]["clauses"][0]["text"].is_string());
        assert!(context.get("interfaces").is_none(), "{context}");
        assert!(context.get("harnesses").is_none(), "{context}");
    }
    assert_eq!(view["contexts"][1]["uses"], json!(["base"]));
    assert_eq!(view["contexts"][1]["contracts"][0]["name"], "stock");

    let view = read(
        &app,
        &ViewInput {
            contexts: None,
            sections: sections(&[ModelSection::Interfaces, ModelSection::ChangeRequests]),
        },
    )
    .await;
    assert_eq!(view["change_requests"][0]["title"], "keep the price");
    assert_eq!(view["contexts"][1]["interfaces"][0]["name"], "api");
    assert_eq!(
        view["contexts"][1]["interfaces"][0]["items"][0]["key"],
        "price"
    );
    // an empty section is `[]`, a section not asked for is no key
    assert_eq!(view["contexts"][0]["interfaces"], json!([]));
    for context in view["contexts"].as_array().unwrap() {
        assert!(context.get("contracts").is_none(), "{context}");
        assert!(context.get("harnesses").is_none(), "{context}");
    }

    let view = read(
        &app,
        &ViewInput {
            contexts: None,
            sections: sections(&[ModelSection::Harnesses]),
        },
    )
    .await;
    assert_eq!(view["contexts"][1]["harnesses"][0]["cases"][0]["name"], "t");
    assert!(view.get("change_requests").is_none());
}

#[tokio::test]
async fn the_contexts_named_are_the_only_ones_in_the_view_and_none_named_is_all_of_them() {
    let app = project("contexts").await;
    let of = |contexts: Option<Vec<&str>>| ViewInput {
        contexts: contexts.map(|names| names.into_iter().map(str::to_string).collect()),
        sections: sections(&[ModelSection::Contracts, ModelSection::ChangeRequests]),
    };
    assert_eq!(
        context_names(&read(&app, &of(None)).await),
        ["base", "core"]
    );
    let only_core = read(&app, &of(Some(vec!["core"]))).await;
    assert_eq!(context_names(&only_core), ["core"]);
    // the change requests belong to the Project, not to a Context
    assert_eq!(only_core["change_requests"].as_array().unwrap().len(), 1);
    // `[]` is no Context, not every one
    let none = read(&app, &of(Some(vec![]))).await;
    assert_eq!(none["contexts"], json!([]));
    assert_eq!(none["change_requests"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_view_of_every_section_is_the_model_a_judged_rule_is_given() {
    let app = project("whole").await;
    let read_at = app
        .view(&ViewInput {
            contexts: None,
            sections: sections(&ModelSection::ALL),
        })
        .await
        .unwrap();
    let rules = app.check_rules(None, None).await.unwrap();
    assert_eq!(read_at.store_revision, rules.store_revision);
    assert_eq!(
        read_at.store_revision,
        app.store().revision().await.unwrap()
    );
    let mut whole = serde_json::to_value(&rules.model).unwrap();
    whole.as_object_mut().unwrap().remove("store_revision");
    assert_eq!(serde_json::to_value(&read_at.view).unwrap(), whole);
}

#[tokio::test]
async fn a_view_that_names_no_section_or_an_unknown_context_is_refused_by_its_rule() {
    let app = project("refused").await;
    let err = app
        .view(&ViewInput {
            contexts: None,
            sections: BTreeSet::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err, AppError::ViewSectionsEmpty);
    let diagnostics = app.diagnose(&err).await;
    assert_eq!(diagnostics[0].code, "input.view-sections-empty");
    assert_eq!(diagnostics[0].field.as_deref(), Some("sections"));

    let err = app
        .view(&ViewInput {
            contexts: Some(vec!["cor".into()]),
            sections: sections(&[ModelSection::Contracts]),
        })
        .await
        .unwrap_err();
    assert_eq!(err, AppError::UnknownContext("cor".into()));
    let diagnostics = app.diagnose(&err).await;
    assert_eq!(diagnostics[0].code, "input.unknown-name");
    assert_eq!(diagnostics[0].did_you_mean, ["core"]);
}
