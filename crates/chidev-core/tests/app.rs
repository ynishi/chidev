//! The use cases over the real store: each test opens an SQLite database of its own in a fresh temporary
//! directory, so no test double of the store is needed.

use chidev_core::app::ports::{AssetKind, AssetNames, IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeSetInput, ClauseInput, ClauseRef, ContractPut, Diagnostic,
    ExampleInput, ExampleTexts, HarnessKind, HarnessPut, HarnessPutContent, InterfacePut,
    ItemInput, ItemRef, ItemsInput, PutDone, PutInput, RetireInput,
};
use chidev_core::domain::entity::contract::Clause;
use chidev_core::domain::repository::ProjectRepository;
use chidev_core::domain::vo::finding::Subject;
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::sqlite::SqliteStore;
use uuid::Uuid;

/// Ids 1, 2, 3, ...
#[derive(Default)]
struct Counter(u128);

impl IdSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_u128(self.0)
    }
}

/// An App over a new database at a path no other test uses.
async fn app(name: &str) -> App<SqliteStore, Counter> {
    let dir = std::env::temp_dir().join(format!("chidev-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
    App::new(
        store,
        Counter::default(),
        FixedClock(Timestamp::from_millis(1_000)),
    )
}

/// What a refusal says: the messages of its diagnostics, one per line.
async fn said(app: &App<SqliteStore, Counter>, err: &AppError) -> String {
    app.diagnose(err)
        .await
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_project_is_made_once() {
    let mut app = app("init").await;
    let id = app.init().await.unwrap();
    assert_eq!(app.store().project().await.unwrap().unwrap().id(), id);
    assert_eq!(app.init().await, Err(AppError::AlreadyInitialized));
}

#[tokio::test]
async fn a_use_case_before_init_finds_no_project() {
    let mut app = app("no-project").await;
    assert_eq!(app.add_context("core", &[]).await, Err(AppError::NoProject));
    assert_eq!(app.status().await, Err(AppError::NoProject));
}

/// The sim Harness `sim` of `core`, which builds `api`: a sweep Case citing the Clause `label` of the Contract
/// `contract` of the Context `context`.
async fn put_sim(
    app: &mut App<SqliteStore, Counter>,
    context: Option<&str>,
    contract: &str,
    label: &str,
) -> Result<(), AppError> {
    app.put_harness_of(
        "core",
        "sim",
        Some(HarnessKind::Sim),
        &["api".to_string()],
        "x\n",
        &[CaseInput {
            name: "sweep".to_string(),
            lines: (1, 1),
            items: Vec::new(),
            clauses: vec![ClauseRef {
                context: context.map(|c| ContextName::try_from(c).unwrap()),
                contract: AssetName::try_from(contract).unwrap(),
                label: label.to_string(),
            }],
        }],
    )
    .await
    .map(|_| ())
}

/// The Clause with this label of the Contract `contract`, in any Context of the Project.
async fn clause_of(app: &App<SqliteStore, Counter>, label: &str) -> Clause {
    let p = app.store().project().await.unwrap().unwrap();
    p.contexts()
        .flat_map(|c| c.contracts())
        .flat_map(|k| k.clauses())
        .find(|c| c.label().to_string() == label)
        .cloned()
        .unwrap()
}

#[tokio::test]
async fn a_sim_harness_a_store_kept_in_a_shape_a_put_is_refused_for_is_read_reported_and_mended_by_a_put()
 {
    let dir = std::env::temp_dir().join(format!("chidev-app-sim-kept-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("chidev.db");
    let open = |start: u128| {
        let path = path.clone();
        async move {
            App::new(
                SqliteStore::open(&path).await.unwrap(),
                Counter(start),
                FixedClock(Timestamp::from_millis(1_000)),
            )
        }
    };
    let mut app = open(0).await;
    app.init().await.unwrap();
    app.add_context("base", &[]).await.unwrap();
    app.add_context("core", &["base".to_string()])
        .await
        .unwrap();
    app.put_contract("core", "stock", "Stock", &clauses(&["C1"]))
        .await
        .unwrap();
    app.put_contract("base", "rules", "Rules", &clauses(&["C2"]))
        .await
        .unwrap();
    app.put_interface("core", "api", "fn a()\n", &[item("a", 1)])
        .await
        .unwrap();

    // a Clause of the Context it uses is reached, but a sim Harness is run against the Contracts of its own
    let err = put_sim(&mut app, Some("base"), "rules", "C2")
        .await
        .unwrap_err();
    let refused = app.diagnose(&err).await;
    assert_eq!(refused.len(), 1, "{refused:#?}");
    assert_eq!(refused[0].code, "sim.clause-foreign");
    assert_eq!(refused[0].level, "error");
    let named: Vec<(&str, &str)> = refused[0]
        .subjects
        .iter()
        .map(|s| (s.kind, s.name.as_str()))
        .collect();
    assert_eq!(named, [("case", "case sim sweep"), ("clause", "rules C2")]);
    assert!(!refused[0].message.contains("0000-0000"), "{refused:#?}");

    // in a shape the protocol runs it is kept
    put_sim(&mut app, None, "stock", "C1").await.unwrap();

    // a store from before the rule, with the sweep citing the Clause of `base`: the document is rewritten
    let stock = clause_of(&app, "C1").await;
    let rules = clause_of(&app, "C2").await;
    let cite = |c: &chidev_core::domain::entity::contract::Clause| {
        format!(
            "\"clauses\":[{{\"id\":\"{}\",\"revision\":{}}}]",
            c.id().as_uuid(),
            c.revision().number()
        )
    };
    app.into_store().close().await.unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let rewritten = conn
        .execute(
            "UPDATE project SET doc = replace(doc, ?1, ?2)",
            [cite(&stock), cite(&rules)],
        )
        .unwrap();
    assert_eq!(rewritten, 1);
    drop(conn);
    let mut app = open(1_000).await;

    // the store is read, and `check` reports the shape by the Clause, once, by name and not by id
    let found = app.check(None, &[]).await.unwrap().findings;
    let kept: Vec<_> = found
        .iter()
        .filter(|f| f.rule.as_str() == "sim.shape-kept")
        .collect();
    assert_eq!(kept.len(), 1, "{found:#?}");
    assert_eq!(kept[0].level, chidev_core::domain::check::Level::Error);
    assert_eq!(kept[0].subject, Some(Subject::Clause(rules.id())));
    assert_eq!(kept[0].about.as_deref(), Some("rules C2"));
    assert!(
        !kept[0].message.contains("0000-0000"),
        "{}",
        kept[0].message
    );
    let shown = Diagnostic::of_found(kept[0]);
    assert_eq!(shown.code, "sim.shape-kept");
    assert_eq!(shown.subjects[0].kind, "clause");
    assert_eq!(shown.subjects[0].name, "rules C2");

    // a change that does not put it goes through
    app.put_contract("core", "extra", "Extra", &clauses(&["C3"]))
        .await
        .unwrap();
    // simulating it is refused for the rule a put of it breaks
    let err = app.simulate("core", "sim").await.unwrap_err();
    let refused = app.diagnose(&err).await;
    assert_eq!(refused[0].code, "sim.clause-foreign", "{refused:#?}");
    // a put of it as it is, is refused for the same rule
    let err = put_sim(&mut app, Some("base"), "rules", "C2")
        .await
        .unwrap_err();
    let refused = app.diagnose(&err).await;
    assert_eq!(refused[0].code, "sim.clause-foreign");
    // the Harness is in the store now, so the remedy is an operation that puts it again
    assert_eq!(refused[0].suggestions.len(), 1, "{refused:#?}");
    // and a put in the shape the protocol runs mends it
    put_sim(&mut app, None, "stock", "C1").await.unwrap();
    let found = app.check(None, &[]).await.unwrap().findings;
    assert!(
        found.iter().all(|f| f.rule.as_str() != "sim.shape-kept"),
        "{found:#?}"
    );
}

#[tokio::test]
async fn an_asset_and_its_name_are_kept_together() {
    let mut app = app("asset-name").await;
    app.init().await.unwrap();
    let ctx = app.add_context("core", &[]).await.unwrap().value;
    let put = app
        .put_contract(
            "core",
            "stack",
            "Stack",
            &[ClauseInput {
                label: "C1".to_string(),
                text: "push then pop".to_string(),
                replaces: None,
            }],
        )
        .await
        .unwrap();
    let store = app.store();
    assert_eq!(
        store
            .asset_id(
                ctx,
                AssetKind::Contract,
                &AssetName::try_from("stack").unwrap()
            )
            .await
            .unwrap(),
        Some(put.value.id.as_uuid())
    );
    let p = store.project().await.unwrap().unwrap();
    assert!(p.context(ctx).unwrap().contract(put.value.id).is_some());
}

#[tokio::test]
async fn a_name_that_breaks_the_rule_is_refused_with_the_error_of_the_rule_as_its_source() {
    use chidev_core::domain::vo::name::{NameError, NameOf};

    let mut app = app("bad-name").await;
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    let revision = app.store().revision().await.unwrap();
    let bad = app
        .put_contract("core", "Stack", "Stack", &clauses(&["C1"]))
        .await
        .unwrap_err();
    let rule = NameError::BadChar {
        of: NameOf::Asset,
        name: "Stack".to_string(),
        ch: 'S',
    };
    assert_eq!(bad, AppError::BadName(rule.clone()));
    let source = std::error::Error::source(&bad).expect("the error of the rule is the source");
    assert_eq!(source.to_string(), rule.to_string());
    // a Context's name is refused by the same rule, as its own kind of name
    assert_eq!(
        app.put_contract("Core", "stack", "Stack", &clauses(&["C1"]))
            .await,
        Err(AppError::BadName(NameError::BadChar {
            of: NameOf::Context,
            name: "Core".to_string(),
            ch: 'C',
        }))
    );
    for refused in [
        app.put_interface("core", "a b", "fn f()\n", &[])
            .await
            .map(|_| ()),
        app.put_harness("core", "a.b", "", &[]).await.map(|_| ()),
        app.retire_contract("core", "a:b").await.map(|_| ()),
        app.pin("core", "a/b", "").await.map(|_| ()),
    ] {
        assert!(matches!(refused, Err(AppError::BadName(_))), "{refused:?}");
    }
    // the diagnostic says what the rule refuses
    let said = said(&app, &bad).await;
    assert!(said.contains("an asset name"), "{said}");
    assert!(said.contains("`S`"), "{said}");
    assert_eq!(app.store().revision().await.unwrap(), revision);
}

/// `stock` with Clauses C1 and C7, its Interface with the Items `reserve` and `release`, and a Harness `stock-tests`
/// whose Case `reserve_refuses` cites the first of each and `release_frees` the second.
async fn stock(name: &str) -> App<SqliteStore, Counter> {
    let mut app = app(name).await;
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract("core", "stock", "Stock", &clauses(&["C1", "C7"]))
        .await
        .unwrap();
    app.put_interface(
        "core",
        "stock",
        "fn reserve()\nfn release()\n",
        &[item("reserve", 1), item("release", 2)],
    )
    .await
    .unwrap();
    app.put_harness(
        "core",
        "stock-tests",
        "fn reserve_refuses() {}\nfn release_frees() {}\n",
        &cases(&["reserve", "release"]),
    )
    .await
    .unwrap();
    app
}

fn clauses(labels: &[&str]) -> Vec<ClauseInput> {
    labels
        .iter()
        .map(|l| ClauseInput {
            label: (*l).to_string(),
            text: format!("{l} holds"),
            replaces: None,
        })
        .collect()
}

fn item(key: &str, line: usize) -> ItemInput {
    ItemInput {
        key: key.to_string(),
        lines: (line, line),
        replaces: None,
    }
}

/// The Cases for the words `reserve` and `release`: `reserve_refuses` cites the Item `reserve` and C1, and
/// `release_frees` cites the Item `release` and C7.
fn cases(words: &[&str]) -> Vec<CaseInput> {
    words
        .iter()
        .enumerate()
        .map(|(k, w)| CaseInput {
            name: if *w == "reserve" {
                "reserve_refuses".to_string()
            } else {
                "release_frees".to_string()
            },
            lines: (k + 1, k + 1),
            items: vec![ItemRef {
                context: None,
                interface: AssetName::try_from("stock").unwrap(),
                key: (*w).to_string(),
            }],
            clauses: vec![ClauseRef {
                context: None,
                contract: AssetName::try_from("stock").unwrap(),
                label: if *w == "reserve" { "C1" } else { "C7" }.to_string(),
            }],
        })
        .collect()
}

/// The change set that removes Clause C7 and Item `release` and puts the Harness without the Case that cites them.
fn removal() -> ChangeSetInput {
    ChangeSetInput {
        change_request: None,
        puts: vec![
            PutInput::Contract(ContractPut {
                context: "core".to_string(),
                name: AssetName::try_from("stock").unwrap(),
                title: "Stock".to_string(),
                clauses: clauses(&["C1"]),
            }),
            PutInput::Interface(InterfacePut {
                context: "core".to_string(),
                name: AssetName::try_from("stock").unwrap(),
                source: "fn reserve()\n".to_string(),
                items: ItemsInput::Written(vec![item("reserve", 1)]),
            }),
            PutInput::Harness(HarnessPut {
                context: "core".to_string(),
                name: AssetName::try_from("stock-tests").unwrap(),
                builds: Vec::new(),
                content: HarnessPutContent::Code {
                    kind: None,
                    source: "fn reserve_refuses() {}\n".to_string(),
                    cases: cases(&["reserve"]),
                },
            }),
        ],
        retires: Vec::new(),
    }
}

#[tokio::test]
async fn a_change_set_removes_what_cases_cite_with_the_harness_in_one_commit() {
    let mut app = stock("change-set").await;
    let before = app.store().revision().await.unwrap();
    let done = app.apply_change_set(&removal()).await.unwrap();
    assert_eq!(
        app.store().revision().await.unwrap().0,
        before.0 + 1,
        "one commit"
    );
    let kinds: Vec<&str> = done.value.puts.iter().map(|p| p.kind()).collect();
    assert_eq!(kinds, ["contract", "interface", "harness"]);
    assert!(done.value.puts.iter().all(|p| match p {
        PutDone::Asset { revision, .. } => revision.number() == 2,
        PutDone::Context { revision, .. } => revision.number() == 2,
        _ => false,
    }));
    // what the puts dropped, by name
    let gone: Vec<&str> = done.value.retired.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        gone,
        ["stock C7", "stock release", "stock-tests release_frees"]
    );
}

#[tokio::test]
async fn the_same_removal_as_a_single_put_is_refused_naming_the_case_and_the_clause() {
    let mut app = stock("single-puts").await;
    let before = app.store().revision().await.unwrap();
    let err = app
        .put_contract("core", "stock", "Stock", &clauses(&["C1"]))
        .await
        .unwrap_err();
    let text = said(&app, &err).await;
    assert!(
        text.contains("stock-tests release_frees") && text.contains("stock C7"),
        "{text}"
    );
    assert!(text.contains("cannot reach"), "{text}");
    assert_eq!(
        app.store().revision().await.unwrap(),
        before,
        "nothing kept"
    );
}

/// The keys, ids and revisions of the Items of the Interface `stock` of `core`, and the revision of the Interface.
async fn stock_items(app: &App<SqliteStore, Counter>) -> (u64, Vec<(String, Uuid, u64)>) {
    let p = app.store().project().await.unwrap().unwrap();
    let i = p
        .contexts()
        .flat_map(|c| c.interfaces())
        .next()
        .expect("the Interface");
    (
        i.revision().number(),
        i.items()
            .iter()
            .map(|it| {
                (
                    it.key().to_string(),
                    it.id().as_uuid(),
                    it.revision().number(),
                )
            })
            .collect(),
    )
}

/// The one diagnostic of a put of `stock` that is refused, and that nothing was kept.
async fn refused_stock_put(
    name: &str,
    source: &str,
    items: &[ItemInput],
) -> (Diagnostic, App<SqliteStore, Counter>) {
    let mut app = stock(name).await;
    let before = app.store().revision().await.unwrap();
    let err = app
        .put_interface("core", "stock", source, items)
        .await
        .unwrap_err();
    assert_eq!(
        app.store().revision().await.unwrap(),
        before,
        "nothing kept"
    );
    let mut all = app.diagnose(&err).await;
    assert_eq!(all.len(), 1, "{all:#?}");
    (all.remove(0), app)
}

#[tokio::test]
async fn a_rename_alone_keeps_the_id_the_citation_and_every_revision() {
    let mut app = stock("rename").await;
    let (revision, before) = stock_items(&app).await;
    let renamed = ItemInput {
        replaces: Some("reserve".to_string()),
        ..item("Stock::reserve", 1)
    };
    app.put_interface(
        "core",
        "stock",
        "fn reserve()\nfn release()\n",
        &[renamed, item("release", 2)],
    )
    .await
    .unwrap();
    let (now_revision, after) = stock_items(&app).await;
    assert_eq!(now_revision, revision);
    assert_eq!(after[0].0, "Stock::reserve");
    // the same id and revision, so the Case that cites it by id is not stale
    assert_eq!((after[0].1, after[0].2), (before[0].1, before[0].2));
    assert_eq!(after[1], before[1]);
    // the old key is gone, the new one is cited, and the Case still reaches the Item
    app.put_harness(
        "core",
        "stock-tests",
        "fn reserve_refuses() {}\nfn release_frees() {}\n",
        &[
            CaseInput {
                items: vec![ItemRef {
                    context: None,
                    interface: AssetName::try_from("stock").unwrap(),
                    key: "Stock::reserve".to_string(),
                }],
                ..cases(&["reserve"]).remove(0)
            },
            CaseInput {
                lines: (2, 2),
                ..cases(&["release"]).remove(0)
            },
        ],
    )
    .await
    .unwrap();
    let found = app.check(None, &[]).await.unwrap().findings;
    assert!(found.iter().all(|f| f.rule.as_str() != "cite.stale"));
}

#[tokio::test]
async fn a_declaration_that_changes_moves_the_item_and_the_interface_and_a_key_that_changes_alone_does_not()
 {
    let mut app = stock("revise").await;
    let (_, before) = stock_items(&app).await;
    app.put_interface(
        "core",
        "stock",
        "fn reserve(n: u8)\nfn release()\n",
        &[item("reserve", 1), item("release", 2)],
    )
    .await
    .unwrap();
    let (revision, after) = stock_items(&app).await;
    assert_eq!(revision, 2);
    assert_eq!(
        after.iter().map(|i| i.2).collect::<Vec<_>>(),
        [before[0].2 + 1, before[1].2]
    );
    assert_eq!(after[0].1, before[0].1);
}

#[tokio::test]
async fn a_replaces_that_the_put_still_has_is_refused_naming_the_key() {
    let (d, _) = refused_stock_put(
        "replaces-kept",
        "fn reserve()\nfn release()\n",
        &[
            ItemInput {
                replaces: Some("release".to_string()),
                ..item("reserve", 1)
            },
            item("release", 2),
        ],
    )
    .await;
    assert_eq!(d.code, "input.replaces-kept");
    assert_eq!(d.level, "error");
    assert_eq!(d.values["key"], "release");
    let named: Vec<&str> = d.subjects.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(named, ["core", "stock"]);
    assert!(d.message.contains("`release`"), "{}", d.message);
}

#[tokio::test]
async fn a_replaces_to_itself_is_not_a_rename_that_is_kept() {
    let mut app = stock("replaces-itself").await;
    let (_, before) = stock_items(&app).await;
    let same = ItemInput {
        replaces: Some("reserve".to_string()),
        ..item("reserve", 1)
    };
    app.put_interface(
        "core",
        "stock",
        "fn reserve()\nfn release()\n",
        &[same, item("release", 2)],
    )
    .await
    .unwrap();
    assert_eq!(stock_items(&app).await.1, before);
}

#[tokio::test]
async fn two_items_that_replace_one_key_are_refused_naming_the_key() {
    let (d, _) = refused_stock_put(
        "rename-conflict",
        "fn x()\nfn y()\n",
        &[
            ItemInput {
                replaces: Some("reserve".to_string()),
                ..item("x", 1)
            },
            ItemInput {
                replaces: Some("reserve".to_string()),
                ..item("y", 2)
            },
        ],
    )
    .await;
    assert_eq!(d.code, "input.rename-conflict");
    assert_eq!(d.values["key"], "reserve");
    assert!(d.message.contains("`reserve`"), "{}", d.message);
}

#[tokio::test]
async fn a_replaces_of_a_key_the_interface_does_not_have_is_an_unknown_name() {
    let (d, _) = refused_stock_put(
        "replaces-unknown",
        "fn x()\n",
        &[ItemInput {
            replaces: Some("reservee".to_string()),
            ..item("x", 1)
        }],
    )
    .await;
    assert_eq!(d.code, "input.unknown-name");
    assert_eq!(d.values["kind"], "item");
    assert_eq!(d.did_you_mean, ["reserve"]);
}

#[tokio::test]
async fn the_same_existing_key_twice_is_refused_as_a_key_and_not_as_an_id() {
    let (d, _) = refused_stock_put(
        "key-twice",
        "fn reserve()\nfn release()\n",
        &[item("reserve", 1), item("reserve", 2)],
    )
    .await;
    assert_eq!(d.code, "interface.key-taken");
    assert_eq!(d.level, "error");
    assert_eq!(
        d.subjects
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["stock reserve"]
    );
    assert!(d.message.contains("stock reserve"), "{}", d.message);
}

#[tokio::test]
async fn an_item_on_blank_lines_is_refused_naming_it() {
    let (d, _) = refused_stock_put(
        "blank-item",
        "fn reserve()\n\n",
        &[item("reserve", 1), item("release", 2)],
    )
    .await;
    assert_eq!(d.code, "interface.blank-item");
    assert_eq!(
        d.subjects
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["stock release"]
    );
}

#[tokio::test]
async fn a_citation_of_a_key_the_interface_does_not_have_is_an_unknown_name_with_the_close_keys() {
    let mut app = stock("cite-unknown").await;
    let err = app
        .put_harness(
            "core",
            "stock-tests",
            "fn reserve_refuses() {}\n",
            &[CaseInput {
                items: vec![ItemRef {
                    context: None,
                    interface: AssetName::try_from("stock").unwrap(),
                    key: "reserv".to_string(),
                }],
                ..cases(&["reserve"]).remove(0)
            }],
        )
        .await
        .unwrap_err();
    let all = app.diagnose(&err).await;
    assert_eq!(all[0].code, "input.unknown-name");
    assert_eq!(all[0].values["name"], "reserv");
    assert_eq!(all[0].did_you_mean, ["reserve"]);
}

#[tokio::test]
async fn a_refused_change_set_names_every_problem_and_keeps_nothing() {
    let mut app = stock("refused-set").await;
    let before = app.store().revision().await.unwrap();
    let mut set = removal();
    // the Harness is left as it is: the Case that cites C7 and `release` stays
    set.puts.pop();
    let err = app.apply_change_set(&set).await.unwrap_err();
    let AppError::Many(all) = &err else {
        panic!("two problems expected: {err}");
    };
    assert_eq!(all.len(), 2);
    let text = said(&app, &err).await;
    assert!(text.contains("case stock-tests release_frees"), "{text}");
    assert!(text.contains("the item `stock release`"), "{text}");
    assert!(text.contains("the clause `stock C7`"), "{text}");
    assert_eq!(app.store().revision().await.unwrap(), before);
}

#[tokio::test]
async fn a_retirement_and_a_put_are_one_change_set() {
    let mut app = stock("retire-and-put").await;
    let before = app.store().revision().await.unwrap();
    let mut set = removal();
    set.puts.pop();
    set.retires.push(RetireInput::Harness {
        context: "core".to_string(),
        harness: AssetName::try_from("stock-tests").unwrap(),
    });
    let done = app.apply_change_set(&set).await.unwrap();
    assert_eq!(app.store().revision().await.unwrap().0, before.0 + 1);
    assert!(done.value.retired.iter().any(|r| r.kind == "harness"));
    let again = app.retire_harness("core", "stock-tests").await;
    assert!(
        matches!(again, Err(AppError::AssetRetired { .. })),
        "{again:?}"
    );
}

#[tokio::test]
async fn a_change_set_adds_a_context_with_its_first_contract_and_interface() {
    let mut app = app("context-put").await;
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    let project = app.store().project().await.unwrap().unwrap();
    let set = ChangeSetInput {
        change_request: None,
        puts: vec![
            // listed before the Context it uses
            PutInput::Context {
                name: "billing".to_string(),
                uses: vec!["ledger".to_string(), "core".to_string()],
            },
            PutInput::Contract(ContractPut {
                context: "ledger".to_string(),
                name: AssetName::try_from("entries").unwrap(),
                title: "Entries".to_string(),
                clauses: clauses(&["C1"]),
            }),
            PutInput::Interface(InterfacePut {
                context: "billing".to_string(),
                name: AssetName::try_from("invoice").unwrap(),
                source: "fn total()\n".to_string(),
                items: ItemsInput::Written(vec![item("total", 1)]),
            }),
            PutInput::Context {
                name: "ledger".to_string(),
                uses: Vec::new(),
            },
        ],
        retires: Vec::new(),
    };
    let done = app.apply_change_set(&set).await.unwrap();
    let kinds: Vec<&str> = done.value.puts.iter().map(|p| p.kind()).collect();
    assert_eq!(kinds, ["context", "contract", "interface", "context"]);
    let after = app.store().project().await.unwrap().unwrap();
    assert_eq!(after.revision().number(), project.revision().number() + 2);
    assert_eq!(after.contexts().count(), 3);
    let billing = after
        .contexts()
        .find(|c| c.name().to_string() == "billing")
        .unwrap();
    assert_eq!(billing.uses().len(), 2);
    assert_eq!(billing.interfaces().count(), 1);
    let ledger = after
        .contexts()
        .find(|c| c.name().to_string() == "ledger")
        .unwrap();
    assert_eq!(ledger.contracts().count(), 1);

    // a name that is taken is refused, and nothing is kept
    let before = app.store().revision().await.unwrap();
    let again = ChangeSetInput {
        change_request: None,
        puts: vec![PutInput::Context {
            name: "ledger".to_string(),
            uses: Vec::new(),
        }],
        retires: Vec::new(),
    };
    assert!(app.apply_change_set(&again).await.is_err());
    assert_eq!(app.store().revision().await.unwrap(), before);
}

#[tokio::test]
async fn a_pin_with_nothing_that_checks_it_names_the_interface_not_its_id() {
    let mut app = app("pin-name").await;
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_interface("core", "stock", "fn reserve()\n", &[item("reserve", 1)])
        .await
        .unwrap();
    let err = app.pin("core", "stock", "code").await.unwrap_err();
    assert_eq!(
        said(&app, &err).await,
        "no case cites an item of `interface stock`, so nothing checked the code to pin"
    );
    let diagnostics = app.diagnose(&err).await;
    assert_eq!(diagnostics[0].code, "pin.unchecked");
}

mod checks {
    use std::borrow::Cow;

    use chidev_core::app::ports::{Invocation, Outcome, Store};
    use chidev_core::app::{
        App, AppError, ChangeRequestInput, ClauseInput, ClauseRef, CruxKind, FindingInput, Intent,
        OptionInput, RaiseCruxInput, Registry, SubjectInput, TargetInput,
    };
    use chidev_core::domain::check::{
        Check, CheckView, Descriptor, Executor, Finding, FindingState, Level, Moment, Params,
    };
    use chidev_core::domain::vo::finding::{RuleId, Subject};
    use chidev_core::domain::vo::name::{AssetName, ContextName};
    use chidev_core::domain::vo::timestamp::Timestamp;
    use chidev_core::infra::clock::FixedClock;
    use chidev_core::infra::sqlite::SqliteStore;

    use super::{Counter, app};

    /// A mandatory check that finds every Clause of the Project, to try the refusal with.
    struct Forbidden;

    static FORBIDDEN: Descriptor = Descriptor {
        id: RuleId::from_static("test.forbidden"),
        summary: Cow::Borrowed("a clause exists"),
        help: Cow::Borrowed("only for the test"),
        moments: Cow::Borrowed(&[Moment::Edit]),
        default_level: Level::Error,
        mandatory: true,
        params: std::borrow::Cow::Borrowed(&[]),
        executor: Executor::Builtin,
    };

    impl Check for Forbidden {
        fn descriptor(&self) -> &'static Descriptor {
            &FORBIDDEN
        }

        fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
            view.project
                .contexts()
                .flat_map(|c| c.contracts())
                .flat_map(|k| k.clauses())
                .map(|cl| {
                    Finding::new(
                        &FORBIDDEN,
                        Some(Subject::Clause(cl.id())),
                        format!("clause {}", cl.label()),
                    )
                })
                .collect()
        }
    }

    fn clauses() -> Vec<ClauseInput> {
        vec![ClauseInput {
            label: "C1".to_string(),
            text: "push then pop".to_string(),
            replaces: None,
        }]
    }

    async fn invocations(app: &App<SqliteStore, Counter>) -> Vec<Invocation> {
        app.store()
            .invocations()
            .await
            .unwrap()
            .into_iter()
            .map(|(_, run)| run)
            .collect()
    }

    /// The rules of `found`.
    fn rules(found: &[chidev_core::app::Found]) -> Vec<&str> {
        found.iter().map(|f| f.rule.as_str()).collect()
    }

    #[tokio::test]
    async fn a_put_does_not_say_the_model_is_unfinished_but_a_check_of_the_whole_does() {
        let mut app = app("checks-put").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        let put = app
            .put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        // A first Contract has a Clause no Case cites, and a Context with no Interface: not an edit's doing.
        assert!(put.findings.is_empty(), "{:?}", rules(&put.findings));
        let ran = invocations(&app).await;
        for state in [
            "cite.uncited-clause",
            "cite.uncited-item",
            "pin.unpinned",
            "context.no-interface",
            "context.no-harness",
        ] {
            assert!(ran.iter().all(|i| i.check.as_str() != state), "{state}");
        }
        // The checks that run at put are recorded, with nothing to find as ok.
        let stale = ran
            .iter()
            .find(|i| i.check.as_str() == "cite.stale")
            .unwrap();
        assert_eq!(stale.moment, Moment::Edit);
        assert_eq!((stale.warnings, stale.outcome), (0, Outcome::Ok));
        // Where the whole is judged, it is said.
        let found = app.check(None, &[]).await.unwrap().findings;
        let said = rules(&found);
        assert!(said.contains(&"cite.uncited-clause"), "{said:?}");
        assert!(said.contains(&"context.no-interface"), "{said:?}");
        assert!(found.iter().all(|f| f.state == FindingState::Open));
    }

    #[tokio::test]
    async fn a_put_reports_the_stale_citation_it_made_and_not_the_one_that_was_there() {
        let mut app = super::stock("checks-introduced").await;
        let stock = |c1: &str| {
            vec![
                ClauseInput {
                    label: "C1".to_string(),
                    text: c1.to_string(),
                    replaces: None,
                },
                ClauseInput {
                    label: "C7".to_string(),
                    text: "C7 holds".to_string(),
                    replaces: None,
                },
            ]
        };
        // Reworded, the Clause a Case cites moves on: this put made the Case stale.
        let put = app
            .put_contract("core", "stock", "Stock", &stock("C1 holds always"))
            .await
            .unwrap();
        let stale: Vec<_> = put
            .findings
            .iter()
            .filter(|f| f.rule.as_str() == "cite.stale")
            .collect();
        assert_eq!(stale.len(), 1, "{:?}", rules(&put.findings));
        assert_eq!(stale[0].about.as_deref(), Some("case reserve_refuses"));
        assert!(
            !rules(&put.findings).contains(&"cite.uncited-clause"),
            "{:?}",
            rules(&put.findings)
        );
        // A put that has nothing to do with it does not say it again, and the record counts the whole run.
        let before = invocations(&app).await.len();
        let other = app
            .put_contract("core", "extra", "Extra", &super::clauses(&["C9"]))
            .await
            .unwrap();
        assert!(
            !rules(&other.findings).contains(&"cite.stale"),
            "{:?}",
            rules(&other.findings)
        );
        let ran = invocations(&app).await;
        let recorded = ran[before..]
            .iter()
            .find(|i| i.check.as_str() == "cite.stale")
            .unwrap();
        assert_eq!((recorded.warnings, recorded.outcome), (1, Outcome::Found));
        // Where the whole is judged, it is still there.
        assert!(rules(&app.check(None, &[]).await.unwrap().findings).contains(&"cite.stale"));
        // A put of the same Clause again moves the Case's target on again: that is this put's.
        let again = app
            .put_contract("core", "stock", "Stock", &stock("C1 holds, again"))
            .await
            .unwrap();
        assert!(rules(&again.findings).contains(&"cite.stale"));
    }

    #[tokio::test]
    async fn check_and_status_find_without_recording_or_keeping() {
        let mut app = app("checks-read").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        let before = invocations(&app).await.len();
        let revision = chidev_core::app::ports::Store::revision(app.store())
            .await
            .unwrap();
        let found = app.check(None, &[]).await.unwrap().findings;
        assert!(
            found
                .iter()
                .any(|f| f.rule.as_str() == "cite.uncited-clause")
        );
        assert_eq!(app.status().await.unwrap().findings, found);
        assert_eq!(invocations(&app).await.len(), before);
        assert_eq!(
            chidev_core::app::ports::Store::revision(app.store())
                .await
                .unwrap(),
            revision
        );
    }

    #[tokio::test]
    async fn a_check_of_one_context_returns_what_is_about_it_and_counts_the_rest() {
        let mut app = app("checks-context").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.add_context("billing", &[]).await.unwrap();
        app.put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        app.put_contract("billing", "bill", "Bill", &clauses())
            .await
            .unwrap();
        let clause = |context: &str, contract: &str| {
            TargetInput::Clause(ClauseRef {
                context: Some(ContextName::try_from(context).unwrap()),
                contract: AssetName::try_from(contract).unwrap(),
                label: "C1".to_string(),
            })
        };
        let open = |title: &str,
                    intent: Intent,
                    targets: Vec<TargetInput>,
                    keep: Vec<TargetInput>| ChangeRequestInput {
            title: title.to_string(),
            situation: None,
            intent,
            parent: None,
            issuer: None,
            targets,
            keep,
            assumptions: Vec::new(),
            addresses: Vec::new(),
        };
        let option = |choice: &str, targets: Vec<TargetInput>| OptionInput {
            choice: choice.to_string(),
            change: None,
            targets,
            keep: Vec::new(),
            consequence: None,
            recommended: None,
        };
        // One that revises a Clause of core (and says it keeps the behaviour), one that keeps a Clause of billing
        // (and names no target to change), one whose question only has an option that names a Clause of core, and
        // one that names nothing.
        for c in [
            open(
                "Keeps wrongly",
                Intent::KeepBehavior,
                vec![clause("core", "stack")],
                Vec::new(),
            ),
            open(
                "Keeps billing",
                Intent::ChangeBehavior,
                Vec::new(),
                vec![clause("billing", "bill")],
            ),
            open("Undecided", Intent::ChangeBehavior, Vec::new(), Vec::new()),
            open(
                "Names nothing",
                Intent::ChangeBehavior,
                Vec::new(),
                Vec::new(),
            ),
        ] {
            app.open_change_request(&c).await.unwrap();
        }
        app.raise_crux(&RaiseCruxInput {
            cr: "Undecided".to_string(),
            kind: CruxKind::Ambiguity,
            question: "Reword it or leave it?".to_string(),
            findings: Vec::new(),
            options: vec![
                option("reword", vec![clause("core", "stack")]),
                option("leave", Vec::new()),
            ],
            importance: None,
            supersedes: None,
        })
        .await
        .unwrap();
        let all = app.check(None, &[]).await.unwrap();
        assert_eq!(all.outside, 0);
        let abouts = |found: &[chidev_core::app::Found], rule: &str| -> Vec<String> {
            found
                .iter()
                .filter(|f| f.rule.as_str() == rule)
                .filter_map(|f| f.about.clone())
                .collect()
        };
        assert_eq!(
            abouts(&all.findings, "cr.awaiting-decision"),
            ["change request `Undecided`"]
        );
        for (context, other, request) in [
            ("core", "billing", "Keeps wrongly"),
            ("billing", "core", "Keeps billing"),
        ] {
            let one = app.check(Some(context), &[]).await.unwrap();
            let mut said = rules(&one.findings);
            said.sort_unstable();
            assert_eq!(
                said,
                [
                    "cite.uncited-clause",
                    "context.no-harness",
                    "context.no-interface",
                    "cr.intent-mismatch"
                ],
                "{context}"
            );
            // The request is there for the Clause it names or keeps in the Context, and the other Context's things
            // and the requests that name nothing of it are counted, not returned.
            assert_eq!(
                abouts(&one.findings, "cr.intent-mismatch"),
                [format!("change request `{request}`")]
            );
            assert_eq!(one.outside, all.findings.len() - 4, "{context} / {other}");
            assert!(
                one.findings
                    .iter()
                    .filter_map(|f| f.about.as_deref())
                    .all(|about| !about.contains(other)),
                "{context}: {:?}",
                one.findings
            );
        }
        // A name that is no Context's is refused.
        assert_eq!(
            app.check(Some("nowhere"), &[]).await,
            Err(AppError::UnknownContext(
                ContextName::try_from("nowhere").unwrap()
            ))
        );
    }

    #[tokio::test]
    async fn a_change_request_that_addresses_a_finding_turns_it_addressed_and_then_decided() {
        let mut app = app("checks-addressed").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        let open = |title: &str| ChangeRequestInput {
            title: title.to_string(),
            situation: None,
            intent: Intent::KeepBehavior,
            parent: None,
            issuer: None,
            targets: Vec::new(),
            keep: Vec::new(),
            assumptions: Vec::new(),
            addresses: vec![FindingInput {
                rule: "cite.uncited-clause".to_string(),
                subject: Some(SubjectInput::Clause(ClauseRef {
                    context: Some(ContextName::try_from("core").unwrap()),
                    contract: AssetName::try_from("stack").unwrap(),
                    label: "C1".to_string(),
                })),
            }],
        };
        let state = |found: &[chidev_core::app::Found]| {
            found
                .iter()
                .find(|f| f.rule.as_str() == "cite.uncited-clause")
                .map(|f| f.state)
        };
        assert_eq!(
            state(&app.check(None, &[]).await.unwrap().findings),
            Some(FindingState::Open)
        );
        app.open_change_request(&open("leave C1 unchecked"))
            .await
            .unwrap();
        assert_eq!(
            state(&app.check(None, &[]).await.unwrap().findings),
            Some(FindingState::Addressed)
        );
        app.resolve_change_request(
            "leave C1 unchecked",
            &chidev_core::app::ResolutionInput::Done,
            "settled",
            &chidev_core::app::DecisionMakerInput::Person("alice".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(
            state(&app.check(None, &[]).await.unwrap().findings),
            Some(FindingState::Decided)
        );
        // Naming a rule that is no rule id is refused.
        let mut bad = open("bad");
        bad.addresses[0].rule = "Not A Rule".to_string();
        assert!(matches!(
            app.open_change_request(&bad).await,
            Err(AppError::Input(_))
        ));
    }

    /// An App over the store at `name`, with the checks `registry` (the builtin ones when none), that gives ids
    /// from `start` on.
    async fn app_over(
        name: &str,
        start: u128,
        registry: Option<Registry>,
    ) -> App<SqliteStore, Counter> {
        let dir = std::env::temp_dir().join(format!("chidev-app-{name}-{}", std::process::id()));
        let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
        let clock = FixedClock(Timestamp::from_millis(1_000));
        match registry {
            Some(registry) => App::with_registry(store, Counter(start), registry, clock),
            None => App::new(store, Counter(start), clock),
        }
    }

    #[tokio::test]
    async fn only_what_a_put_introduced_refuses_it_not_what_was_there() {
        let name = "refuse-introduced";
        let dir = std::env::temp_dir().join(format!("chidev-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // The Project as it was kept before the check existed: a Clause the check forbids.
        let mut plain = app_over(name, 0, None).await;
        plain.init().await.unwrap();
        plain.add_context("core", &[]).await.unwrap();
        plain
            .put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        plain.into_store().close().await.unwrap();
        let mut app = app_over(
            name,
            1_000,
            Some(Registry::new(&[&Forbidden], &[]).unwrap()),
        )
        .await;
        // A put that leaves that Clause as it is introduces nothing, and is kept.
        let kept = app
            .put_interface("core", "api", "fn a()\n", &[super::item("a", 1)])
            .await
            .unwrap();
        assert!(kept.findings.is_empty(), "{:?}", rules(&kept.findings));
        // A put that makes another one is refused, naming only that one.
        let revision = chidev_core::app::ports::Store::revision(app.store())
            .await
            .unwrap();
        let refused = app
            .put_contract("core", "more", "More", &super::clauses(&["C2"]))
            .await;
        let Err(AppError::Refused(found)) = refused else {
            panic!("expected a refusal, got {refused:?}");
        };
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].about.as_deref(), Some("more C2"));
        assert_eq!(
            chidev_core::app::ports::Store::revision(app.store())
                .await
                .unwrap(),
            revision
        );
    }

    #[tokio::test]
    async fn a_mandatory_check_at_error_refuses_the_use_case_and_nothing_is_kept() {
        let dir = std::env::temp_dir().join(format!("chidev-app-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
        let registry = Registry::new(&[&Forbidden], &[]).unwrap();
        let mut app = App::with_registry(
            store,
            Counter::default(),
            registry,
            FixedClock(Timestamp::from_millis(1_000)),
        );
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        // Adding the Context was an edit, and the check ran on it; the refused put leaves that record as it was.
        let kept = invocations(&app).await.len();
        let refused = app.put_contract("core", "stack", "Stack", &clauses()).await;
        let Err(AppError::Refused(found)) = refused else {
            panic!("expected a refusal, got {refused:?}");
        };
        assert_eq!(found[0].rule.as_str(), "test.forbidden");
        assert_eq!(found[0].level, Level::Error);
        use chidev_core::app::ports::AssetKind;
        use chidev_core::app::ports::AssetNames;
        let ctx = chidev_core::domain::repository::ProjectRepository::project(app.store())
            .await
            .unwrap()
            .unwrap()
            .contexts()
            .next()
            .unwrap()
            .id();
        assert_eq!(
            app.store()
                .asset_id(
                    ctx,
                    AssetKind::Contract,
                    &AssetName::try_from("stack").unwrap()
                )
                .await
                .unwrap(),
            None
        );
        assert_eq!(invocations(&app).await.len(), kept);
    }
}

mod judged {
    use chidev_core::app::ports::{Outcome, Store};
    use chidev_core::app::{
        App, AppError, ChangeRequestInput, ClauseInput, ClauseRef, FindingInput, Intent,
        JudgedState, Registry, ReportInput, ReportedFinding, SubjectInput,
    };
    use chidev_core::domain::check::{Executor, FindingState, Level, Moment};
    use chidev_core::domain::vo::name::{AssetName, ContextName};
    use chidev_core::domain::vo::timestamp::Timestamp;
    use chidev_core::infra::clock::FixedClock;
    use chidev_core::infra::judged::parse;
    use chidev_core::infra::sqlite::SqliteStore;

    use super::Counter;

    fn rule(id: &str, subject: &str, level: &str) -> String {
        format!(
            "id = \"{id}\"\nsummary = \"s\"\nsubject = \"{subject}\"\nmoments = [\"edit\"]\nlevel = \"{level}\"\n\
             question = \"q\"\ntrue = \"t\"\nfalse = \"f\"\nfix = \"x\"\n"
        )
    }

    /// An App with the judged rules `texts`, a Context `core` and a Contract `stack` with clauses C1 and C2.
    async fn app_with(name: &str, texts: &[String]) -> App<SqliteStore, Counter> {
        let dir = std::env::temp_dir().join(format!("chidev-judged-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
        let rules = texts.iter().map(|t| parse(t).unwrap()).collect();
        let registry = Registry::builtin_with_judged(rules, &[]).unwrap();
        let mut app = App::with_registry(
            store,
            Counter::default(),
            registry,
            FixedClock(Timestamp::from_millis(1_000)),
        );
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        put_stack(&mut app, "push then pop", "empty pops nothing").await;
        app
    }

    async fn app(name: &str) -> App<SqliteStore, Counter> {
        app_with(name, &[rule("judged.vague", "clause", "warning")]).await
    }

    async fn put_stack(app: &mut App<SqliteStore, Counter>, c1: &str, c2: &str) {
        let clause = |label: &str, text: &str| ClauseInput {
            label: label.to_string(),
            text: text.to_string(),
            replaces: None,
        };
        app.put_contract(
            "core",
            "stack",
            "Stack",
            &[clause("C1", c1), clause("C2", c2)],
        )
        .await
        .unwrap();
    }

    fn clause(label: &str) -> Option<SubjectInput> {
        Some(SubjectInput::Clause(ClauseRef {
            context: Some(ContextName::try_from("core").unwrap()),
            contract: AssetName::try_from("stack").unwrap(),
            label: label.to_string(),
        }))
    }

    fn finding(subject: Option<SubjectInput>, message: &str) -> ReportedFinding {
        ReportedFinding {
            subject,
            message: message.to_string(),
            level: None,
        }
    }

    /// Own the versions before mutating the App, as an agent does after reading check_rules.
    fn reporter(
        app: &App<SqliteStore, Counter>,
    ) -> impl Fn(&str, u64, Vec<ReportedFinding>) -> ReportInput + use<> {
        let versions: std::collections::BTreeMap<String, String> = app
            .registry()
            .judged()
            .iter()
            .map(|r| (r.id().to_string(), r.digest().to_string()))
            .collect();
        move |rule, read_at, findings| ReportInput {
            rule: rule.to_string(),
            rule_digest: versions.get(rule).cloned().unwrap_or_default(),
            judge: "agent-1".to_string(),
            read_at,
            findings,
        }
    }

    async fn now(app: &App<SqliteStore, Counter>) -> u64 {
        app.store().revision().await.unwrap().0
    }

    fn judged_findings(found: &[chidev_core::app::Found]) -> Vec<&chidev_core::app::Found> {
        found
            .iter()
            .filter(|f| f.rule.as_str().starts_with("judged."))
            .collect()
    }

    #[tokio::test]
    async fn the_rules_and_a_view_of_the_model_are_given_to_the_agent() {
        let app = app_with(
            "rules",
            &[
                rule("judged.vague", "clause", "warning"),
                rule("judged.quiet", "context", "off"),
                rule("judged.later", "clause", "note").replace("edit", "check"),
            ],
        )
        .await;
        let view = app.check_rules(None, None).await.unwrap();
        let ids: Vec<&str> = view.rules.iter().map(|r| r.rule.id().as_str()).collect();
        // the rule the policy default turned off is left out
        assert_eq!(ids, ["judged.vague", "judged.later"]);
        assert!(view.rules.iter().all(|r| r.state == JudgedState::Never));
        assert_eq!(view.store_revision.0, now(&app).await);
        assert_eq!(view.model.store_revision(), view.store_revision.0);
        let json = serde_json::to_value(&view.model).unwrap();
        assert_eq!(json["version"], 5);
        let clauses = &json["contexts"][0]["contracts"][0]["clauses"];
        assert_eq!(json["contexts"][0]["contracts"][0]["name"], "stack");
        assert_eq!(clauses[0]["label"], "C1");
        assert_eq!(clauses[0]["text"], "push then pop");
        assert_eq!(clauses[0]["revision"], 1);

        let at_put = app.check_rules(Some(Moment::Edit), None).await.unwrap();
        assert_eq!(at_put.rules.len(), 1);
        let one = app.check_rules(None, Some("judged.later")).await.unwrap();
        assert_eq!(one.rules[0].level, Level::Note);
        assert_eq!(
            app.check_rules(None, Some("judged.quiet"))
                .await
                .map(|_| ()),
            Err(AppError::RuleOff("judged.quiet".to_string()))
        );
        assert!(matches!(
            app.check_rules(None, Some("judged.none")).await,
            Err(AppError::UnknownRule(_))
        ));
        assert!(matches!(
            app.check_rules(None, Some("pin.stale")).await,
            Err(AppError::NotJudged { .. })
        ));
        // asking changes nothing
        let revision = now(&app).await;
        app.check_rules(None, None).await.unwrap();
        assert_eq!(now(&app).await, revision);
    }

    #[tokio::test]
    async fn a_report_that_cannot_be_kept_is_refused_and_keeps_nothing() {
        async fn refuse(app: &mut App<SqliteStore, Counter>, r: ReportInput) -> AppError {
            let before = app.store().revision().await.unwrap();
            let e = app.finding_report(&r).await.unwrap_err();
            assert_eq!(app.store().revision().await.unwrap(), before);
            e
        }

        let mut app = app("refused").await;
        let report = reporter(&app);
        let at = now(&app).await;
        let ok = || vec![finding(clause("C1"), "vague")];
        assert!(matches!(
            refuse(&mut app, report("judged.none", at, ok())).await,
            AppError::UnknownRule(_)
        ));
        assert!(matches!(
            refuse(&mut app, report("cite.stale", at, ok())).await,
            AppError::NotJudged {
                executor: "builtin",
                ..
            }
        ));
        // a context where a clause is judged, a project finding, a missing clause, a clause given twice
        let wrong = vec![finding(
            Some(SubjectInput::Context(
                ContextName::try_from("core").unwrap(),
            )),
            "m",
        )];
        assert!(matches!(
            refuse(&mut app, report("judged.vague", at, wrong)).await,
            AppError::Input(m) if m.contains("judges clause findings")
        ));
        assert!(matches!(
            refuse(
                &mut app,
                report("judged.vague", at, vec![finding(None, "m")])
            )
            .await,
            AppError::Input(_)
        ));
        assert!(matches!(
            refuse(
                &mut app,
                report("judged.vague", at, vec![finding(clause("C9"), "m")])
            )
            .await,
            AppError::UnknownClause { .. }
        ));
        let twice = vec![finding(clause("C1"), "a"), finding(clause("C1"), "b")];
        assert!(matches!(
            refuse(&mut app, report("judged.vague", at, twice)).await,
            AppError::Input(m) if m.contains("two findings")
        ));
        // levels: above the rule's, and off
        for (level, expect) in [
            (Level::Error, "above the warning"),
            (Level::Off, "leave it out"),
        ] {
            let mut f = finding(clause("C1"), "m");
            f.level = Some(level);
            assert!(matches!(
                refuse(&mut app, report("judged.vague", at, vec![f])).await,
                AppError::Input(m) if m.contains(expect)
            ));
        }
        // a store revision that does not exist yet, no judge, no message
        assert!(matches!(
            refuse(&mut app, report("judged.vague", at + 5, ok())).await,
            AppError::Input(m) if m.contains("read_at")
        ));
        let mut nameless = report("judged.vague", at, ok());
        nameless.judge = " ".to_string();
        assert!(matches!(
            refuse(&mut app, nameless).await,
            AppError::Input(_)
        ));
        assert!(matches!(
            refuse(
                &mut app,
                report("judged.vague", at, vec![finding(clause("C1"), " ")])
            )
            .await,
            AppError::Input(_)
        ));
        assert!(
            app.check(None, &[])
                .await
                .unwrap()
                .findings
                .iter()
                .all(|f| !f.rule.as_str().starts_with("judged."))
        );
    }

    #[tokio::test]
    async fn a_report_replaces_every_earlier_finding_of_its_rule_and_is_recorded() {
        let mut app = app_with(
            "replace",
            &[
                rule("judged.vague", "clause", "warning"),
                rule("judged.other", "clause", "note"),
            ],
        )
        .await;
        let report = reporter(&app);
        let at = now(&app).await;
        let done = app
            .finding_report(&report(
                "judged.vague",
                at,
                vec![
                    finding(clause("C1"), "vague"),
                    finding(clause("C2"), "vague too"),
                ],
            ))
            .await
            .unwrap();
        assert_eq!(
            (done.findings, done.replaced, done.on_older_revision),
            (2, 0, false)
        );
        app.finding_report(&report(
            "judged.other",
            now(&app).await,
            vec![finding(clause("C1"), "o")],
        ))
        .await
        .unwrap();
        let found = app.check(None, &[]).await.unwrap().findings;
        let mine = judged_findings(&found);
        assert_eq!(mine.len(), 3);
        let vague: Vec<_> = mine
            .iter()
            .filter(|f| f.rule.as_str() == "judged.vague")
            .collect();
        assert_eq!(vague[0].level, Level::Warning);
        assert_eq!(vague[0].state, FindingState::Open);
        assert_eq!(vague[0].about.as_deref(), Some("stack C1"));
        assert!(!vague[0].stale);
        assert_eq!(vague[0].judgment.as_ref().unwrap().judge, "agent-1");

        // judged again: one finding, at a lower level; the other rule's finding stays
        let mut lower = finding(clause("C2"), "only this");
        lower.level = Some(Level::Note);
        let done = app
            .finding_report(&report("judged.vague", now(&app).await, vec![lower]))
            .await
            .unwrap();
        assert_eq!((done.findings, done.replaced), (1, 2));
        let found = app.check(None, &[]).await.unwrap().findings;
        let vague: Vec<_> = judged_findings(&found)
            .into_iter()
            .filter(|f| f.rule.as_str() == "judged.vague")
            .collect();
        assert_eq!(vague.len(), 1);
        assert_eq!(
            (vague[0].message.as_str(), vague[0].level),
            ("only this", Level::Note)
        );
        assert_eq!(judged_findings(&found).len(), 2);

        // judged again, and nothing found: the rule has no finding left, and is current, not never judged
        app.finding_report(&report("judged.vague", now(&app).await, Vec::new()))
            .await
            .unwrap();
        let found = app.check(None, &[]).await.unwrap().findings;
        assert_eq!(judged_findings(&found).len(), 1);
        let view = app.check_rules(None, Some("judged.vague")).await.unwrap();
        assert_eq!(view.rules[0].state, JudgedState::Current);

        // each report was recorded as a run of a judged rule, by the judge
        let ran: Vec<_> = app
            .store()
            .invocations()
            .await
            .unwrap()
            .into_iter()
            .map(|(_, run)| run)
            .collect();
        let reports: Vec<_> = ran
            .iter()
            .filter(|i| i.executor == Executor::Judged)
            .collect();
        assert_eq!(reports.len(), 4);
        assert_eq!(reports[0].check.as_str(), "judged.vague");
        assert_eq!(
            (reports[0].warnings, reports[0].outcome),
            (2, Outcome::Found)
        );
        assert_eq!(
            (reports[2].notes, reports[2].judge.as_deref()),
            (1, Some("agent-1"))
        );
        assert_eq!(reports[3].outcome, Outcome::Ok);
    }

    #[tokio::test]
    async fn a_finding_is_stale_once_its_subject_changed_and_a_rule_once_what_it_judges_did() {
        let mut app = app("stale").await;
        let report = reporter(&app);
        let at = now(&app).await;
        app.finding_report(&report(
            "judged.vague",
            at,
            vec![finding(clause("C1"), "vague")],
        ))
        .await
        .unwrap();
        let stale_of =
            |found: &[chidev_core::app::Found]| judged_findings(found).first().map(|f| f.stale);
        assert_eq!(
            stale_of(&app.check(None, &[]).await.unwrap().findings),
            Some(false)
        );
        assert!(app.status().await.unwrap().judged.is_empty());

        // another clause changes: the rule's part of the model moved, so it is to be judged again, but the
        // finding about C1 stands
        put_stack(&mut app, "push then pop", "an empty stack pops nothing").await;
        assert_eq!(
            stale_of(&app.check(None, &[]).await.unwrap().findings),
            Some(false)
        );
        let pending = app.status().await.unwrap().judged;
        assert_eq!(pending.len(), 1);
        assert_eq!(
            (pending[0].rule.as_str(), pending[0].state),
            ("judged.vague", JudgedState::Stale)
        );

        // its own clause changes: the finding is stale, and still shown
        put_stack(
            &mut app,
            "push then pop, in order",
            "an empty stack pops nothing",
        )
        .await;
        let found = app.check(None, &[]).await.unwrap().findings;
        assert_eq!(stale_of(&found), Some(true));
        assert_eq!(app.status().await.unwrap().findings, found);

        // judged again on the view as it is: current
        let read = now(&app).await;
        app.finding_report(&report(
            "judged.vague",
            read,
            vec![finding(clause("C1"), "still vague")],
        ))
        .await
        .unwrap();
        assert_eq!(
            stale_of(&app.check(None, &[]).await.unwrap().findings),
            Some(false)
        );
        assert!(app.status().await.unwrap().judged.is_empty());
    }

    #[tokio::test]
    async fn a_report_on_a_view_older_than_the_model_is_kept_and_marked_stale_but_a_report_is_not_a_change()
     {
        let mut app = app_with(
            "older",
            &[
                rule("judged.vague", "clause", "warning"),
                rule("judged.other", "clause", "warning"),
            ],
        )
        .await;
        let report = reporter(&app);
        let read = now(&app).await;
        // two reports on the same view: the first is a commit, but it does not change the model
        for id in ["judged.vague", "judged.other"] {
            let done = app
                .finding_report(&report(id, read, vec![finding(clause("C1"), "m")]))
                .await
                .unwrap();
            assert!(!done.on_older_revision, "{id}");
        }
        // the model moves after the agent read the view
        put_stack(&mut app, "push then pop, in order", "x").await;
        let done = app
            .finding_report(&report(
                "judged.vague",
                read,
                vec![finding(clause("C1"), "m")],
            ))
            .await
            .unwrap();
        assert!(done.on_older_revision);
        let found = app.check(None, &[]).await.unwrap().findings;
        let vague = judged_findings(&found)
            .into_iter()
            .find(|f| f.rule.as_str() == "judged.vague")
            .unwrap();
        assert!(vague.stale && vague.judgment.as_ref().unwrap().on_older_revision);
        let rules = app.check_rules(None, Some("judged.vague")).await.unwrap();
        assert_eq!(rules.rules[0].state, JudgedState::Stale);
    }

    #[tokio::test]
    async fn a_change_request_can_address_a_judged_finding() {
        let mut app = app("addressed").await;
        let report = reporter(&app);
        let at = now(&app).await;
        app.finding_report(&report(
            "judged.vague",
            at,
            vec![finding(clause("C1"), "vague")],
        ))
        .await
        .unwrap();
        let state = |found: &[chidev_core::app::Found]| judged_findings(found)[0].state;
        assert_eq!(
            state(&app.check(None, &[]).await.unwrap().findings),
            FindingState::Open
        );
        app.open_change_request(&ChangeRequestInput {
            title: "reword C1".to_string(),
            situation: None,
            intent: Intent::ChangeBehavior,
            parent: None,
            issuer: None,
            targets: Vec::new(),
            keep: Vec::new(),
            assumptions: Vec::new(),
            addresses: vec![FindingInput {
                rule: "judged.vague".to_string(),
                subject: Some(SubjectInput::Clause(ClauseRef {
                    context: Some(ContextName::try_from("core").unwrap()),
                    contract: AssetName::try_from("stack").unwrap(),
                    label: "C1".to_string(),
                })),
            }],
        })
        .await
        .unwrap();
        assert_eq!(
            state(&app.check(None, &[]).await.unwrap().findings),
            FindingState::Addressed
        );
    }

    #[tokio::test]
    async fn a_stale_pin_names_what_moved() {
        let mut app = app("pin-names").await;
        app.put_interface(
            "core",
            "stack-api",
            "fn push(v: u8)\n",
            &[chidev_core::app::ItemInput {
                key: "push".to_string(),
                lines: (1, 1),
                replaces: None,
            }],
        )
        .await
        .unwrap();
        let cites = chidev_core::app::CaseInput {
            name: "pushes".to_string(),
            lines: (1, 1),
            items: vec![chidev_core::app::ItemRef {
                context: Some(ContextName::try_from("core").unwrap()),
                interface: AssetName::try_from("stack-api").unwrap(),
                key: "push".to_string(),
            }],
            clauses: vec![ClauseRef {
                context: Some(ContextName::try_from("core").unwrap()),
                contract: AssetName::try_from("stack").unwrap(),
                label: "C1".to_string(),
            }],
        };
        app.put_harness("core", "stack-test", "fn t() {}\n", &[cites])
            .await
            .unwrap();
        app.pin("core", "stack-api", "code").await.unwrap();
        put_stack(&mut app, "push then pop, in order", "x").await;
        let found = app.check(None, &[]).await.unwrap().findings;
        let pin = found
            .iter()
            .find(|f| f.rule.as_str() == "pin.stale")
            .unwrap();
        assert_eq!(
            pin.message,
            "the code was pinned against what has moved: contract `stack` changed"
        );
    }
}

mod diagnostics {
    //! A refusal as diagnostics: the names the store has, the names that are close to one that does not exist, and
    //! the operations that would mend it.

    use chidev_core::app::{
        App, ChangeRequestInput, ChangeSetInput, ClauseRef, DecisionMakerInput, Diagnostic, Intent,
        Mend, PutInput, ResolutionInput, SuggestedOp, TargetInput,
    };
    use chidev_core::domain::violation::Applicability;
    use chidev_core::domain::vo::name::{AssetName, ContextName};
    use chidev_core::infra::sqlite::SqliteStore;

    use super::{Counter, app, clauses};

    fn open(title: &str, parent: Option<&str>) -> ChangeRequestInput {
        ChangeRequestInput {
            title: title.to_string(),
            situation: None,
            intent: Intent::ChangeBehavior,
            parent: parent.map(ToString::to_string),
            issuer: None,
            targets: Vec::new(),
            keep: Vec::new(),
            assumptions: Vec::new(),
            addresses: Vec::new(),
        }
    }

    fn alice() -> DecisionMakerInput {
        DecisionMakerInput::Person("alice".to_string())
    }

    /// A project with the Context `core` and the Contract `stock`, in a database of its own.
    async fn project() -> App<SqliteStore, Counter> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut app = app(&format!("diagnostics-{n}")).await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.put_contract("core", "stock", "Stock", &clauses(&["C1"]))
            .await
            .unwrap();
        app
    }

    async fn only(app: &App<SqliteStore, Counter>, err: chidev_core::app::AppError) -> Diagnostic {
        let mut all = app.diagnose(&err).await;
        assert_eq!(all.len(), 1, "{all:#?}");
        all.remove(0)
    }

    #[tokio::test]
    async fn a_change_request_title_one_letter_off_has_the_right_one_suggested() {
        let mut app = project().await;
        app.open_change_request(&open("Round halves up", None))
            .await
            .unwrap();
        let err = app
            .resolve_change_request("Round halfs up", &ResolutionInput::Done, "r", &alice())
            .await
            .unwrap_err();
        let d = only(&app, err).await;
        assert_eq!(d.code, "input.unknown-name");
        assert_eq!(d.did_you_mean, ["Round halves up"]);
        assert_eq!(d.values["kind"], "change request");
        assert_eq!(d.values["name"], "Round halfs up");
        assert_eq!(d.message, "no change request is named `Round halfs up`");
    }

    #[tokio::test]
    async fn a_change_set_for_an_unopened_change_request_suggests_opening_it() {
        let mut app = project().await;
        app.open_change_request(&open("Something else entirely", None))
            .await
            .unwrap();
        let set = ChangeSetInput {
            change_request: Some("Not opened".to_string()),
            puts: vec![PutInput::Context {
                name: "extra".to_string(),
                uses: Vec::new(),
            }],
            retires: Vec::new(),
        };
        let err = app.apply_change_set(&set).await.unwrap_err();
        let d = only(&app, err).await;
        assert_eq!(d.code, "input.unknown-name");
        assert!(d.did_you_mean.is_empty());
        assert_eq!(d.suggestions.len(), 1);
        assert_eq!(
            d.suggestions[0].mend,
            Mend::Op(SuggestedOp::OpenChangeRequest {
                title: Some("Not opened".to_string())
            })
        );
        assert_eq!(d.suggestions[0].applicability, Applicability::Review);
    }

    #[tokio::test]
    async fn a_name_that_is_not_a_context_clause_or_asset_has_the_names_that_exist() {
        let mut app = project().await;
        let err = app
            .put_contract("cor", "stock", "Stock", &clauses(&["C1"]))
            .await
            .unwrap_err();
        let d = only(&app, err).await;
        assert_eq!(d.code, "input.unknown-name");
        assert_eq!(d.did_you_mean, ["core"]);
        assert_eq!(
            d.suggestions[0].mend,
            Mend::Op(SuggestedOp::AddContext {
                name: Some("cor".to_string())
            })
        );

        let err = app.status().await.and_then(|_| {
            Err::<(), _>(chidev_core::app::AppError::UnknownAsset {
                context: ContextName::try_from("core").unwrap(),
                kind: "contract",
                name: AssetName::try_from("stok").unwrap(),
            })
        });
        let d = only(&app, err.unwrap_err()).await;
        assert_eq!(d.did_you_mean, ["stock"]);
        assert_eq!(d.subjects[0].name, "core");

        let target = TargetInput::Clause(ClauseRef {
            context: Some(ContextName::try_from("core").unwrap()),
            contract: AssetName::try_from("stock").unwrap(),
            label: "C2".to_string(),
        });
        let mut wants = open("Needs a clause", None);
        wants.targets.push(target);
        let err = app.open_change_request(&wants).await.unwrap_err();
        let d = only(&app, err).await;
        assert_eq!(d.values["kind"], "clause");
        assert_eq!(d.did_you_mean, ["C1"]);
    }

    #[tokio::test]
    async fn a_parent_with_open_parts_is_refused_naming_the_parts_and_how_to_resolve_them() {
        let mut app = project().await;
        app.open_change_request(&open("Parent", None))
            .await
            .unwrap();
        app.open_change_request(&open("Part one", Some("Parent")))
            .await
            .unwrap();
        let err = app
            .resolve_change_request("Parent", &ResolutionInput::Done, "r", &alice())
            .await
            .unwrap_err();
        let d = only(&app, err).await;
        assert_eq!(d.code, "cr.parent-open");
        assert_eq!(
            d.message,
            "the change request `Parent` has 1 part(s) that are not resolved: `Part one`"
        );
        assert_eq!(d.values["count"], 1);
        assert_eq!(
            d.suggestions[0].mend,
            Mend::Op(SuggestedOp::ResolveDone {
                title: "Part one".to_string()
            })
        );
        assert_eq!(d.suggestions[0].applicability, Applicability::Placeholder);
    }

    #[tokio::test]
    async fn a_resolved_change_request_is_refused_and_reopening_is_suggested() {
        let mut app = project().await;
        app.open_change_request(&open("Done already", None))
            .await
            .unwrap();
        app.resolve_change_request("Done already", &ResolutionInput::Done, "r", &alice())
            .await
            .unwrap();
        let err = app
            .resolve_change_request("Done already", &ResolutionInput::Done, "r", &alice())
            .await
            .unwrap_err();
        let d = only(&app, err).await;
        assert_eq!(d.code, "cr.already-resolved");
        assert_eq!(d.subjects[0].name, "Done already");
        assert_eq!(
            d.suggestions[0].mend,
            Mend::Op(SuggestedOp::ReopenChangeRequest {
                title: "Done already".to_string()
            })
        );
    }

    #[tokio::test]
    async fn a_refusal_without_a_rule_yet_is_still_a_diagnostic_and_says_what_it_was() {
        let mut app = project().await;
        let err = app
            .put_contract("core", "dup", "Dup", &clauses(&["C1", "C1"]))
            .await
            .unwrap_err();
        let d = only(&app, err).await;
        assert_eq!(d.code, "domain.unmapped");
        assert!(d.message.contains("twice"), "{}", d.message);
        assert!(
            d.values["error"]
                .as_str()
                .unwrap()
                .contains("DuplicateLabel")
        );
    }

    #[tokio::test]
    async fn the_rules_are_listed_and_each_is_explained() {
        let app = project().await;
        let all = app.rules();
        assert!(
            all.iter()
                .any(|r| r.id == "cr.parent-open" && r.kind == "invariant")
        );
        assert!(
            all.iter()
                .any(|r| r.id == "input.unknown-name" && r.kind == "input")
        );
        assert!(
            all.iter()
                .any(|r| r.id == "cite.stale" && r.kind == "check")
        );
        let one = app.explain("cr.parent-open").unwrap();
        assert_eq!(one.values, ["count"]);
        assert!(one.example.is_some());
        assert!(one.help.contains("Resolve each part"));
        let check = app.explain("cite.stale").unwrap();
        assert_eq!(check.kind, "check");
        assert_eq!(check.executor, Some("builtin"));
        let err = app.explain("cr.parent-opn").unwrap_err();
        let d = app.diagnose(&err).await;
        assert_eq!(d[0].did_you_mean, ["cr.parent-open"]);
    }
}

/// An example Case named `name`, whose expected output is `expected`, citing the Clause `label` of `stock`.
fn example_case(name: &str, expected: &str, label: &str) -> ExampleInput {
    ExampleInput {
        name: name.to_string(),
        clauses: vec![ClauseRef {
            context: None,
            contract: AssetName::try_from("stock").unwrap(),
            label: label.to_string(),
        }],
        example: ExampleTexts {
            args: "-n 2".to_string(),
            stdin: "in\n".to_string(),
            expected: expected.to_string(),
            stderr: None,
        },
    }
}

/// The revisions of the Harness `name` of `core` and of its Cases, and its kind.
async fn harness_state(
    app: &App<SqliteStore, Counter>,
    name: &str,
) -> (&'static str, u64, Vec<u64>) {
    let p = app.store().project().await.unwrap().unwrap();
    let ctx = p
        .contexts()
        .find(|c| c.name().to_string() == "core")
        .unwrap();
    let id = app
        .store()
        .asset_id(
            ctx.id(),
            AssetKind::Harness,
            &AssetName::try_from(name).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    let h = ctx
        .harnesses()
        .find(|h| h.id().as_uuid() == id)
        .expect("the harness");
    (
        h.kind().word(),
        h.revision().number(),
        h.cases().iter().map(|c| c.revision().number()).collect(),
    )
}

#[tokio::test]
async fn a_harness_of_examples_is_put_and_one_changed_text_moves_its_case_and_the_harness() {
    let mut app = stock("examples").await;
    let builds = ["stock".to_string()];
    let first = app
        .put_examples_harness(
            "core",
            "acceptance",
            &builds,
            &[
                example_case("a1", "1\n#exit 0\n", "C1"),
                example_case("a2", "2\n#exit 0\n", "C7"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(first.value.revision.number(), 1);
    assert_eq!(
        harness_state(&app, "acceptance").await,
        ("examples", 1, vec![1, 1])
    );

    // the same examples again move nothing
    let again = app
        .put_examples_harness(
            "core",
            "acceptance",
            &builds,
            &[
                example_case("a1", "1\n#exit 0\n", "C1"),
                example_case("a2", "2\n#exit 0\n", "C7"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(again.value.id, first.value.id);
    assert_eq!(again.value.revision.number(), 1);

    // one text of one example is a new revision of its Case and of the Harness, and not of the other Case
    let changed = app
        .put_examples_harness(
            "core",
            "acceptance",
            &builds,
            &[
                example_case("a1", "1\n#exit 0\n", "C1"),
                example_case("a2", "2\n#exit 1\n", "C7"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(changed.value.revision.number(), 2);
    assert_eq!(
        harness_state(&app, "acceptance").await,
        ("examples", 2, vec![1, 2])
    );
    // the tests Harness beside it did not move
    assert_eq!(
        harness_state(&app, "stock-tests").await,
        ("tests", 1, vec![1, 1])
    );
}

#[tokio::test]
async fn a_put_of_examples_that_is_not_their_shape_is_refused_by_the_rule_that_names_it() {
    let mut app = stock("examples-refused").await;
    let builds = ["stock".to_string()];
    let code = |d: &[Diagnostic]| d.iter().map(|d| d.code.clone()).collect::<Vec<_>>();

    // no Interface to build
    let err = app
        .put_examples_harness(
            "core",
            "acceptance",
            &[],
            &[example_case("a1", "#exit 0\n", "C1")],
        )
        .await
        .unwrap_err();
    assert_eq!(code(&app.diagnose(&err).await), ["examples.builds-none"]);

    // an expected output with no `#exit` line, said of the Case by name
    let err = app
        .put_examples_harness(
            "core",
            "acceptance",
            &builds,
            &[example_case("a1", "1\n", "C1")],
        )
        .await
        .unwrap_err();
    let d = app.diagnose(&err).await;
    assert_eq!(code(&d), ["examples.expected-exit"]);
    assert_eq!(d[0].subjects[0].name, "case acceptance a1");

    // a Case that cites no Clause, and a name no example has
    let mut uncited = example_case("a1", "#exit 0\n", "C1");
    uncited.clauses.clear();
    let err = app
        .put_examples_harness("core", "acceptance", &builds, &[uncited])
        .await
        .unwrap_err();
    assert_eq!(code(&app.diagnose(&err).await), ["examples.cites-none"]);
    let err = app
        .put_examples_harness(
            "core",
            "acceptance",
            &builds,
            &[example_case("a/1", "#exit 0\n", "C1")],
        )
        .await
        .unwrap_err();
    let d = app.diagnose(&err).await;
    assert_eq!(code(&d), ["examples.case-name"]);

    // a text past the limit of a put
    let mut big = example_case("a1", "#exit 0\n", "C1");
    big.example.stdin = "x".repeat(1024 * 1024 + 1);
    let err = app
        .put_examples_harness("core", "acceptance", &builds, &[big])
        .await
        .unwrap_err();
    let d = app.diagnose(&err).await;
    assert_eq!(code(&d), ["examples.too-large"]);

    // the door for code does not take the kind of examples
    let err = app
        .put_harness_of(
            "core",
            "acceptance",
            Some(HarnessKind::Examples),
            &builds,
            "",
            &[],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Input(_)), "{err:?}");

    // nothing was made by all of that
    let p = app.store().project().await.unwrap().unwrap();
    let core = p
        .contexts()
        .find(|c| c.name().to_string() == "core")
        .unwrap();
    assert_eq!(core.harnesses().count(), 1);

    // and code put to a Harness of examples must say it is code
    app.put_examples_harness(
        "core",
        "acceptance",
        &builds,
        &[example_case("a1", "#exit 0\n", "C1")],
    )
    .await
    .unwrap();
    let err = app
        .put_harness(
            "core",
            "acceptance",
            "fn reserve_refuses() {}\n",
            &cases(&["reserve"]),
        )
        .await
        .unwrap_err();
    assert_eq!(code(&app.diagnose(&err).await), ["examples.kind-content"]);
    app.put_harness_of(
        "core",
        "acceptance",
        Some(HarnessKind::Tests),
        &[],
        "fn reserve_refuses() {}\n",
        &cases(&["reserve"]),
    )
    .await
    .unwrap();
    // the Harness moves with its kind; `reserve` is a Case it did not have (it held `a1`), so it starts at 1
    assert_eq!(
        harness_state(&app, "acceptance").await,
        ("tests", 2, vec![1])
    );
}

#[tokio::test]
async fn a_change_set_makes_a_contract_and_a_harness_of_examples_that_cites_its_clause() {
    let mut app = app("examples-set").await;
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_interface("core", "stock", "fn reserve()\n", &[item("reserve", 1)])
        .await
        .unwrap();
    let set = ChangeSetInput {
        change_request: None,
        puts: vec![
            PutInput::Contract(ContractPut {
                context: "core".to_string(),
                name: AssetName::try_from("stock").unwrap(),
                title: "Stock".to_string(),
                clauses: clauses(&["C1"]),
            }),
            PutInput::Harness(HarnessPut {
                context: "core".to_string(),
                name: AssetName::try_from("acceptance").unwrap(),
                builds: vec![AssetName::try_from("stock").unwrap()],
                content: HarnessPutContent::Examples(vec![example_case(
                    "a1",
                    "1\n#exit 0\n",
                    "C1",
                )]),
            }),
        ],
        retires: Vec::new(),
    };
    let done = app.apply_change_set(&set).await.unwrap();
    let kinds: Vec<&str> = done.value.puts.iter().map(|p| p.kind()).collect();
    assert_eq!(kinds, ["contract", "harness"]);
    let clause = clause_of(&app, "C1").await;
    let p = app.store().project().await.unwrap().unwrap();
    let harness = p
        .contexts()
        .flat_map(|c| c.harnesses())
        .find(|h| h.kind().word() == "examples")
        .expect("the harness of examples");
    let case = &harness.cases()[0];
    assert_eq!(
        case.cites().clauses().keys().copied().collect::<Vec<_>>(),
        [clause.id()]
    );
    assert!(case.cites().items().is_empty());
    assert_eq!(case.example().unwrap().expected(), "1\n#exit 0\n");
    assert_eq!(harness.builds().len(), 1);
}
