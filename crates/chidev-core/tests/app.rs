//! The use cases over the real store: each test opens an SQLite database of its own in a fresh temporary
//! directory, so no test double of the store is needed.

use chidev_core::app::ports::{AssetKind, AssetNames, IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeSetInput, ClauseInput, ClauseRef, ContractPut, Diagnostic,
    HarnessKind, HarnessPut, InterfacePut, ItemInput, ItemRef, PutInput, RetireInput,
};
use chidev_core::domain::entity::contract::Clause;
use chidev_core::domain::repository::ProjectRepository;
use chidev_core::domain::vo::finding::Subject;
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
                context: context.map(str::to_string),
                contract: contract.to_string(),
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
        .find(|c| c.content().label().to_string() == label)
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
    let found = app.check().await.unwrap();
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
    let found = app.check().await.unwrap();
    assert!(
        found.iter().all(|f| f.rule.as_str() != "sim.shape-kept"),
        "{found:#?}"
    );
}

#[tokio::test]
async fn an_asset_and_its_name_are_kept_together() {
    let mut app = app("asset-name").await;
    app.init().await.unwrap();
    let ctx = app.add_context("core", &[]).await.unwrap();
    let put = app
        .put_contract(
            "core",
            "stack",
            "Stack",
            &[ClauseInput {
                label: "C1".to_string(),
                text: "push then pop".to_string(),
            }],
        )
        .await
        .unwrap();
    let store = app.store();
    assert_eq!(
        store
            .asset_id(ctx, AssetKind::Contract, "stack")
            .await
            .unwrap(),
        Some(put.value.id.as_uuid())
    );
    let p = store.project().await.unwrap().unwrap();
    assert!(p.context(ctx).unwrap().contract(put.value.id).is_some());
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
                interface: "stock".to_string(),
                key: (*w).to_string(),
            }],
            clauses: vec![ClauseRef {
                context: None,
                contract: "stock".to_string(),
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
                name: "stock".to_string(),
                title: "Stock".to_string(),
                clauses: clauses(&["C1"]),
            }),
            PutInput::Interface(InterfacePut {
                context: "core".to_string(),
                name: "stock".to_string(),
                source: "fn reserve()\n".to_string(),
                items: Some(vec![item("reserve", 1)]),
                renames: Vec::new(),
            }),
            PutInput::Harness(HarnessPut {
                context: "core".to_string(),
                name: "stock-tests".to_string(),
                kind: None,
                builds: Vec::new(),
                source: "fn reserve_refuses() {}\n".to_string(),
                cases: cases(&["reserve"]),
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
    let kinds: Vec<&str> = done.value.puts.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, ["contract", "interface", "harness"]);
    assert!(
        done.value
            .puts
            .iter()
            .all(|p| p.revision.map(|r| r.number()) == Some(2))
    );
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
                    interface: "stock".to_string(),
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
    let found = app.check().await.unwrap();
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
                    interface: "stock".to_string(),
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
        harness: "stock-tests".to_string(),
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
                name: "entries".to_string(),
                title: "Entries".to_string(),
                clauses: clauses(&["C1"]),
            }),
            PutInput::Interface(InterfacePut {
                context: "billing".to_string(),
                name: "invoice".to_string(),
                source: "fn total()\n".to_string(),
                items: Some(vec![item("total", 1)]),
                renames: Vec::new(),
            }),
            PutInput::Context {
                name: "ledger".to_string(),
                uses: Vec::new(),
            },
        ],
        retires: Vec::new(),
    };
    let done = app.apply_change_set(&set).await.unwrap();
    let kinds: Vec<&str> = done.value.puts.iter().map(|p| p.kind).collect();
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

    use chidev_core::app::ports::{Invocation, Outcome};
    use chidev_core::app::{
        App, AppError, ChangeRequestInput, ClauseInput, ClauseRef, FindingInput, Intent, Registry,
        SubjectInput,
    };
    use chidev_core::domain::check::{
        Check, CheckView, Descriptor, Executor, Finding, FindingState, Hook, Level, Params,
    };
    use chidev_core::domain::vo::finding::RuleId;
    use chidev_core::domain::vo::timestamp::Timestamp;
    use chidev_core::infra::clock::FixedClock;
    use chidev_core::infra::sqlite::SqliteStore;

    use super::{Counter, app};

    /// A mandatory check that finds the Project whenever it has a Context, to try the refusal with.
    struct Forbidden;

    static FORBIDDEN: Descriptor = Descriptor {
        id: RuleId::from_static("test.forbidden"),
        summary: Cow::Borrowed("a context exists"),
        help: Cow::Borrowed("only for the test"),
        hooks: Cow::Borrowed(&[Hook::Put]),
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
            view.after
                .contexts()
                .map(|c| Finding::new(&FORBIDDEN, None, format!("context {}", c.name())))
                .collect()
        }
    }

    fn clauses() -> Vec<ClauseInput> {
        vec![ClauseInput {
            label: "C1".to_string(),
            text: "push then pop".to_string(),
        }]
    }

    async fn invocations(app: &App<SqliteStore, Counter>) -> Vec<Invocation> {
        app.store().invocations().await.unwrap()
    }

    #[tokio::test]
    async fn a_put_reports_what_the_checks_find_and_records_each_run() {
        let mut app = app("checks-put").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        let put = app
            .put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        let rules: Vec<&str> = put.findings.iter().map(|f| f.rule.as_str()).collect();
        assert!(rules.contains(&"cite.uncited-clause"), "{rules:?}");
        assert!(rules.contains(&"context.no-interface"));
        assert!(put.findings.iter().all(|f| f.state == FindingState::Open));
        let ran = invocations(&app).await;
        let uncited = ran
            .iter()
            .find(|i| i.check.as_str() == "cite.uncited-clause")
            .unwrap();
        assert_eq!(uncited.hook, Hook::Put);
        assert_eq!((uncited.warnings, uncited.outcome), (1, Outcome::Found));
        // A check with nothing to find is recorded too, as ok.
        let stale = ran
            .iter()
            .find(|i| i.check.as_str() == "cite.stale")
            .unwrap();
        assert_eq!((stale.warnings, stale.outcome), (0, Outcome::Ok));
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
        let found = app.check().await.unwrap();
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
            crux: None,
            assumptions: Vec::new(),
            addresses: vec![FindingInput {
                rule: "cite.uncited-clause".to_string(),
                subject: Some(SubjectInput::Clause(ClauseRef {
                    context: Some("core".to_string()),
                    contract: "stack".to_string(),
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
        assert_eq!(state(&app.check().await.unwrap()), Some(FindingState::Open));
        app.open_change_request(&open("leave C1 unchecked"))
            .await
            .unwrap();
        assert_eq!(
            state(&app.check().await.unwrap()),
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
            state(&app.check().await.unwrap()),
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
                .asset_id(ctx, AssetKind::Contract, "stack")
                .await
                .unwrap(),
            None
        );
        assert!(invocations(&app).await.is_empty());
    }
}

mod judged {
    use chidev_core::app::ports::{Outcome, Store};
    use chidev_core::app::{
        App, AppError, ChangeRequestInput, ClauseInput, ClauseRef, FindingInput, Intent,
        JudgedState, Registry, ReportInput, ReportedFinding, SubjectInput,
    };
    use chidev_core::domain::check::{Executor, FindingState, Hook, Level};
    use chidev_core::domain::vo::timestamp::Timestamp;
    use chidev_core::infra::clock::FixedClock;
    use chidev_core::infra::judged::parse;
    use chidev_core::infra::sqlite::SqliteStore;

    use super::Counter;

    fn rule(id: &str, subject: &str, level: &str) -> String {
        format!(
            "id = \"{id}\"\nsummary = \"s\"\nsubject = \"{subject}\"\nhooks = [\"put\"]\nlevel = \"{level}\"\n\
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
            context: Some("core".to_string()),
            contract: "stack".to_string(),
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
                rule("judged.later", "clause", "note").replace("put", "check"),
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
        assert_eq!(json["version"], 3);
        let clauses = &json["contexts"][0]["contracts"][0]["clauses"];
        assert_eq!(json["contexts"][0]["contracts"][0]["name"], "stack");
        assert_eq!(clauses[0]["label"], "C1");
        assert_eq!(clauses[0]["text"], "push then pop");
        assert_eq!(clauses[0]["revision"], 1);

        let at_put = app.check_rules(Some(Hook::Put), None).await.unwrap();
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
            Some(SubjectInput::Context("core".to_string())),
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
            app.check()
                .await
                .unwrap()
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
        let found = app.check().await.unwrap();
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
        let found = app.check().await.unwrap();
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
        let found = app.check().await.unwrap();
        assert_eq!(judged_findings(&found).len(), 1);
        let view = app.check_rules(None, Some("judged.vague")).await.unwrap();
        assert_eq!(view.rules[0].state, JudgedState::Current);

        // each report was recorded as a run of a judged rule, by the judge
        let ran = app.store().invocations().await.unwrap();
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
        assert_eq!(stale_of(&app.check().await.unwrap()), Some(false));
        assert!(app.status().await.unwrap().judged.is_empty());

        // another clause changes: the rule's part of the model moved, so it is to be judged again, but the
        // finding about C1 stands
        put_stack(&mut app, "push then pop", "an empty stack pops nothing").await;
        assert_eq!(stale_of(&app.check().await.unwrap()), Some(false));
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
        let found = app.check().await.unwrap();
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
        assert_eq!(stale_of(&app.check().await.unwrap()), Some(false));
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
        let found = app.check().await.unwrap();
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
        assert_eq!(state(&app.check().await.unwrap()), FindingState::Open);
        app.open_change_request(&ChangeRequestInput {
            title: "reword C1".to_string(),
            situation: None,
            intent: Intent::ChangeBehavior,
            parent: None,
            issuer: None,
            targets: Vec::new(),
            keep: Vec::new(),
            crux: None,
            assumptions: Vec::new(),
            addresses: vec![FindingInput {
                rule: "judged.vague".to_string(),
                subject: Some(SubjectInput::Clause(ClauseRef {
                    context: Some("core".to_string()),
                    contract: "stack".to_string(),
                    label: "C1".to_string(),
                })),
            }],
        })
        .await
        .unwrap();
        assert_eq!(state(&app.check().await.unwrap()), FindingState::Addressed);
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
                context: Some("core".to_string()),
                interface: "stack-api".to_string(),
                key: "push".to_string(),
            }],
            clauses: vec![ClauseRef {
                context: Some("core".to_string()),
                contract: "stack".to_string(),
                label: "C1".to_string(),
            }],
        };
        app.put_harness("core", "stack-test", "fn t() {}\n", &[cites])
            .await
            .unwrap();
        app.pin("core", "stack-api", "code").await.unwrap();
        put_stack(&mut app, "push then pop, in order", "x").await;
        let found = app.check().await.unwrap();
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
        PutInput, ResolutionInput, TargetInput,
    };
    use chidev_core::infra::sqlite::SqliteStore;
    use serde_json::json;

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
            crux: None,
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
            d.suggestions[0].op,
            json!({"op": "cr_open", "title": "Not opened", "intent": "change_behavior"})
        );
        assert_eq!(d.suggestions[0].applicability, "review");
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
            d.suggestions[0].op,
            json!({"op": "context_add", "name": "cor"})
        );

        let err = app.status().await.and_then(|_| {
            Err::<(), _>(chidev_core::app::AppError::UnknownAsset {
                context: "core".to_string(),
                kind: "contract",
                name: "stok".to_string(),
            })
        });
        let d = only(&app, err.unwrap_err()).await;
        assert_eq!(d.did_you_mean, ["stock"]);
        assert_eq!(d.subjects[0].name, "core");

        let target = TargetInput::Clause(ClauseRef {
            context: Some("core".to_string()),
            contract: "stock".to_string(),
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
        assert_eq!(d.suggestions[0].op["op"], "cr_resolve");
        assert_eq!(d.suggestions[0].op["title"], "Part one");
        assert_eq!(d.suggestions[0].applicability, "placeholder");
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
        assert_eq!(d.suggestions[0].op["op"], "cr_reopen");
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
