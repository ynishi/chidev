//! A Clause is known by its id, and its label is the name a person uses for it: a new label alone moves no
//! revision, `replaces` keeps an id across it, and `put_clauses` changes some Clauses of a Contract and leaves the
//! others as they are.

use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeSetInput, ClauseInput, ClauseRef, ClausesPut, ContractPut,
    HarnessPut, ItemInput, ItemRef, PutInput,
};
use chidev_core::domain::repository::ProjectRepository;
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

type TestApp = App<SqliteStore, Counter>;

fn clause(label: &str, text: &str) -> ClauseInput {
    ClauseInput {
        label: label.to_string(),
        text: text.to_string(),
        replaces: None,
    }
}

fn renamed(label: &str, text: &str, was: &str) -> ClauseInput {
    ClauseInput {
        replaces: Some(was.to_string()),
        ..clause(label, text)
    }
}

fn texts() -> [(&'static str, &'static str); 4] {
    [
        ("C1", "a unit is reserved"),
        ("C2", "a unit is released"),
        ("C3", "stock is not negative"),
        ("C4", "a reservation is not kept for ever"),
    ]
}

fn all() -> Vec<ClauseInput> {
    texts().iter().map(|(l, t)| clause(l, t)).collect()
}

/// The Case `name`, which cites the Item `price` and the Clause `label` of the Contract `stock`.
fn case(name: &str, label: &str) -> CaseInput {
    CaseInput {
        name: name.to_string(),
        lines: (1, 1),
        items: vec![ItemRef {
            context: None,
            interface: "api".to_string(),
            key: "price".to_string(),
        }],
        clauses: vec![ClauseRef {
            context: None,
            contract: "stock".to_string(),
            label: label.to_string(),
        }],
    }
}

fn cases() -> Vec<CaseInput> {
    vec![case("a", "C1"), case("b", "C2"), case("c", "C3")]
}

/// A Context `core` with the Contract `stock` (C1 to C4), the Interface `api` (the Item `price`) and the Harness
/// `tests`, whose Cases `a`, `b` and `c` cite C1, C2 and C3.
async fn stock(name: &str) -> TestApp {
    let dir = std::env::temp_dir().join(format!("chidev-clauses-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
    let mut app = App::new(
        store,
        Counter::default(),
        FixedClock(Timestamp::from_millis(1_000)),
    );
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract("core", "stock", "Stock", &all())
        .await
        .unwrap();
    app.put_interface(
        "core",
        "api",
        "fn price()\n",
        &[ItemInput {
            key: "price".to_string(),
            lines: (1, 1),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    app.put_harness(
        "core",
        "tests",
        "fn a() {}\nfn b() {}\nfn c() {}\n",
        &cases(),
    )
    .await
    .unwrap();
    app
}

/// The Contract `stock`: its revision, and the id, label, revision and sentence of each Clause, in order.
async fn state(app: &TestApp) -> (u64, Vec<(Uuid, String, u64, String)>) {
    let p = app.store().project().await.unwrap().unwrap();
    let k = p
        .contexts()
        .flat_map(|c| c.contracts())
        .next()
        .expect("the Contract");
    (
        k.revision().number(),
        k.clauses()
            .iter()
            .map(|c| {
                (
                    c.id().as_uuid(),
                    c.label().to_string(),
                    c.revision().number(),
                    c.text().as_str().to_string(),
                )
            })
            .collect(),
    )
}

fn labels(clauses: &[(Uuid, String, u64, String)]) -> Vec<&str> {
    clauses.iter().map(|c| c.1.as_str()).collect()
}

fn revisions(clauses: &[(Uuid, String, u64, String)]) -> Vec<u64> {
    clauses.iter().map(|c| c.2).collect()
}

fn stale(found: &[chidev_core::app::Found]) -> usize {
    found
        .iter()
        .filter(|f| f.rule.as_str() == "cite.stale")
        .count()
}

#[tokio::test]
async fn clauses_put_rewords_one_adds_one_and_removes_one_and_the_others_stay() {
    let mut app = stock("partial").await;
    let (revision, before) = state(&app).await;
    assert_eq!(revision, 1);
    let put = app
        .put_clauses(
            "core",
            "stock",
            &[
                clause("C1", "a unit is reserved at most once"),
                clause("C5", "a unit is counted"),
            ],
            &["C4".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(put.value.revision.number(), 2);
    let (revision, after) = state(&app).await;
    assert_eq!(revision, 2);
    // the order is kept, the new Clause comes after, the removed one is gone
    assert_eq!(labels(&after), ["C1", "C2", "C3", "C5"]);
    assert_eq!(revisions(&after), [2, 1, 1, 1]);
    assert_eq!(after[0].3, "a unit is reserved at most once");
    // the Clauses not named keep their id and their sentence
    for k in [1, 2] {
        assert_eq!(after[k], before[k]);
    }
    assert_eq!(after[0].0, before[0].0);
    assert!(before.iter().all(|b| b.0 != after[3].0));
    // the Case that cites the reworded Clause is stale, and the two that cite the others are not
    assert_eq!(stale(&put.findings), 1, "{:?}", put.findings);
}

#[tokio::test]
async fn the_same_sentence_again_moves_nothing() {
    let mut app = stock("same").await;
    let (_, before) = state(&app).await;
    let put = app
        .put_clauses("core", "stock", &[clause("C2", "a unit is released")], &[])
        .await
        .unwrap();
    assert_eq!(put.value.revision.number(), 1);
    assert_eq!(state(&app).await, (1, before));
}

#[tokio::test]
async fn a_new_label_by_replaces_keeps_the_id_the_place_and_every_revision() {
    let mut app = stock("replaces").await;
    let (_, before) = state(&app).await;
    let put = app
        .put_clauses(
            "core",
            "stock",
            &[renamed("C7", "a unit is released", "C2")],
            &[],
        )
        .await
        .unwrap();
    assert_eq!(put.value.revision.number(), 1);
    let (revision, after) = state(&app).await;
    assert_eq!(revision, 1);
    assert_eq!(labels(&after), ["C1", "C7", "C3", "C4"]);
    assert_eq!(after[1].0, before[1].0);
    assert_eq!(revisions(&after), [1, 1, 1, 1]);
    // the Case that cites it by id stays current
    assert_eq!(stale(&put.findings), 0, "{:?}", put.findings);
    assert_eq!(
        stale(&app.check(None).await.unwrap().findings),
        0,
        "no citation went stale"
    );
    // the old label is gone for a Harness put, which names it
    let err = app
        .put_harness("core", "tests", "fn a() {}\n", &[case("b", "C2")])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::UnknownClause { label, .. } if label == "C2"),
        "{err:?}"
    );
    // and the new one is cited
    app.put_harness("core", "tests", "fn b() {}\n", &[case("b", "C7")])
        .await
        .unwrap();
}

#[tokio::test]
async fn replaces_in_a_whole_put_keeps_the_id_and_the_revisions_too() {
    let mut app = stock("replaces-whole").await;
    let (_, before) = state(&app).await;
    let mut given = all();
    given[1] = renamed("C8", "a unit is released", "C2");
    let put = app
        .put_contract("core", "stock", "Stock", &given)
        .await
        .unwrap();
    assert_eq!(put.value.revision.number(), 1);
    let (revision, after) = state(&app).await;
    assert_eq!(revision, 1);
    assert_eq!(labels(&after), ["C1", "C8", "C3", "C4"]);
    assert_eq!(after[1].0, before[1].0);
    assert_eq!(revisions(&after), [1, 1, 1, 1]);
    assert_eq!(stale(&put.findings), 0, "{:?}", put.findings);
}

#[tokio::test]
async fn a_name_that_cannot_be_carried_out_is_refused_with_everything_it_found_and_nothing_changes()
{
    let mut app = stock("refused").await;
    let before = state(&app).await;
    let unknown = |label: &str, field| AppError::ClauseUnknown {
        context: "core".to_string(),
        contract: "stock".to_string(),
        label: label.to_string(),
        field,
    };
    let conflict = |label: &str, side| AppError::ClauseRenameConflict {
        context: "core".to_string(),
        contract: "stock".to_string(),
        label: label.to_string(),
        side,
    };

    let err = app
        .put_clauses("core", "stock", &[renamed("C5", "x", "C9")], &[])
        .await
        .unwrap_err();
    assert_eq!(err, unknown("C9", "replaces"));
    let err = app
        .put_clauses("core", "stock", &[], &["C9".to_string()])
        .await
        .unwrap_err();
    assert_eq!(err, unknown("C9", "remove"));
    // a replaces of its own label changes no name, and is no conflict
    app.put_clauses(
        "core",
        "stock",
        &[renamed("C1", "a unit is reserved", "C1")],
        &[],
    )
    .await
    .unwrap();
    let kept = app.store().revision().await.unwrap();

    let kept_label = app
        .put_clauses(
            "core",
            "stock",
            &[renamed("C1", "x", "C2"), clause("C2", "y")],
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(
        kept_label,
        AppError::ClauseReplacesKept {
            context: "core".to_string(),
            contract: "stock".to_string(),
            label: "C2".to_string(),
        }
    );
    let twice = app
        .put_clauses(
            "core",
            "stock",
            &[clause("C1", "x"), clause("C1", "y")],
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(twice, conflict("C1", "label"));
    let replaced_twice = app
        .put_clauses(
            "core",
            "stock",
            &[renamed("C5", "x", "C1"), renamed("C6", "y", "C1")],
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(replaced_twice, conflict("C1", "replaces"));
    let removed_and_given = app
        .put_clauses("core", "stock", &[clause("C1", "x")], &["C1".to_string()])
        .await
        .unwrap_err();
    assert_eq!(removed_and_given, conflict("C1", "remove"));
    let removed_and_replaced = app
        .put_clauses(
            "core",
            "stock",
            &[renamed("C5", "x", "C1")],
            &["C1".to_string()],
        )
        .await
        .unwrap_err();
    assert_eq!(removed_and_replaced, conflict("C1", "remove"));

    // the problems of one entry come together, in the order the entry lists them
    let many = app
        .put_clauses(
            "core",
            "stock",
            &[
                renamed("C1", "x", "C2"),
                clause("C2", "y"),
                clause("C3", "z"),
                clause("C3", "w"),
            ],
            &["C9".to_string()],
        )
        .await
        .unwrap_err();
    assert_eq!(
        many,
        AppError::Many(vec![
            AppError::ClauseReplacesKept {
                context: "core".to_string(),
                contract: "stock".to_string(),
                label: "C2".to_string(),
            },
            conflict("C3", "label"),
            unknown("C9", "remove"),
        ])
    );

    // the Contract has to exist
    let err = app
        .put_clauses("core", "nope", &[clause("C1", "x")], &[])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::UnknownAsset { kind: "contract", name, .. } if name == "nope"),
        "{err:?}"
    );

    // nothing was kept
    assert_eq!(state(&app).await, before);
    assert_eq!(app.store().revision().await.unwrap(), kept);
}

#[tokio::test]
async fn a_whole_put_with_a_replaces_is_refused_by_the_same_rules() {
    let mut app = stock("whole-refused").await;
    let before = state(&app).await;
    let mut given = all();
    given[1] = renamed("C8", "a unit is released", "C9");
    let err = app
        .put_contract("core", "stock", "Stock", &given)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        AppError::ClauseUnknown {
            context: "core".to_string(),
            contract: "stock".to_string(),
            label: "C9".to_string(),
            field: "replaces",
        }
    );
    // a label the Contract has, given twice, was an id given twice
    let mut twice = all();
    twice[1] = clause("C1", "again");
    let err = app
        .put_contract("core", "stock", "Stock", &twice)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        AppError::ClauseRenameConflict {
            context: "core".to_string(),
            contract: "stock".to_string(),
            label: "C1".to_string(),
            side: "label",
        }
    );
    assert_eq!(state(&app).await, before);
}

fn put_clauses_entry(clauses: Vec<ClauseInput>, remove: &[&str]) -> PutInput {
    PutInput::Clauses(ClausesPut {
        context: "core".to_string(),
        contract: "stock".to_string(),
        clauses,
        remove: remove.iter().map(|s| (*s).to_string()).collect(),
    })
}

fn put_tests(cases: Vec<CaseInput>) -> PutInput {
    PutInput::Harness(HarnessPut {
        context: "core".to_string(),
        name: "tests".to_string(),
        kind: None,
        builds: Vec::new(),
        source: "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n".to_string(),
        cases,
    })
}

#[tokio::test]
async fn a_harness_in_the_same_set_cites_a_label_the_clauses_entry_adds_and_one_it_renames() {
    let mut app = stock("same-set").await;
    let (_, before) = state(&app).await;
    let set = ChangeSetInput {
        change_request: None,
        puts: vec![
            put_clauses_entry(
                vec![
                    renamed("C6", "a unit is reserved", "C1"),
                    clause("C9", "a unit is counted"),
                ],
                &[],
            ),
            put_tests(vec![
                case("a", "C6"),
                case("b", "C2"),
                case("c", "C3"),
                case("d", "C9"),
            ]),
        ],
        retires: Vec::new(),
    };
    let done = app.apply_change_set(&set).await.unwrap();
    let kinds: Vec<&str> = done.value.puts.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, ["clauses", "harness"]);
    let (revision, after) = state(&app).await;
    // the added Clause moves the Contract, the new label alone does not move the Clause
    assert_eq!(revision, 2);
    assert_eq!(labels(&after), ["C6", "C2", "C3", "C4", "C9"]);
    assert_eq!(after[0].0, before[0].0);
    assert_eq!(revisions(&after), [1, 1, 1, 1, 1]);
}

#[tokio::test]
async fn a_contract_and_a_clauses_entry_for_one_contract_in_one_set_are_refused() {
    let mut app = stock("named-twice").await;
    let before = state(&app).await;
    let set = ChangeSetInput {
        change_request: None,
        puts: vec![
            PutInput::Contract(ContractPut {
                context: "core".to_string(),
                name: "stock".to_string(),
                title: "Stock".to_string(),
                clauses: all(),
            }),
            put_clauses_entry(vec![clause("C1", "x")], &[]),
        ],
        retires: Vec::new(),
    };
    let err = app.apply_change_set(&set).await.unwrap_err();
    let said = app.diagnose(&err).await;
    assert_eq!(said[0].code, "change-set.named-twice", "{said:#?}");
    assert_eq!(state(&app).await, before);
}
