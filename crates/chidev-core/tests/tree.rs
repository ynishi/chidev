//! The copies of the placed assets in the working tree, through the use cases. The working tree is a double that
//! says what is in each file and counts how it is asked, except where a test needs the real one over a real
//! `chidev.toml` and real files. What the rules say of each state of a copy is in the unit tests of the domain; what
//! is tried here is the wiring: which paths read the tree, how often, and what comes out of them.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chidev_core::app::commit::Gated;
use chidev_core::app::ports::external::ExternalReport;
use chidev_core::app::ports::measure::{BoxFuture, RunnerError};
use chidev_core::app::ports::tree::{TreeTarget, WorkingTree};
use chidev_core::app::ports::{
    AssetKind, AssetNames, IdSource, Invocation, InvocationSeq, JudgedReport, JudgedRepository,
    Store, StoreRevision,
};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, DecisionMakerInput,
    ExampleInput, ExampleTexts, Found, Intent, ItemInput, ItemRef, Registry, ResolutionInput,
    TreeAsset,
};
use chidev_core::domain::check::{Layer, Level, Moment, Origin, Selector, Setting};
use chidev_core::domain::entity::change_request::ChangeRequest;
use chidev_core::domain::entity::crux::Crux;
use chidev_core::domain::entity::measurement::Measurement;
use chidev_core::domain::entity::pin::Pin;
use chidev_core::domain::entity::project::Project;
use chidev_core::domain::repository::{
    ChangeRequestRepository, CruxRepository, PinRepository, ProjectRepository, StoreError,
};
use chidev_core::domain::vo::digest::Digest;
use chidev_core::domain::vo::id::{ChangeRequestId, ContextId, CruxId, InterfaceId, PinId};
use chidev_core::domain::vo::measure::MeasureKind;
use chidev_core::domain::vo::name::AssetName;
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::domain::vo::tree::CopyRead;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::project::{self, FILE};
use chidev_core::infra::runner::{self, ProcessRunner};
use chidev_core::infra::sqlite::SqliteStore;
use uuid::Uuid;

#[derive(Default)]
struct Counter(u128);

impl IdSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_u128(self.0)
    }
}

fn clock() -> FixedClock {
    FixedClock(Timestamp::from_millis(1_000))
}

type TestApp = App<SqliteStore, Counter>;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A directory of its own, removed when it is dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(what: &str) -> Dir {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "chidev-tree-{what}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    async fn store(&self) -> SqliteStore {
        SqliteStore::open(self.0.join("store").join("chidev.db"))
            .await
            .unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The code pinned for the Interface `api`, and the source of the Harness `tests`.
const CODE: &str = "fn push() {}";
const SOURCE: &str = "fn pushes() {}\n";

/// A working tree that says, by an asset's name, what its file holds (an asset it is not told of is unplaced), and
/// counts how many times it is asked and what for.
#[derive(Debug, Clone, Default)]
struct FakeTree {
    asked: Arc<Mutex<Vec<Vec<TreeTarget>>>>,
    copies: Arc<Mutex<BTreeMap<String, CopyRead>>>,
}

impl FakeTree {
    /// What the file of the asset `name` holds now.
    fn holds(&self, name: &str, copy: CopyRead) {
        self.copies.lock().unwrap().insert(name.to_string(), copy);
    }

    /// The text `text` in the file of the asset `name`.
    fn writes(&self, name: &str, text: &str) {
        self.holds(name, CopyRead::Read(Digest::of(text.as_bytes())));
    }

    /// How many times the tree was asked.
    fn reads(&self) -> usize {
        self.asked.lock().unwrap().len()
    }

    /// The targets of the `n`th ask, by name.
    fn targets(&self, n: usize) -> Vec<String> {
        self.asked.lock().unwrap()[n]
            .iter()
            .map(|t| match t {
                TreeTarget::Interface { name, .. } => format!("interface {name}"),
                TreeTarget::Harness { name, .. } => format!("harness {name}"),
            })
            .collect()
    }
}

impl WorkingTree for FakeTree {
    fn digests_of<'a>(
        &'a self,
        targets: &'a [TreeTarget],
    ) -> BoxFuture<'a, Vec<(TreeTarget, CopyRead)>> {
        Box::pin(async move {
            self.asked.lock().unwrap().push(targets.to_vec());
            let copies = self.copies.lock().unwrap();
            targets
                .iter()
                .map(|t| {
                    let (TreeTarget::Interface { name, .. } | TreeTarget::Harness { name, .. }) = t;
                    let copy = copies
                        .get(name.as_str())
                        .cloned()
                        .unwrap_or(CopyRead::Unplaced);
                    (t.clone(), copy)
                })
                .collect()
        })
    }
}

fn clauses() -> Vec<ClauseInput> {
    vec![ClauseInput {
        label: "C1".into(),
        text: "push then pop".into(),
        replaces: None,
    }]
}

fn item(key: &str) -> Vec<ItemInput> {
    vec![ItemInput {
        key: key.into(),
        lines: (1, 1),
        replaces: None,
    }]
}

/// A Project in the Context `core`: the Interface `api`, pinned, with the Harness `tests` that checks it; the
/// Interface `lib` with no Pin; and the Interface `free`, with no Pin either.
async fn populate(app: &mut TestApp) {
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract("core", "stack", "Stack", &clauses())
        .await
        .unwrap();
    for (name, key) in [("api", "push"), ("lib", "other"), ("free", "spare")] {
        app.put_interface("core", name, &format!("fn {key}();\n"), &item(key))
            .await
            .unwrap();
    }
    app.put_harness(
        "core",
        "tests",
        SOURCE,
        &[CaseInput {
            name: "pushes".into(),
            lines: (1, 1),
            items: vec![ItemRef {
                context: None,
                interface: AssetName::try_from("api").unwrap(),
                key: "push".into(),
            }],
            clauses: vec![ClauseRef {
                context: None,
                contract: AssetName::try_from("stack").unwrap(),
                label: "C1".into(),
            }],
        }],
    )
    .await
    .unwrap();
    app.pin("core", "api", CODE).await.unwrap();
}

/// The Project of [`populate`] over a store of its own, with `tree` as its working tree.
async fn world(dir: &Dir, tree: &FakeTree) -> TestApp {
    let mut app = App::new(dir.store().await, Counter::default(), clock()).with_tree(tree.clone());
    populate(&mut app).await;
    app
}

/// The findings of the family `tree.` among `found`, as `(rule, about)`.
fn tree_findings(found: &[Found]) -> Vec<(String, Option<String>)> {
    found
        .iter()
        .filter(|f| f.rule.as_str().starts_with("tree."))
        .map(|f| (f.rule.to_string(), f.about.clone()))
        .collect()
}

fn alice() -> DecisionMakerInput {
    DecisionMakerInput::Person("alice".into())
}

fn request(title: &str) -> ChangeRequestInput {
    ChangeRequestInput {
        title: title.into(),
        situation: None,
        intent: Intent::KeepBehavior,
        parent: None,
        issuer: None,
        targets: vec![],
        keep: vec![],
        assumptions: vec![],
        addresses: vec![],
    }
}

#[tokio::test]
async fn check_and_status_read_the_tree_once_each_and_an_edit_a_pin_and_a_resolution_never() {
    let dir = Dir::new("counts");
    let tree = FakeTree::default();
    let mut app = world(&dir, &tree).await;
    // the puts and the pin of the Project are edits and a confirmation: none of them looked at the tree
    assert_eq!(tree.reads(), 0);

    app.check(None, &[]).await.unwrap();
    assert_eq!(tree.reads(), 1);
    // every Interface, and the Harness that holds a source, asked together
    let mut asked = tree.targets(0);
    asked.sort();
    assert_eq!(
        asked,
        [
            "harness tests",
            "interface api",
            "interface free",
            "interface lib"
        ]
    );
    app.status().await.unwrap();
    assert_eq!(tree.reads(), 2);

    // an edit, a pin and a resolution
    app.put_contract("core", "stack", "Stack again", &clauses())
        .await
        .unwrap();
    app.pin("core", "api", "fn push() { 2 }").await.unwrap();
    app.open_change_request(&request("r")).await.unwrap();
    app.resolve_change_request("r", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();
    assert_eq!(tree.reads(), 2);
}

#[tokio::test]
async fn a_question_reads_the_tree_once_and_keeps_that_the_copy_moved() {
    let dir = Dir::new("ask");
    let tree = FakeTree::default();
    let mut app = world(&dir, &tree).await;
    tree.writes("api", "fn push() { changed }");

    let asked = app.check_run().await.unwrap();
    assert_eq!(tree.reads(), 1);
    assert_eq!(
        tree_findings(&asked.findings),
        [(
            "tree.code-moved".to_string(),
            Some("interface api".to_string())
        )]
    );
    let moved = asked
        .findings
        .iter()
        .find(|f| f.rule.as_str() == "tree.code-moved")
        .unwrap();
    assert_eq!(moved.level, Level::Warning);

    // the run that asked is kept with the finding it counted
    let kept: Vec<Invocation> = app
        .store()
        .invocations()
        .await
        .unwrap()
        .into_iter()
        .map(|(_, i)| i)
        .filter(|i| i.check.as_str() == "tree.code-moved")
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].moment, Moment::Check);
    assert_eq!(kept[0].warnings, 1);
    // and a check of the same state finds the same
    let checked = app.check(None, &[]).await.unwrap();
    assert_eq!(
        tree_findings(&checked.findings),
        tree_findings(&asked.findings)
    );
}

#[tokio::test]
async fn a_copy_that_is_the_same_is_not_found_and_one_that_cannot_be_looked_at_is_unread() {
    let dir = Dir::new("states");
    let tree = FakeTree::default();
    let app = world(&dir, &tree).await;
    tree.writes("api", CODE);
    tree.writes("tests", SOURCE);
    assert!(
        tree_findings(&app.check(None, &[]).await.unwrap().findings).is_empty(),
        "a copy that is the text the model holds is found by none"
    );

    // a copy that moved: the Interface's by `tree.code-moved`, the Harness's by `tree.harness-moved`
    tree.writes("api", "fn push() { 2 }");
    tree.writes("tests", "fn pushes() { 2 }\n");
    assert_eq!(
        tree_findings(&app.check(None, &[]).await.unwrap().findings),
        [
            (
                "tree.code-moved".to_string(),
                Some("interface api".to_string())
            ),
            (
                "tree.harness-moved".to_string(),
                Some("case pushes".to_string())
            )
        ]
    );

    // files that are not there or cannot be read, with a Pin (`api`) and without (`lib`)
    tree.holds("api", CopyRead::Missing);
    tree.holds("lib", CopyRead::Missing);
    tree.holds(
        "tests",
        CopyRead::Unreadable("tests.rs: not UTF-8 text".into()),
    );
    let found = app.check(None, &[]).await.unwrap().findings;
    assert_eq!(
        tree_findings(&found),
        [
            ("tree.unread".to_string(), Some("interface api".to_string())),
            ("tree.unread".to_string(), Some("interface lib".to_string())),
            ("tree.unread".to_string(), Some("case pushes".to_string())),
        ]
    );
    assert!(found.iter().any(|f| f.message.contains("is missing")));
    assert!(found.iter().any(|f| {
        f.message
            .ends_with("cannot be read: tests.rs: not UTF-8 text")
    }));
}

#[tokio::test]
async fn status_names_what_has_no_copy_what_has_no_pin_and_what_moved_and_finds_only_the_last() {
    let dir = Dir::new("status");
    let tree = FakeTree::default();
    let app = world(&dir, &tree).await;
    // `api` moved, `lib` has a file and no Pin, `free` and the Harness `tests` are placed nowhere
    tree.writes("api", "fn push() { 2 }");
    tree.writes("lib", "fn other() {}");

    let status = app.status().await.unwrap();
    let asset = |name: &str| TreeAsset {
        context: "core".to_string(),
        name: name.to_string(),
    };
    let tree_status = status.tree.expect("the tree was looked at");
    assert_eq!(tree_status.unplaced.interfaces, [asset("free")]);
    assert_eq!(tree_status.unplaced.harnesses, [asset("tests")]);
    assert_eq!(tree_status.unpinned, [asset("lib")]);
    assert_eq!(tree_status.moved.len(), 1);
    assert_eq!(
        (
            tree_status.moved[0].context.as_str(),
            tree_status.moved[0].interface.as_str(),
            tree_status.moved[0].declared
        ),
        ("core", "api", "off")
    );
    // what has no copy, and what has a copy and no Pin, is no finding
    assert_eq!(
        tree_findings(&status.findings),
        [(
            "tree.code-moved".to_string(),
            Some("interface api".to_string())
        )]
    );
}

/// The policy that sets the level of each of `entries` (a rule or a family with its dot).
fn registry_with(entries: &[(&str, Level)]) -> Registry {
    let layer = Layer {
        origin: Origin::Project,
        entries: entries
            .iter()
            .map(|(selector, level)| {
                (
                    Selector::parse(selector).unwrap(),
                    Setting {
                        level: Some(*level),
                        ..Setting::default()
                    },
                )
            })
            .collect(),
    };
    Registry::builtin(&[layer]).unwrap()
}

#[tokio::test]
async fn a_project_that_turns_every_tree_check_off_reads_nothing_and_one_check_left_on_reads_it() {
    let tree = FakeTree::default();
    tree.writes("api", "fn push() { 2 }");

    let dir = Dir::new("off");
    let mut app = App::with_registry(
        dir.store().await,
        Counter::default(),
        registry_with(&[("tree.", Level::Off)]),
        clock(),
    )
    .with_tree(tree.clone());
    populate(&mut app).await;
    assert!(tree_findings(&app.check(None, &[]).await.unwrap().findings).is_empty());
    app.check_run().await.unwrap();
    let status = app.status().await.unwrap();
    assert_eq!(tree.reads(), 0, "nothing looks at a tree no check reads");
    assert_eq!(
        status.tree, None,
        "and status does not say what it did not look at"
    );

    // one left on: the tree is read, and it is the one that finds
    let dir = Dir::new("one");
    let mut app = App::with_registry(
        dir.store().await,
        Counter::default(),
        registry_with(&[("tree.", Level::Off), ("tree.code-moved", Level::Note)]),
        clock(),
    )
    .with_tree(tree.clone());
    populate(&mut app).await;
    let found = app.check(None, &[]).await.unwrap().findings;
    assert_eq!(tree.reads(), 1);
    assert_eq!(
        tree_findings(&found),
        [(
            "tree.code-moved".to_string(),
            Some("interface api".to_string())
        )]
    );
}

#[tokio::test]
async fn an_application_given_no_working_tree_places_nothing() {
    let dir = Dir::new("none");
    let mut app = App::new(dir.store().await, Counter::default(), clock());
    populate(&mut app).await;
    let status = app.status().await.unwrap();
    assert!(tree_findings(&status.findings).is_empty());
    let tree = status.tree.expect("looked at");
    assert_eq!(tree.unplaced.interfaces.len(), 3);
    assert!(tree.unpinned.is_empty() && tree.moved.is_empty());
}

/// What a store that is told the code of a Pin gives back.
#[derive(Debug, Clone, Copy)]
enum Kept {
    /// The code as it was pinned.
    AsPinned,
    /// Text that is not the code the Pin digests.
    Other,
    /// None: the Pin was made before the code was kept.
    Nothing,
}

/// A store as it is, except for the code it says it kept for a Pin.
struct Keeping(SqliteStore, Kept);

impl ProjectRepository for Keeping {
    async fn project(&self) -> Result<Option<Project>, StoreError> {
        self.0.project().await
    }
}

impl PinRepository for Keeping {
    async fn pin_of(&self, interface: InterfaceId) -> Result<Option<Pin>, StoreError> {
        self.0.pin_of(interface).await
    }

    async fn pins(&self) -> Result<Vec<Pin>, StoreError> {
        self.0.pins().await
    }
}

impl ChangeRequestRepository for Keeping {
    async fn change_request(
        &self,
        id: ChangeRequestId,
    ) -> Result<Option<ChangeRequest>, StoreError> {
        self.0.change_request(id).await
    }

    async fn change_requests(&self) -> Result<Vec<ChangeRequest>, StoreError> {
        self.0.change_requests().await
    }
}

impl CruxRepository for Keeping {
    async fn crux(&self, id: CruxId) -> Result<Option<Crux>, StoreError> {
        self.0.crux(id).await
    }

    async fn cruxes(&self) -> Result<Vec<Crux>, StoreError> {
        self.0.cruxes().await
    }

    async fn cruxes_raised_in(&self, raised_in: ChangeRequestId) -> Result<Vec<Crux>, StoreError> {
        self.0.cruxes_raised_in(raised_in).await
    }
}

impl AssetNames for Keeping {
    async fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &AssetName,
    ) -> Result<Option<Uuid>, StoreError> {
        self.0.asset_id(context, kind, name).await
    }

    async fn asset_name(&self, id: Uuid) -> Result<Option<AssetName>, StoreError> {
        self.0.asset_name(id).await
    }
}

impl JudgedRepository for Keeping {
    async fn judged_reports(&self) -> Result<Vec<JudgedReport>, StoreError> {
        self.0.judged_reports().await
    }
}

impl Store for Keeping {
    async fn external_reports(&self) -> Result<Vec<ExternalReport>, StoreError> {
        self.0.external_reports().await
    }

    async fn latest_measurements(&self) -> Result<Vec<Measurement>, StoreError> {
        self.0.latest_measurements().await
    }

    async fn latest_measurements_of(
        &self,
        kind: MeasureKind,
    ) -> Result<Vec<Measurement>, StoreError> {
        self.0.latest_measurements_of(kind).await
    }

    async fn measurements(&self) -> Result<Vec<Measurement>, StoreError> {
        self.0.measurements().await
    }

    async fn invocations(&self) -> Result<Vec<(InvocationSeq, Invocation)>, StoreError> {
        self.0.invocations().await
    }

    async fn clear_freed(&self) -> Result<(), StoreError> {
        self.0.clear_freed().await
    }

    async fn pin_code(&self, pin: PinId) -> Result<Option<String>, StoreError> {
        let kept = self.0.pin_code(pin).await?;
        Ok(match self.1 {
            Kept::AsPinned => kept,
            Kept::Other => kept.map(|c| format!("{c} // another text")),
            Kept::Nothing => None,
        })
    }

    async fn revision(&self) -> Result<StoreRevision, StoreError> {
        self.0.revision().await
    }

    async fn model_revision(&self) -> Result<StoreRevision, StoreError> {
        self.0.model_revision().await
    }

    async fn commit(&self, read_at: StoreRevision, gated: Gated) -> Result<(), StoreError> {
        self.0.commit(read_at, gated).await
    }
}

/// The Project of [`populate`] and a Harness of examples `acceptance` that builds `api`, under a store that says
/// `kept` of the code of a Pin.
async fn keeping(dir: &Dir, kept: Kept) -> App<Keeping, Counter> {
    let mut app = App::new(dir.store().await, Counter::default(), clock());
    populate(&mut app).await;
    app.put_examples_harness(
        "core",
        "acceptance",
        &["api".to_string()],
        &[ExampleInput {
            name: "a1".into(),
            clauses: vec![ClauseRef {
                context: None,
                contract: AssetName::try_from("stack").unwrap(),
                label: "C1".into(),
            }],
            example: ExampleTexts {
                args: String::new(),
                stdin: String::new(),
                expected: "#exit 0\n".into(),
                stderr: None,
            },
        }],
    )
    .await
    .unwrap();
    App::new(Keeping(app.into_store(), kept), Counter(10_000), clock())
}

fn not_kept(interface: &str) -> AppError {
    AppError::PinCodeNotKept {
        interface: AssetName::try_from(interface).unwrap(),
    }
}

#[tokio::test]
async fn code_the_store_does_not_keep_for_a_pin_is_refused_by_a_mutation_run_and_by_a_run_of_examples()
 {
    for kept in [Kept::Other, Kept::Nothing] {
        let dir = Dir::new("kept");
        let mut app = keeping(&dir, kept).await;
        assert_eq!(
            app.measure(MeasureKind::Mutate, "core", "api")
                .await
                .unwrap_err(),
            not_kept("api"),
            "{kept:?}: the mutation run"
        );
        assert_eq!(
            app.run_examples("core", "acceptance").await.unwrap_err(),
            not_kept("api"),
            "{kept:?}: the run of examples"
        );
    }
    // the same store that keeps the code as it was pinned gets as far as the runner, which is not there
    let dir = Dir::new("kept-as-pinned");
    let mut app = keeping(&dir, Kept::AsPinned).await;
    assert_eq!(
        app.measure(MeasureKind::Mutate, "core", "api")
            .await
            .unwrap_err(),
        AppError::Runner(RunnerError::Unconfigured)
    );
    assert_eq!(
        app.run_examples("core", "acceptance").await.unwrap_err(),
        AppError::Runner(RunnerError::Unconfigured)
    );
}

/// A project directory with a `chidev.toml` that lays out the Interface `api` in `code.txt` and the Harness `tests`
/// in `tests.txt`.
struct RealTree {
    dir: Dir,
}

const TABLE: &str = r#"[measure]
build = ["sh", "-c", "true"]
test = ["sh", "-c", "true"]
env = {}

[measure.mutants]
text = ["sh"]

[[measure.layout]]
context = "core"
interface = "api"
lang = "text"
path = "code.txt"

[[measure.layout]]
context = "core"
harness = "tests"
path = "tests.txt"
"#;

impl RealTree {
    fn new() -> RealTree {
        let dir = Dir::new("real");
        std::fs::create_dir_all(dir.0.join("project")).unwrap();
        let p = RealTree { dir };
        std::fs::write(p.root().join(FILE), TABLE).unwrap();
        p
    }

    fn root(&self) -> PathBuf {
        self.dir.0.join("project")
    }

    fn runner(&self) -> ProcessRunner {
        runner::load(&project::find(&self.root()).unwrap())
            .unwrap()
            .expect("a [measure] table")
    }

    fn write(&self, file: &str, text: &str) {
        std::fs::write(self.root().join(file), text).unwrap();
    }
}

#[tokio::test]
async fn the_files_a_layout_names_are_the_copies_and_a_changed_one_is_found_with_no_put_again() {
    let project = RealTree::new();
    let mut app = App::new(project.dir.store().await, Counter::default(), clock())
        .with_tree(project.runner());
    populate(&mut app).await;

    // the files hold what the model does: the Pin's code, and the Harness's source
    project.write("code.txt", CODE);
    project.write("tests.txt", SOURCE);
    let found = app.check(None, &[]).await.unwrap().findings;
    assert!(tree_findings(&found).is_empty(), "{found:?}");
    let status = app.status().await.unwrap().tree.expect("looked at");
    // `lib` and `free` are laid out nowhere
    assert_eq!(status.unplaced.interfaces.len(), 2);
    assert!(status.unplaced.harnesses.is_empty() && status.moved.is_empty());

    // the pinned file is changed on disk, and nothing is put again
    project.write("code.txt", "fn push() { 2 }");
    let found = app.check(None, &[]).await.unwrap().findings;
    assert_eq!(
        tree_findings(&found),
        [(
            "tree.code-moved".to_string(),
            Some("interface api".to_string())
        )]
    );
    assert_eq!(
        app.status()
            .await
            .unwrap()
            .tree
            .expect("looked")
            .moved
            .len(),
        1
    );
    // the same text again is the same
    project.write("code.txt", CODE);
    assert!(tree_findings(&app.check(None, &[]).await.unwrap().findings).is_empty());

    // the source of the Harness, a file that is gone, and one that is not text
    project.write("tests.txt", "fn pushes() { 2 }\n");
    std::fs::remove_file(project.root().join("code.txt")).unwrap();
    let found = app.check(None, &[]).await.unwrap().findings;
    assert_eq!(
        tree_findings(&found),
        [
            (
                "tree.harness-moved".to_string(),
                Some("case pushes".to_string())
            ),
            ("tree.unread".to_string(), Some("interface api".to_string())),
        ]
    );
    std::fs::write(project.root().join("code.txt"), [0xff, 0xfe]).unwrap();
    let found = app.check(None, &[]).await.unwrap().findings;
    let unread: Vec<&str> = found
        .iter()
        .filter(|f| f.rule.as_str() == "tree.unread")
        .map(|f| f.message.as_str())
        .collect();
    assert_eq!(unread.len(), 1, "{unread:?}");
    assert!(
        unread[0].ends_with("code.txt: not UTF-8 text"),
        "{unread:?}"
    );
    // a directory where the file is laid out
    std::fs::remove_file(project.root().join("code.txt")).unwrap();
    std::fs::create_dir(project.root().join("code.txt")).unwrap();
    let found = app.check(None, &[]).await.unwrap().findings;
    let unread: Vec<&str> = found
        .iter()
        .filter(|f| f.rule.as_str() == "tree.unread")
        .map(|f| f.message.as_str())
        .collect();
    assert_eq!(unread.len(), 1, "{unread:?}");
    assert!(unread[0].contains("code.txt: "), "{unread:?}");
    assert!(!unread[0].contains("executable"), "{unread:?}");
}

#[tokio::test]
async fn the_digest_of_a_file_is_the_digest_of_the_pin_made_of_its_text() {
    // bytes that a normaliser would change: a CRLF, no last line feed, a non-ASCII character
    let text = "fn push() {}\r\n// caf\u{e9}\t";
    let project = RealTree::new();
    let mut app = App::new(project.dir.store().await, Counter::default(), clock())
        .with_tree(project.runner());
    populate(&mut app).await;
    // pin the text as `code_file` gives it, and write the file with the same bytes
    app.pin("core", "api", text).await.unwrap();
    project.write("code.txt", text);
    project.write("tests.txt", SOURCE);
    let found = app.check(None, &[]).await.unwrap().findings;
    assert!(tree_findings(&found).is_empty(), "{found:?}");
    // one byte of difference is a difference
    project.write("code.txt", &text.replace('\t', " "));
    let found = app.check(None, &[]).await.unwrap().findings;
    assert_eq!(tree_findings(&found).len(), 1, "{found:?}");
}
