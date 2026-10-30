//! `view` reads the part of the model that was asked for: the sections named, in the Contexts named.

use std::collections::BTreeSet;

use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, BasisInput, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, CruxKind,
    DecisionInput, DecisionMakerInput, ExampleInput, ExampleTexts, Importance, Intent, ItemInput,
    ItemRef, OptionInput, RaiseCruxInput, RecommendationInput, TargetInput, ViewInput,
};
use chidev_core::domain::vo::model_section::ModelSection;
use chidev_core::domain::vo::name::{AssetName, ContextName};
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
            replaces: None,
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
                interface: AssetName::try_from("api").unwrap(),
                key: "price".into(),
            }],
            clauses: vec![ClauseRef {
                context: None,
                contract: AssetName::try_from("stock").unwrap(),
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
            context: Some(ContextName::try_from("core").unwrap()),
            interface: AssetName::try_from("api").unwrap(),
            key: "price".into(),
        })],
        assumptions: vec![],
        addresses: vec![],
    })
    .await
    .unwrap();
    app.raise_crux(&RaiseCruxInput {
        cr: "keep the price".into(),
        kind: CruxKind::Ambiguity,
        question: "Does keeping the price keep its rounding?".into(),
        findings: vec![],
        options: vec![
            OptionInput {
                choice: "yes".into(),
                change: Some("a price is rounded as it is".into()),
                targets: vec![],
                keep: vec![],
                consequence: Some("nothing else moves".into()),
                recommended: Some(RecommendationInput {
                    reason: Some("the literal reading".into()),
                    confidence: Some(70),
                }),
            },
            OptionInput {
                choice: "no".into(),
                change: None,
                targets: vec![TargetInput::Clause(ClauseRef {
                    context: Some(ContextName::try_from("core").unwrap()),
                    contract: AssetName::try_from("stock").unwrap(),
                    label: "C1".into(),
                })],
                keep: vec![],
                consequence: None,
                recommended: None,
            },
        ],
        importance: Some(Importance::High),
        supersedes: None,
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
    assert_eq!(view["version"], 5);
    assert!(view.get("change_requests").is_none());
    assert!(view.get("cruxes").is_none());
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
async fn the_view_is_version_5_and_shows_the_kind_of_a_harness_and_the_examples_of_its_cases() {
    let mut app = project("examples").await;
    app.put_examples_harness(
        "core",
        "acceptance",
        &["api".to_string()],
        &[
            ExampleInput {
                name: "a1".into(),
                clauses: vec![ClauseRef {
                    context: None,
                    contract: AssetName::try_from("stock").unwrap(),
                    label: "C1".into(),
                }],
                example: ExampleTexts {
                    args: "-n 1".into(),
                    stdin: "in\n".into(),
                    expected: "out\n#exit 0\n".into(),
                    stderr: Some("warn\n".into()),
                },
            },
            ExampleInput {
                name: "a2".into(),
                clauses: vec![ClauseRef {
                    context: None,
                    contract: AssetName::try_from("stock").unwrap(),
                    label: "C1".into(),
                }],
                example: ExampleTexts {
                    args: String::new(),
                    stdin: String::new(),
                    expected: "#exit 2\n".into(),
                    stderr: None,
                },
            },
        ],
    )
    .await
    .unwrap();
    let view = read(
        &app,
        &ViewInput {
            contexts: Some(vec!["core".to_string()]),
            sections: sections(&[ModelSection::Harnesses]),
        },
    )
    .await;
    assert_eq!(view["version"], 5);
    let harnesses = view["contexts"][0]["harnesses"].as_array().unwrap();
    let by = |name: &str| harnesses.iter().find(|h| h["name"] == name).unwrap();
    // a Harness of code says its kind, and has its source and its Cases on lines
    let tests = by("tests");
    assert_eq!(tests["kind"], "tests");
    assert_eq!(tests["source"], "fn t() {}\n");
    assert_eq!(tests["cases"][0]["lines"], json!([1, 1]));
    assert!(tests["cases"][0].get("example").is_none());
    // one of examples has no source, and its Cases are on no lines but hold their examples
    let acceptance = by("acceptance");
    assert_eq!(acceptance["kind"], "examples");
    assert!(acceptance.get("source").is_none(), "{acceptance:#}");
    let cases = acceptance["cases"].as_array().unwrap();
    assert!(cases.iter().all(|c| c.get("lines").is_none()));
    assert_eq!(
        cases[0]["example"],
        json!({"args":"-n 1","stdin":"in\n","expected":"out\n#exit 0\n","stderr":"warn\n"})
    );
    assert_eq!(
        cases[1]["example"],
        json!({"args":"","stdin":"","expected":"#exit 2\n"})
    );
    assert_eq!(cases[0]["cites"]["items"], json!([]));
    assert_eq!(cases[0]["cites"]["clauses"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_view_of_every_section_is_the_model_a_judged_rule_is_given() {
    let app = project("whole").await;
    let read_at = app
        .view(&ViewInput {
            contexts: None,
            sections: sections(ModelSection::ALL),
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
    assert_eq!(
        err,
        AppError::UnknownContext(ContextName::try_from("cor").unwrap())
    );
    let diagnostics = app.diagnose(&err).await;
    assert_eq!(diagnostics[0].code, "input.unknown-name");
    assert_eq!(diagnostics[0].did_you_mean, ["core"]);
}

fn alice() -> DecisionMakerInput {
    DecisionMakerInput::Person("alice".into())
}

fn arbitrary() -> BasisInput {
    BasisInput::Arbitrary("settled".into())
}

#[tokio::test]
async fn the_cruxes_are_a_section_of_their_own_beside_the_change_requests() {
    let app = project("cruxes").await;
    let view = read(
        &app,
        &ViewInput {
            contexts: None,
            sections: sections(&[ModelSection::Cruxes]),
        },
    )
    .await;
    // asked for alone, they are alone: no change request, no asset section, and the Contexts name themselves
    assert!(view.get("change_requests").is_none());
    assert_eq!(context_names(&view), ["base", "core"]);
    assert!(view["contexts"][0].get("contracts").is_none());
    let cruxes = view["cruxes"].as_array().unwrap();
    assert_eq!(cruxes.len(), 1);
    let crux = &cruxes[0];
    let options = crux["options"].as_array().unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0]["choice"], "yes");
    assert_eq!(options[0]["wording"], "a price is rounded as it is");
    assert_eq!(options[0]["consequence"], "nothing else moves");
    assert_eq!(options[1]["expects"][0]["target"], "clause");
    assert_eq!(options[1]["expects"][0]["kind"], "target");
    assert_ne!(options[0]["id"], options[1]["id"]);
    assert_eq!(
        crux["question"],
        "Does keeping the price keep its rounding?"
    );
    assert_eq!(crux["kind"], "ambiguity");
    assert_eq!(crux["importance"], "high");
    assert_eq!(crux["recommended"]["option"], options[0]["id"]);
    assert_eq!(crux["recommended"]["reason"], "the literal reading");
    assert_eq!(crux["recommended"]["confidence"], 70);
    assert_eq!(crux["findings"], json!([]));
    assert_eq!(crux["supersedes"], Value::Null);
    assert_eq!(crux["state"], "open");
    assert_eq!(crux["answer"], Value::Null);

    // with the change requests, the Crux points at the one it stops, and that one holds no Crux
    let both = read(
        &app,
        &ViewInput {
            contexts: Some(vec![]),
            sections: sections(&[ModelSection::ChangeRequests, ModelSection::Cruxes]),
        },
    )
    .await;
    let request = &both["change_requests"][0];
    assert_eq!(both["cruxes"][0]["raised_in"], request["id"]);
    assert!(request.get("crux").is_none(), "{request}");
    assert_eq!(request["state"], "open");
}

#[tokio::test]
async fn a_decision_is_in_the_view_in_the_words_of_its_kind() {
    let mut app = project("answers").await;
    let ask = |question: &str| RaiseCruxInput {
        cr: "keep the price".into(),
        kind: CruxKind::Ambiguity,
        question: question.into(),
        findings: vec![],
        options: vec![OptionInput {
            choice: "only".into(),
            change: None,
            targets: vec![],
            keep: vec![],
            consequence: None,
            recommended: None,
        }],
        importance: None,
        supersedes: None,
    };
    let mut ids = Vec::new();
    for q in ["worded", "free", "withdrawn"] {
        ids.push(app.raise_crux(&ask(q)).await.unwrap().value.id.to_string());
    }
    let first = read(
        &app,
        &ViewInput {
            contexts: Some(vec![]),
            sections: sections(&[ModelSection::Cruxes]),
        },
    )
    .await;
    let chosen = first["cruxes"][0]["options"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let first_id = first["cruxes"][0]["id"].as_str().unwrap().to_string();
    let decisions = [
        (
            first_id.clone(),
            DecisionInput::Chosen {
                option: chosen.clone(),
                reason: Some("fairer".into()),
            },
        ),
        (
            ids[0].clone(),
            DecisionInput::Worded {
                wording: "round half to even".into(),
                reason: None,
            },
        ),
        (
            ids[1].clone(),
            DecisionInput::Unspecified {
                keep: vec!["a total is whole cents".into()],
            },
        ),
        (ids[2].clone(), DecisionInput::Withdrawn),
    ];
    for (crux, decision) in &decisions {
        app.decide_crux(crux, decision, None, &arbitrary(), &alice())
            .await
            .unwrap();
    }
    let view = read(
        &app,
        &ViewInput {
            contexts: Some(vec![]),
            sections: sections(&[ModelSection::ChangeRequests, ModelSection::Cruxes]),
        },
    )
    .await;
    let answer = |n: usize| &view["cruxes"][n]["answer"];
    assert_eq!(
        answer(0)["decision"],
        json!({"chosen": {"option": chosen, "reason": "fairer"}})
    );
    assert_eq!(
        answer(1)["decision"],
        json!({"worded": {"wording": "round half to even", "reason": null}})
    );
    assert_eq!(
        answer(2)["decision"],
        json!({"unspecified": ["a total is whole cents"]})
    );
    assert_eq!(answer(3)["decision"], json!("withdrawn"));
    for n in 0..4 {
        assert_eq!(view["cruxes"][n]["state"], "answered");
        assert_eq!(
            answer(n)["basis"],
            json!({"arbitrary": {"reason": "settled"}})
        );
        assert_eq!(answer(n)["by"], json!({"kind": "person", "name": "alice"}));
        assert_eq!(answer(n)["at"], 1_000);
        assert!(answer(n)["id"].is_string());
    }
    // every Crux has a decision, so the change request is decided
    assert_eq!(view["change_requests"][0]["state"], "decided");
}

#[tokio::test]
async fn the_stamps_of_a_change_request_and_of_the_project_cover_its_cruxes() {
    use chidev_core::domain::vo::finding::{Subject, SubjectKind};
    use chidev_core::domain::vo::id::ChangeRequestId;

    let mut app = project("stamps").await;
    let model = app.check_rules(None, None).await.unwrap().model;
    let whole = serde_json::to_value(&model).unwrap();
    let id = ChangeRequestId::from_uuid(
        whole["change_requests"][0]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    let stamps = |m: &chidev_core::app::ModelView| {
        (
            m.stamp_of(Some(Subject::ChangeRequest(id))),
            m.scope_stamp(SubjectKind::ChangeRequest),
            m.scope_stamp(SubjectKind::Project),
            m.scope_stamp(SubjectKind::Context),
        )
    };
    let before = stamps(&model);
    // A second question for the same request leaves the request as its own entry shows it (`open`), and still changes
    // what it is stamped with.
    app.raise_crux(&RaiseCruxInput {
        cr: "keep the price".into(),
        kind: CruxKind::Conflict,
        question: "Which of the two promises holds?".into(),
        findings: vec![],
        options: vec![OptionInput {
            choice: "the first".into(),
            change: None,
            targets: vec![],
            keep: vec![],
            consequence: None,
            recommended: None,
        }],
        importance: None,
        supersedes: None,
    })
    .await
    .unwrap();
    let later = app.check_rules(None, None).await.unwrap().model;
    assert_eq!(
        serde_json::to_value(&later).unwrap()["change_requests"],
        whole["change_requests"]
    );
    let after = stamps(&later);
    assert_ne!(before.0, after.0, "the entry of the change request");
    assert_ne!(before.1, after.1, "the change requests");
    assert_ne!(before.2, after.2, "the project");
    assert_eq!(before.3, after.3, "the Contexts do not hold Cruxes");
}
