//! The Items an Interface is put with when `items` is left out: the provider of the language finds them, the use
//! case turns a `renames` into the id an Item keeps, and every way this cannot be done is a rule of its own. The
//! same source of Items serves `items_rekey`, which gives the Items an Interface keeps the keys and lines the
//! provider finds in its code. The source of Items is a fake that answers what the test says (the process adapter is tested where it is, in
//! `infra/runner`, and end to end in `chidev-cli`); the store is the real one.

use std::sync::{Arc, Mutex};

use chidev_core::app::ports::items::{FoundItem, ItemSource, ItemsError};
use chidev_core::app::ports::measure::BoxFuture;
use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeSetInput, ContractPut, Diagnostic, InterfacePut, ItemInput,
    ItemRef, ItemRevised, ItemsInput, KeyChanged, PutInput, RenameInput,
};
use chidev_core::domain::repository::ProjectRepository;
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::sqlite::SqliteStore;
use uuid::Uuid;

/// Ids `start + 1`, `start + 2`, ...
#[derive(Default)]
struct Counter(u128);

impl IdSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_u128(self.0)
    }
}

type Language = dyn Fn(&str, &str) -> Result<String, ItemsError> + Send + Sync;
type Answer = dyn Fn(&str, &str) -> Result<Vec<FoundItem>, ItemsError> + Send + Sync;
type Moment = dyn Fn() -> BoxFuture<'static, ()> + Send + Sync;

/// A source of Items that answers as the test says, and keeps what it was asked.
#[derive(Clone)]
struct Fake {
    /// The language of (context, interface).
    language: Arc<Language>,
    /// The Items of (language, source).
    answer: Arc<Answer>,
    /// What `items` was asked: the language and the source.
    asked: Arc<Mutex<Vec<(String, String)>>>,
    /// Run while the provider is being asked, as the world goes on while a process runs.
    meanwhile: Option<Arc<Moment>>,
}

impl std::fmt::Debug for Fake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Fake")
    }
}

impl Fake {
    /// Every Interface is `rust`, and the provider answers `found`.
    fn answering(found: &[(&str, usize, usize)]) -> Fake {
        let found: Vec<FoundItem> = found
            .iter()
            .map(|(key, first, last)| FoundItem {
                key: (*key).to_string(),
                lines: (*first, *last),
            })
            .collect();
        Fake::with(
            Arc::new(|_, _| Ok("rust".to_string())),
            Arc::new(move |_, _| Ok(found.clone())),
        )
    }

    /// Every Interface is `rust`, and the provider answers `error`.
    fn failing(error: ItemsError) -> Fake {
        Fake::with(
            Arc::new(|_, _| Ok("rust".to_string())),
            Arc::new(move |_, _| Err(error.clone())),
        )
    }

    fn with(language: Arc<Language>, answer: Arc<Answer>) -> Fake {
        Fake {
            language,
            answer,
            asked: Arc::default(),
            meanwhile: None,
        }
    }

    fn asked(&self) -> Vec<(String, String)> {
        self.asked.lock().unwrap().clone()
    }
}

impl ItemSource for Fake {
    fn language(&self, context: &ContextName, interface: &AssetName) -> Result<String, ItemsError> {
        (self.language)(context.as_str(), interface.as_str())
    }

    fn items<'a>(
        &'a self,
        lang: &'a str,
        source: &'a str,
    ) -> BoxFuture<'a, Result<Vec<FoundItem>, ItemsError>> {
        Box::pin(async move {
            self.asked
                .lock()
                .unwrap()
                .push((lang.to_string(), source.to_string()));
            if let Some(meanwhile) = &self.meanwhile {
                meanwhile().await;
            }
            (self.answer)(lang, source)
        })
    }
}

type Stock = App<SqliteStore, Counter>;

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("chidev-items-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn open(path: &std::path::Path, start: u128) -> Stock {
    App::new(
        SqliteStore::open(path).await.unwrap(),
        Counter(start),
        FixedClock(Timestamp::from_millis(1_000)),
    )
}

fn item(key: &str, line: usize) -> ItemInput {
    ItemInput {
        key: key.to_string(),
        lines: (line, line),
        replaces: None,
    }
}

fn rename(from: &str, to: &str) -> RenameInput {
    RenameInput {
        from: from.to_string(),
        to: to.to_string(),
    }
}

const SOURCE: &str = "fn reserve()\nfn release()\n";

/// A project with the Context `core` and its Interface `stock`, put with the Items `reserve` and `release`
/// written, over `fake`.
async fn stock(name: &str, fake: Fake) -> Stock {
    let path = dir(name).join("chidev.db");
    let mut app = open(&path, 0).await.with_items(fake);
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_interface(
        "core",
        "stock",
        SOURCE,
        &[item("reserve", 1), item("release", 2)],
    )
    .await
    .unwrap();
    app
}

/// The Interface revision, and the key, id and revision of each Item of the Interface `stock`.
async fn stock_items(app: &Stock) -> (u64, Vec<(String, Uuid, u64)>) {
    stock_items_of(app, "reserve").await
}

/// As [`stock_items`], for the Interface whose code has `word`.
async fn stock_items_of(app: &Stock, word: &str) -> (u64, Vec<(String, Uuid, u64)>) {
    let p = app.store().project().await.unwrap().unwrap();
    let i = p
        .contexts()
        .flat_map(|c| c.interfaces())
        .find(|i| i.source().as_str().contains(word))
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

/// The Items of a put that are left to the provider, with these renames.
fn found(renames: &[RenameInput]) -> ItemsInput {
    ItemsInput::Found {
        renames: renames.to_vec(),
    }
}

/// The one diagnostic of a put of `stock` that is refused, with nothing kept.
async fn refused(app: &mut Stock, source: &str, items: ItemsInput) -> Diagnostic {
    let before = app.store().revision().await.unwrap();
    let err = app
        .put_interface_of("core", "stock", source, items)
        .await
        .unwrap_err();
    assert_eq!(
        app.store().revision().await.unwrap(),
        before,
        "nothing kept"
    );
    let mut all = app.diagnose(&err).await;
    assert_eq!(all.len(), 1, "{all:#?}");
    all.remove(0)
}

fn subjects(d: &Diagnostic) -> Vec<&str> {
    d.subjects.iter().map(|s| s.name.as_str()).collect()
}

#[tokio::test]
async fn an_interface_put_without_items_is_given_the_keys_and_lines_the_provider_found() {
    let fake = Fake::answering(&[("fits", 1, 1), ("Stock::spare", 2, 3)]);
    let mut app = open(&dir("found").join("chidev.db"), 0)
        .await
        .with_items(fake.clone());
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    let source = "fn fits()\nfn spare(\n    n: u8)\n";
    let done = app
        .put_interface_of("core", "stock", source, found(&[]))
        .await
        .unwrap();
    assert_eq!(done.value.revision.number(), 1);
    // the provider was asked for the language of the layout, with the code as it was put
    assert_eq!(fake.asked(), [("rust".to_string(), source.to_string())]);
    let (_, items) = stock_items_of(&app, "fits").await;
    assert_eq!(
        items.iter().map(|i| i.0.as_str()).collect::<Vec<_>>(),
        ["fits", "Stock::spare"]
    );
    // the signature of an Item is the text of its lines, as with Items that are written
    let p = app.store().project().await.unwrap().unwrap();
    let signatures: Vec<String> = p
        .contexts()
        .flat_map(|c| c.interfaces())
        .flat_map(|i| i.items().iter().map(|it| it.signature().to_string()))
        .collect();
    assert_eq!(signatures, ["fn fits()", "fn spare(\n    n: u8)"]);
}

#[tokio::test]
async fn items_that_are_written_are_all_of_them_and_no_provider_is_asked() {
    let fake = Fake::failing(ItemsError::Failed("must not be asked".to_string()));
    let mut app = stock("written", fake.clone()).await;
    app.put_interface("core", "stock", SOURCE, &[item("reserve", 1)])
        .await
        .unwrap();
    assert!(fake.asked().is_empty());
    let (_, items) = stock_items(&app).await;
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn a_put_again_through_the_provider_keeps_what_it_found_before_and_moves_nothing() {
    let fake = Fake::answering(&[("reserve", 1, 1), ("release", 2, 2)]);
    let mut app = stock("again", fake).await;
    let before = stock_items(&app).await;
    app.put_interface_of("core", "stock", SOURCE, found(&[]))
        .await
        .unwrap();
    assert_eq!(stock_items(&app).await, before);
}

#[tokio::test]
async fn a_rename_hands_the_id_over_and_moves_no_revision() {
    // the provider now calls the first Item `Stock::reserve`
    let fake = Fake::answering(&[("Stock::reserve", 1, 1), ("release", 2, 2)]);
    let mut app = stock("rename", fake).await;
    let (revision, before) = stock_items(&app).await;
    app.put_interface_of(
        "core",
        "stock",
        SOURCE,
        found(&[rename("reserve", "Stock::reserve")]),
    )
    .await
    .unwrap();
    let (now, after) = stock_items(&app).await;
    assert_eq!(now, revision, "the Interface did not move");
    assert_eq!(after[0].0, "Stock::reserve");
    // the same id and the same revision: a rename is not a change of what the Item declares
    assert_eq!((after[0].1, after[0].2), (before[0].1, before[0].2));
    assert_eq!(after[1], before[1]);
}

#[tokio::test]
async fn a_rename_is_for_the_keys_it_names_and_a_key_the_provider_changed_alone_is_a_new_item() {
    // both keys changed, and only the first is named
    let fake = Fake::answering(&[("Stock::reserve", 1, 1), ("Stock::release", 2, 2)]);
    let mut app = stock("rename-named", fake).await;
    let (_, before) = stock_items(&app).await;
    app.put_interface_of(
        "core",
        "stock",
        SOURCE,
        found(&[rename("reserve", "Stock::reserve")]),
    )
    .await
    .unwrap();
    let (_, after) = stock_items(&app).await;
    assert_eq!(after.len(), 2);
    assert_eq!(
        (after[0].0.as_str(), after[0].1),
        ("Stock::reserve", before[0].1)
    );
    // the other was not named: a new Item, and the old one is retired
    assert_eq!(after[1].0, "Stock::release");
    assert_ne!(after[1].1, before[1].1);
}

#[tokio::test]
async fn a_rename_to_the_same_key_is_not_a_rename_that_is_kept() {
    let fake = Fake::answering(&[("reserve", 1, 1), ("release", 2, 2)]);
    let mut app = stock("rename-itself", fake).await;
    let before = stock_items(&app).await;
    app.put_interface_of(
        "core",
        "stock",
        SOURCE,
        found(&[rename("reserve", "reserve")]),
    )
    .await
    .unwrap();
    assert_eq!(stock_items(&app).await, before);
}

#[tokio::test]
async fn a_put_without_items_in_a_language_the_layout_does_not_give_is_refused_by_name() {
    let fake = Fake::with(
        Arc::new(|_, _| Err(ItemsError::NoLanguage)),
        Arc::new(|_, _| Ok(Vec::new())),
    );
    let mut app = stock("required", fake.clone()).await;
    let d = refused(&mut app, SOURCE, found(&[])).await;
    assert_eq!(d.code, "input.items-required");
    assert_eq!(d.level, "error");
    assert_eq!(subjects(&d), ["core", "stock"]);
    assert!(fake.asked().is_empty(), "no language, no provider asked");
}

#[tokio::test]
async fn a_language_with_no_provider_and_a_project_without_a_measure_table_are_both_no_provider() {
    let fake = Fake::with(
        Arc::new(|_, _| Ok("go".to_string())),
        Arc::new(|_, _| Err(ItemsError::NoProvider)),
    );
    let mut app = stock("no-provider", fake).await;
    let d = refused(&mut app, SOURCE, found(&[])).await;
    assert_eq!(d.code, "items.no-provider");
    assert_eq!(d.values["lang"], "go");
    assert!(d.message.contains("no command for `go`"), "{}", d.message);

    // the application that was given no source of Items (no `[measure]`) cannot say a language either
    let mut bare = open(&dir("bare").join("chidev.db"), 0).await;
    bare.init().await.unwrap();
    bare.add_context("core", &[]).await.unwrap();
    let err = bare
        .put_interface_of("core", "stock", SOURCE, found(&[]))
        .await
        .unwrap_err();
    let d = bare.diagnose(&err).await.remove(0);
    assert_eq!(d.code, "items.no-provider");
    assert!(d.values["lang"].is_null());
    assert!(d.message.contains("no `[measure]` table"), "{}", d.message);
    assert_eq!(subjects(&d), ["core", "stock"]);
}

#[tokio::test]
async fn a_provider_that_disagrees_or_cannot_run_is_refused_by_name_with_the_language() {
    let mut app = stock(
        "protocol",
        Fake::failing(ItemsError::Protocol("lines 4..=5 are past the last".into())),
    )
    .await;
    let d = refused(&mut app, SOURCE, found(&[])).await;
    assert_eq!(d.code, "items.protocol");
    assert_eq!(d.values["lang"], "rust");
    assert_eq!(d.values["detail"], "lines 4..=5 are past the last");
    assert!(d.message.contains("past the last"), "{}", d.message);

    let mut app = stock(
        "failed",
        Fake::failing(ItemsError::Failed(
            "the provider did not answer in time".into(),
        )),
    )
    .await;
    let d = refused(&mut app, SOURCE, found(&[])).await;
    assert_eq!(d.code, "items.failed");
    assert_eq!(d.values["lang"], "rust");
    assert_eq!(subjects(&d), ["core", "stock"]);
}

#[tokio::test]
async fn a_rename_of_a_key_that_is_not_there_is_refused_whichever_side_it_is_on() {
    let fake = Fake::answering(&[("Stock::reserve", 1, 1), ("release", 2, 2)]);
    let mut app = stock("rename-unknown", fake).await;
    // a `from` the Interface did not have
    let d = refused(
        &mut app,
        SOURCE,
        found(&[rename("reservee", "Stock::reserve")]),
    )
    .await;
    assert_eq!(d.code, "input.rename-unknown");
    assert_eq!(d.values["key"], "reservee");
    assert_eq!(d.values["field"], "from");
    assert_eq!(subjects(&d), ["core", "stock"]);
    // a `to` the provider did not find
    let d = refused(
        &mut app,
        SOURCE,
        found(&[rename("reserve", "Stock::nothing")]),
    )
    .await;
    assert_eq!(d.code, "input.rename-unknown");
    assert_eq!(d.values["key"], "Stock::nothing");
    assert_eq!(d.values["field"], "to");
    // an Interface that is new has no key to rename
    let err = app
        .put_interface_of(
            "core",
            "fresh",
            SOURCE,
            found(&[rename("reserve", "release")]),
        )
        .await
        .unwrap_err();
    let d = app.diagnose(&err).await.remove(0);
    assert_eq!(
        (d.code.as_str(), d.values["field"].as_str()),
        ("input.rename-unknown", Some("from"))
    );
}

#[tokio::test]
async fn a_rename_named_twice_on_either_side_is_refused_naming_the_key_and_the_side() {
    let fake = Fake::answering(&[("x", 1, 1), ("y", 2, 2)]);
    let mut app = stock("rename-conflict", fake).await;
    let d = refused(
        &mut app,
        SOURCE,
        found(&[rename("reserve", "x"), rename("reserve", "y")]),
    )
    .await;
    assert_eq!(d.code, "input.rename-conflict");
    assert_eq!(d.values["key"], "reserve");
    assert_eq!(d.values["side"], "from");
    let d = refused(
        &mut app,
        SOURCE,
        found(&[rename("reserve", "x"), rename("release", "x")]),
    )
    .await;
    assert_eq!(d.code, "input.rename-conflict");
    assert_eq!(d.values["key"], "x");
    assert_eq!(d.values["side"], "to");
    // the Items that are written keep the rule they had, with the side they name
    let d = refused(
        &mut app,
        "fn x()\nfn y()\n",
        ItemsInput::Written(vec![
            ItemInput {
                replaces: Some("reserve".to_string()),
                ..item("x", 1)
            },
            ItemInput {
                replaces: Some("reserve".to_string()),
                ..item("y", 2)
            },
        ]),
    )
    .await;
    assert_eq!(d.code, "input.rename-conflict");
    assert_eq!(d.values["side"], "replaces");
}

#[tokio::test]
async fn a_rename_whose_from_the_provider_still_finds_is_refused_like_a_replaces_that_is_kept() {
    let fake = Fake::answering(&[("reserve", 1, 1), ("release", 2, 2)]);
    let mut app = stock("rename-kept", fake).await;
    let d = refused(&mut app, SOURCE, found(&[rename("release", "reserve")])).await;
    assert_eq!(d.code, "input.replaces-kept");
    assert_eq!(d.values["key"], "release");
    assert_eq!(subjects(&d), ["core", "stock"]);
}

#[tokio::test]
async fn a_key_the_provider_gives_twice_is_the_domains_to_refuse_and_a_rename_does_not_hide_it() {
    let fake = Fake::answering(&[("x", 1, 1), ("x", 2, 2)]);
    let mut app = stock("key-twice", fake).await;
    let d = refused(&mut app, SOURCE, found(&[rename("reserve", "x")])).await;
    assert_eq!(d.code, "interface.key-taken");
}

#[tokio::test]
async fn the_provider_is_asked_before_the_store_is_read_so_a_write_meanwhile_is_not_a_conflict() {
    let path = dir("before-read").join("chidev.db");
    let mut other = open(&path, 1000).await;
    let other = {
        // set the project up through the second connection, then lend it to the provider's "meanwhile"
        other.init().await.unwrap();
        other.add_context("core", &[]).await.unwrap();
        Arc::new(tokio::sync::Mutex::new(other))
    };
    let mut fake = Fake::answering(&[("reserve", 1, 1)]);
    let writer = Arc::clone(&other);
    fake.meanwhile = Some(Arc::new(move || {
        let writer = Arc::clone(&writer);
        Box::pin(async move {
            // someone else writes while the provider runs
            writer
                .lock()
                .await
                .add_context("elsewhere", &[])
                .await
                .unwrap();
        })
    }));
    let mut app = open(&path, 0).await.with_items(fake);
    app.put_interface_of("core", "stock", SOURCE, found(&[]))
        .await
        .expect("the store is read after the provider answered, so it is read as it stands");
    let p = app.store().project().await.unwrap().unwrap();
    assert_eq!(p.contexts().count(), 2, "the other write is in the model");
}

#[tokio::test]
async fn a_provider_that_fails_refuses_the_whole_change_set_and_names_each_interface_it_failed_for()
{
    let fake = Fake::with(
        Arc::new(|_, interface| match interface {
            "none" => Err(ItemsError::NoLanguage),
            _ => Ok("rust".to_string()),
        }),
        Arc::new(|_, source| {
            if source.contains("broken") {
                Err(ItemsError::Failed("exit 1".to_string()))
            } else {
                Ok(vec![FoundItem {
                    key: "ok".to_string(),
                    lines: (1, 1),
                }])
            }
        }),
    );
    let mut app = stock("set", fake).await;
    let before = app.store().revision().await.unwrap();
    let interface = |name: &str, source: &str| {
        PutInput::Interface(InterfacePut {
            context: "core".to_string(),
            name: AssetName::try_from(name).unwrap(),
            source: source.to_string(),
            items: found(&[]),
        })
    };
    let set = ChangeSetInput {
        change_request: None,
        puts: vec![
            PutInput::Contract(ContractPut {
                context: "core".to_string(),
                name: AssetName::try_from("stock").unwrap(),
                title: "Stock".to_string(),
                clauses: Vec::new(),
            }),
            interface("good", "fn ok()\n"),
            interface("none", "fn ok()\n"),
            interface("broken", "fn broken()\n"),
        ],
        retires: Vec::new(),
    };
    let err = app.apply_change_set(&set).await.unwrap_err();
    assert!(
        matches!(err, AppError::Many(ref all) if all.len() == 2),
        "{err:?}"
    );
    let codes: Vec<(String, Vec<String>)> = app
        .diagnose(&err)
        .await
        .into_iter()
        .map(|d| {
            (
                d.code.clone(),
                subjects(&d).iter().map(|s| s.to_string()).collect(),
            )
        })
        .collect();
    assert_eq!(
        codes,
        [
            (
                "input.items-required".to_string(),
                vec!["core".to_string(), "none".to_string()]
            ),
            (
                "items.failed".to_string(),
                vec!["core".to_string(), "broken".to_string()]
            ),
        ]
    );
    assert_eq!(
        app.store().revision().await.unwrap(),
        before,
        "nothing kept"
    );
}

// ---- items_rekey ----

/// A project with the Context `core` and its Interface `stock`, put with `source` and the Items written, over
/// `fake`.
async fn kept(name: &str, fake: Fake, source: &str, items: &[ItemInput]) -> Stock {
    let mut app = open(&dir(name).join("chidev.db"), 0).await.with_items(fake);
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_interface("core", "stock", source, items)
        .await
        .unwrap();
    app
}

fn keys_of(items: &[(String, Uuid, u64)]) -> Vec<&str> {
    items.iter().map(|i| i.0.as_str()).collect()
}

fn changed(from: &str, to: &str) -> KeyChanged {
    KeyChanged {
        from: from.to_string(),
        to: to.to_string(),
    }
}

#[tokio::test]
async fn a_rekey_changes_the_keys_and_moves_no_revision_when_the_lines_declare_the_same() {
    let fake = Fake::answering(&[("Stock::reserve", 1, 1), ("Stock::release", 2, 2)]);
    let mut app = stock("rekey-keys", fake.clone()).await;
    let (revision, before) = stock_items(&app).await;
    let store = app.store().revision().await.unwrap();
    let done = app.items_rekey("core", "stock").await.unwrap().value;
    assert_eq!(
        done.rekeyed,
        [
            changed("reserve", "Stock::reserve"),
            changed("release", "Stock::release")
        ]
    );
    assert!(done.revised.is_empty() && done.unmatched.is_empty());
    assert_eq!(done.revision, revision);
    // the provider was asked for the code the Interface keeps
    assert_eq!(fake.asked(), [("rust".to_string(), SOURCE.to_string())]);
    let (now, after) = stock_items(&app).await;
    assert_eq!(now, revision, "the Interface did not move");
    assert_eq!(keys_of(&after), ["Stock::reserve", "Stock::release"]);
    // the same ids and the same revisions: a new key is not a change of what an Item declares
    assert_eq!(
        after.iter().map(|i| (i.1, i.2)).collect::<Vec<_>>(),
        before.iter().map(|i| (i.1, i.2)).collect::<Vec<_>>()
    );
    assert!(app.store().revision().await.unwrap() > store, "it is kept");
}

#[tokio::test]
async fn a_rekey_to_lines_that_declare_something_else_revises_that_item_and_says_so() {
    let source = "fn reserve(\n    n: u8)\nfn release()\n";
    let fake = Fake::answering(&[("reserve", 1, 2), ("release", 3, 3)]);
    let mut app = kept(
        "rekey-lines",
        fake,
        source,
        &[item("reserve", 1), item("release", 3)],
    )
    .await;
    let (_, before) = stock_items(&app).await;
    let done = app.items_rekey("core", "stock").await.unwrap().value;
    // the keys are the provider's already; only the lines of the first differ, and so does its declaration
    assert!(done.rekeyed.is_empty());
    assert_eq!(
        done.revised,
        [ItemRevised {
            key: "reserve".to_string(),
            from: 1,
            to: 2
        }]
    );
    assert_eq!(done.revision, 2);
    let (revision, after) = stock_items(&app).await;
    assert_eq!(revision, 2);
    assert_eq!(
        after.iter().map(|i| i.2).collect::<Vec<_>>(),
        [2, 1],
        "the first Item is at its next revision and the other stays"
    );
    assert_eq!(after[0].1, before[0].1, "the id is kept");
    let p = app.store().project().await.unwrap().unwrap();
    let signature = p
        .contexts()
        .flat_map(|c| c.interfaces())
        .flat_map(|i| i.items())
        .map(|it| it.signature().to_string())
        .collect::<Vec<_>>();
    assert_eq!(signature, ["fn reserve(\n    n: u8)", "fn release()"]);
}

#[tokio::test]
async fn a_rekey_that_revises_an_item_a_case_cites_says_the_citation_is_behind() {
    let source = "fn reserve(\n    n: u8)\nfn release()\n";
    let fake = Fake::answering(&[("reserve", 1, 2), ("release", 3, 3)]);
    let mut app = kept(
        "rekey-stale-cite",
        fake,
        source,
        &[item("reserve", 1), item("release", 3)],
    )
    .await;
    app.put_harness(
        "core",
        "tests",
        "fn reserves() {}\n",
        &[CaseInput {
            name: "reserves".to_string(),
            lines: (1, 1),
            items: vec![ItemRef {
                context: None,
                interface: AssetName::try_from("stock").unwrap(),
                key: "reserve".to_string(),
            }],
            clauses: Vec::new(),
        }],
    )
    .await
    .unwrap();
    let done = app.items_rekey("core", "stock").await.unwrap();
    assert_eq!(done.value.revised.len(), 1);
    // The edit made a citation stale, and says so; it did not need a put to find it.
    let stale: Vec<_> = done
        .findings
        .iter()
        .filter(|f| f.rule.as_str() == "cite.stale")
        .collect();
    assert_eq!(stale.len(), 1, "{:?}", done.findings);
    assert_eq!(stale[0].about.as_deref(), Some("case reserves"));
    // A rekey that moves nothing says nothing.
    let again = app.items_rekey("core", "stock").await.unwrap();
    assert!(again.value.revised.is_empty());
    assert!(again.findings.is_empty(), "{:?}", again.findings);
}

#[tokio::test]
async fn a_rekey_pairs_no_item_with_a_declaration_it_shares_lines_with_others_over() {
    // one Item on two lines, where the provider finds two declarations: the Item is listed, not rekeyed
    let source = "fn a();\nfn b();\nfn c();\n";
    let fake = Fake::answering(&[("A", 1, 1), ("B", 2, 2), ("C", 3, 3)]);
    let mut app = kept(
        "rekey-many",
        fake,
        source,
        &[
            ItemInput {
                key: "a and b".to_string(),
                lines: (1, 2),
                replaces: None,
            },
            item("c", 3),
        ],
    )
    .await;
    let (revision, before) = stock_items_of(&app, "fn a").await;
    let done = app.items_rekey("core", "stock").await.unwrap().value;
    assert_eq!(done.rekeyed, [changed("c", "C")]);
    assert_eq!(done.unmatched, ["a and b"]);
    let (now, after) = stock_items_of(&app, "fn a").await;
    assert_eq!(now, revision);
    assert_eq!(keys_of(&after), ["a and b", "C"]);
    assert_eq!(after[0], before[0], "the one left is as it was");

    // the reverse: two Items on the lines of one declaration
    let fake = Fake::answering(&[("AB", 1, 2)]);
    let mut app = kept(
        "rekey-many-kept",
        fake,
        "fn a();\nfn b();\n",
        &[item("a", 1), item("b", 2)],
    )
    .await;
    let store_before = app.store().revision().await.unwrap();
    let done = app.items_rekey("core", "stock").await.unwrap().value;
    assert!(done.rekeyed.is_empty());
    assert_eq!(done.unmatched, ["a", "b"]);
    assert_eq!(
        app.store().revision().await.unwrap(),
        store_before,
        "nothing changed, so nothing is kept"
    );
}

#[tokio::test]
async fn a_rekey_that_gives_two_items_one_key_is_refused_naming_the_key_and_changes_nothing() {
    // two paired Items taking one key
    let fake = Fake::answering(&[("x", 1, 1), ("x", 2, 2)]);
    let mut app = stock("rekey-twice", fake).await;
    let before = (
        stock_items(&app).await,
        app.store().revision().await.unwrap(),
    );
    let err = app.items_rekey("core", "stock").await.unwrap_err();
    let d = app.diagnose(&err).await;
    assert_eq!(d.len(), 1, "{d:#?}");
    assert_eq!(d[0].code, "interface.key-taken");
    assert_eq!(
        (
            stock_items(&app).await,
            app.store().revision().await.unwrap()
        ),
        before
    );

    // a key an Item that is not paired keeps
    let source = "fn reserve()\n\nfn release()\n";
    let fake = Fake::answering(&[("release", 1, 1)]);
    let mut app = kept(
        "rekey-kept-key",
        fake,
        source,
        &[item("reserve", 1), item("release", 3)],
    )
    .await;
    let before = (
        stock_items_of(&app, "reserve").await,
        app.store().revision().await.unwrap(),
    );
    let err = app.items_rekey("core", "stock").await.unwrap_err();
    let d = app.diagnose(&err).await;
    assert_eq!(d[0].code, "interface.key-taken");
    assert_eq!(
        (
            stock_items_of(&app, "reserve").await,
            app.store().revision().await.unwrap()
        ),
        before
    );
}

#[tokio::test]
async fn a_rekey_with_no_provider_or_a_failing_one_is_refused_by_name_and_changes_nothing() {
    for (error, code) in [
        (ItemsError::Failed("exit 3".into()), "items.failed"),
        (ItemsError::Protocol("lines".into()), "items.protocol"),
        (ItemsError::NoProvider, "items.no-provider"),
    ] {
        let mut app = stock("rekey-provider", Fake::failing(error)).await;
        let before = (
            stock_items(&app).await,
            app.store().revision().await.unwrap(),
        );
        let err = app.items_rekey("core", "stock").await.unwrap_err();
        let d = app.diagnose(&err).await;
        assert_eq!(d.len(), 1, "{d:#?}");
        assert_eq!(d[0].code, code);
        assert_eq!(subjects(&d[0]), ["core", "stock"]);
        assert_eq!(
            (
                stock_items(&app).await,
                app.store().revision().await.unwrap()
            ),
            before
        );
    }
    // no language laid out for the Interface
    let fake = Fake::with(
        Arc::new(|_, _| Err(ItemsError::NoLanguage)),
        Arc::new(|_, _| Ok(Vec::new())),
    );
    let mut app = stock("rekey-language", fake).await;
    let err = app.items_rekey("core", "stock").await.unwrap_err();
    assert_eq!(app.diagnose(&err).await[0].code, "input.items-required");
    // an application with no source of Items at all
    let mut bare = open(&dir("rekey-bare").join("chidev.db"), 0).await;
    bare.init().await.unwrap();
    bare.add_context("core", &[]).await.unwrap();
    bare.put_interface("core", "stock", SOURCE, &[item("reserve", 1)])
        .await
        .unwrap();
    let err = bare.items_rekey("core", "stock").await.unwrap_err();
    assert_eq!(bare.diagnose(&err).await[0].code, "items.no-provider");
}

#[tokio::test]
async fn a_rekey_of_what_is_not_there_is_refused() {
    let mut app = stock("rekey-unknown", Fake::answering(&[])).await;
    assert!(matches!(
        app.items_rekey("nowhere", "stock").await,
        Err(AppError::UnknownContext(_))
    ));
    assert!(matches!(
        app.items_rekey("core", "nothing").await,
        Err(AppError::UnknownAsset { .. })
    ));
    app.retire_interface("core", "stock").await.unwrap();
    assert!(matches!(
        app.items_rekey("core", "stock").await,
        Err(AppError::AssetRetired { .. })
    ));
}

#[tokio::test]
async fn a_rekey_is_kept_as_the_store_stood_and_a_store_that_moved_refuses_it() {
    let path = dir("rekey-conflict").join("chidev.db");
    let other = {
        let mut other = open(&path, 1000).await;
        other.init().await.unwrap();
        other.add_context("core", &[]).await.unwrap();
        other
            .put_interface("core", "stock", SOURCE, &[item("reserve", 1)])
            .await
            .unwrap();
        Arc::new(tokio::sync::Mutex::new(other))
    };
    let mut fake = Fake::answering(&[("Stock::reserve", 1, 1)]);
    let writer = Arc::clone(&other);
    fake.meanwhile = Some(Arc::new(move || {
        let writer = Arc::clone(&writer);
        Box::pin(async move {
            writer
                .lock()
                .await
                .add_context("elsewhere", &[])
                .await
                .unwrap();
        })
    }));
    let mut app = open(&path, 0).await.with_items(fake);
    let err = app.items_rekey("core", "stock").await.unwrap_err();
    assert!(
        matches!(
            err,
            AppError::Store(chidev_core::domain::repository::StoreError::Conflict)
        ),
        "{err:?}"
    );
    let (_, items) = stock_items(&app).await;
    assert_eq!(keys_of(&items), ["reserve"], "nothing was kept");
}

#[tokio::test]
async fn a_store_of_schema_1_comes_to_the_providers_keys_with_the_ids_and_the_citations_kept() {
    // an Interface whose Items are named by what their signatures were, and a Case that cites one
    let path = dir("legacy").join("chidev.db");
    {
        let mut app = open(&path, 0).await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.put_interface(
            "core",
            "stock",
            SOURCE,
            &[item("fn reserve()", 1), item("fn release()", 2)],
        )
        .await
        .unwrap();
        app.put_harness(
            "core",
            "stock-tests",
            "fn reserve_refuses() {}\n",
            &[CaseInput {
                name: "reserve_refuses".to_string(),
                lines: (1, 1),
                items: vec![ItemRef {
                    context: None,
                    interface: AssetName::try_from("stock").unwrap(),
                    key: "fn reserve()".to_string(),
                }],
                clauses: Vec::new(),
            }],
        )
        .await
        .unwrap();
        app.into_store().close().await.unwrap();
    }
    // write the Project as schema 1 did: an Item has its signature and no key
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let text: String = conn
            .query_row("SELECT doc FROM project", [], |r| r.get(0))
            .unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["schema"] = serde_json::json!(1);
        for context in doc["contexts"].as_array_mut().unwrap() {
            let Some(interfaces) = context["interfaces"].as_array_mut() else {
                continue;
            };
            for item in interfaces
                .iter_mut()
                .flat_map(|i| i["items"].as_array_mut().unwrap())
            {
                let key = item.as_object_mut().unwrap().remove("key").unwrap();
                item["signature"] = key;
            }
        }
        conn.execute("UPDATE project SET schema = 1, doc = ?1", [doc.to_string()])
            .unwrap();
    }
    let fake = Fake::answering(&[("reserve", 1, 1), ("release", 2, 2)]);
    let mut app = open(&path, 5000).await.with_items(fake);
    let (revision, before) = stock_items(&app).await;
    // the decoded keys are the signatures
    assert_eq!(keys_of(&before), ["fn reserve()", "fn release()"]);
    let done = app.items_rekey("core", "stock").await.unwrap().value;
    assert_eq!(
        done.rekeyed,
        [
            changed("fn reserve()", "reserve"),
            changed("fn release()", "release")
        ]
    );
    assert!(done.revised.is_empty() && done.unmatched.is_empty());
    let (now, after) = stock_items(&app).await;
    assert_eq!(now, revision);
    assert_eq!(keys_of(&after), ["reserve", "release"]);
    assert_eq!(
        after.iter().map(|i| (i.1, i.2)).collect::<Vec<_>>(),
        before.iter().map(|i| (i.1, i.2)).collect::<Vec<_>>(),
        "the ids and the revisions are the ones the schema 1 store had"
    );
    // the Case still cites the Item, and no citation went stale
    let p = app.store().project().await.unwrap().unwrap();
    let cited: Vec<_> = p
        .contexts()
        .flat_map(|c| c.harnesses())
        .flat_map(|h| h.cases())
        .flat_map(|c| c.cites().items().keys().map(|id| id.as_uuid()))
        .collect();
    assert_eq!(cited, [before[0].1]);
    assert!(chidev_core::domain::service::cite::stale_cites(&p).is_empty());
}
