//! What a measurement stands on, settings included, through the use cases: a run is stamped with the digest of the
//! settings it ran under; it stays current while those settings are the ones now, whatever else the table says; and
//! it is stale, not current by default, once the settings cannot be compared (there is no `[measure]`, or the runner
//! cannot read them for the subject). The runner here is the real one over a real `chidev.toml`, except where a test
//! needs a runner that answers as the test says.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chidev_core::app::commit::Gated;
use chidev_core::app::ports::external::ExternalReport;
use chidev_core::app::ports::measure::{
    BoxFuture, ExamplesReport, ExamplesRequest, MutateReport, MutateRequest, Runner, RunnerError,
    SettingsSaid, SettingsTarget, SimulateReport, SimulateRequest,
};
use chidev_core::app::ports::{
    AssetKind, AssetNames, IdSource, Invocation, InvocationSeq, JudgedReport, JudgedRepository,
    Store, StoreRevision,
};
use chidev_core::app::{
    App, CaseInput, ClauseInput, ClauseRef, ItemInput, ItemRef, MeasurementStanding,
    MeasurementState, Registry,
};
use chidev_core::domain::check::{CheckView, Moment};
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
use chidev_core::domain::vo::measure::{MeasureKind, SettingsDigest, SettingsNow};
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::project::{self, FILE};
use chidev_core::infra::runner::{self, ProcessRunner};
use chidev_core::infra::sqlite::SqliteStore;
use uuid::Uuid;

/// Ids from where it is started.
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

/// A project directory with a `chidev.toml`, and a store beside it.
struct Fixture {
    dir: PathBuf,
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

impl Fixture {
    fn new(table: &str) -> Fixture {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "chidev-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("project")).unwrap();
        let fixture = Fixture { dir };
        fixture.table(table);
        fixture
    }

    fn root(&self) -> PathBuf {
        self.dir.join("project")
    }

    /// Writes `table` as the project's `chidev.toml`.
    fn table(&self, table: &str) {
        std::fs::write(self.root().join(FILE), table).unwrap();
    }

    /// Writes the executable `name` of the project.
    fn script(&self, name: &str, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(self.root().join(name), text).unwrap();
        std::fs::set_permissions(
            self.root().join(name),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    fn runner(&self) -> ProcessRunner {
        runner::load(&project::find(&self.root()).unwrap())
            .unwrap()
            .expect("a [measure] table")
    }

    async fn store(&self) -> SqliteStore {
        SqliteStore::open(self.dir.join("store").join("chidev.db"))
            .await
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A table with a build, a test, a provider, a sim of the Context `core`, and a layout for its Interface `api` and
/// for its Harnesses `tests` and `more`.
const TABLE: &str = r#"[measure]
build = ["sh", "-c", "true"]
test = ["sh", "-c", "true"]
env = {}

[measure.mutants]
text = ["sh"]

[measure.sim.core]
interpreter = ["sh"]
program = ["x"]
seeds = 3

[[measure.layout]]
context = "core"
interface = "api"
lang = "text"
path = "code.txt"

[[measure.layout]]
context = "core"
harness = "tests"
path = "tests.txt"

[[measure.layout]]
context = "core"
harness = "more"
path = "more.txt"
"#;

/// [`TABLE`] with the top-level keys `keys` before its first table.
fn with(keys: &str) -> String {
    TABLE.replacen("[measure]\n", &format!("[measure]\n{keys}\n"), 1)
}

/// The real runner, except that a mutation run is not made: it reports the digest of the settings it resolved, as
/// the real one does, and nothing else. What it stands for is the settings, which is what these tests are about.
#[derive(Debug)]
struct Resolving(ProcessRunner);

impl Runner for Resolving {
    fn settings<'a>(
        &'a self,
        context: &'a ContextName,
        interface: &'a AssetName,
        harnesses: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        self.0.settings(context, interface, harnesses)
    }

    fn mutate<'a>(
        &'a self,
        request: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(async move {
            let names: Vec<AssetName> = request.harnesses.iter().map(|h| h.name.clone()).collect();
            let settings = self
                .0
                .settings(&request.context, &request.interface, &names)
                .await?;
            Ok(MutateReport {
                settings,
                sites: 0,
                planned: 0,
                mutants: Vec::new(),
                failure: None,
                evidence: Vec::new(),
            })
        })
    }

    fn sim_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        self.0.sim_settings(context, harness, builds)
    }

    fn simulate<'a>(
        &'a self,
        request: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>> {
        self.0.simulate(request)
    }

    fn examples_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        self.0.examples_settings(context, harness, builds)
    }

    fn run_examples<'a>(
        &'a self,
        request: &'a ExamplesRequest,
    ) -> BoxFuture<'a, Result<ExamplesReport, RunnerError>> {
        self.0.run_examples(request)
    }
}

/// A runner that says one thing of the settings before a run and runs under another.
#[derive(Debug)]
struct Racy;

fn digest_of(word: &str) -> SettingsDigest {
    SettingsDigest::new(Digest::of(word.as_bytes()))
}

impl Runner for Racy {
    fn settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Ok(digest_of("asked before the run")) })
    }

    fn mutate<'a>(
        &'a self,
        _: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(async {
            Ok(MutateReport {
                settings: digest_of("ran under"),
                sites: 0,
                planned: 0,
                mutants: Vec::new(),
                failure: None,
                evidence: Vec::new(),
            })
        })
    }

    fn sim_settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn simulate<'a>(
        &'a self,
        _: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn examples_settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn run_examples<'a>(
        &'a self,
        _: &'a ExamplesRequest,
    ) -> BoxFuture<'a, Result<ExamplesReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }
}

fn clauses() -> Vec<ClauseInput> {
    vec![ClauseInput {
        label: "C1".into(),
        text: "push then pop".into(),
        replaces: None,
    }]
}

/// A Harness `name` of the Context `core` that checks the Interface `api` and the Contract `stack`.
async fn put_tests(app: &mut TestApp, name: &str, case: &str) {
    app.put_harness(
        "core",
        name,
        "fn pushes() {}\n",
        &[CaseInput {
            name: case.into(),
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
}

/// A Project with a pinned Interface `api` that a Harness `tests` checks, in the Context `core`.
async fn pinned(app: &mut TestApp) {
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract("core", "stack", "Stack", &clauses())
        .await
        .unwrap();
    app.put_interface(
        "core",
        "api",
        "fn push();\n",
        &[ItemInput {
            key: "push".into(),
            lines: (1, 1),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    put_tests(app, "tests", "pushes").await;
    app.pin("core", "api", "fn push() {}").await.unwrap();
}

/// A Project that was measured under the real runner over `fixture`'s table.
async fn measured(fixture: &Fixture) -> TestApp {
    let mut app = App::new(fixture.store().await, Counter::default(), clock())
        .with_runner(Resolving(fixture.runner()));
    pinned(&mut app).await;
    app.measure(MeasureKind::Mutate, "core", "api")
        .await
        .unwrap();
    app
}

/// The same store and Project, as a program that starts again finds them, with the runner that `fixture`'s table
/// gives now.
async fn restarted(app: TestApp, fixture: &Fixture) -> TestApp {
    App::new(app.into_store(), Counter(10_000), clock()).with_runner(Resolving(fixture.runner()))
}

/// The same store and Project, started by a program whose project has no `[measure]` table.
fn unconfigured(app: TestApp) -> TestApp {
    App::new(app.into_store(), Counter(10_000), clock())
}

/// Where the one mutation run stands, in the word for it.
async fn state(app: &TestApp) -> &'static str {
    standing(app).await.state.word()
}

/// The standing of the one mutation run.
async fn standing(app: &TestApp) -> MeasurementStanding {
    let mut status = app.status().await.unwrap();
    assert_eq!(status.measurements.len(), 1, "{:?}", status.measurements);
    status.measurements.remove(0)
}

/// What moved of the one mutation run, as `(word, name)` of each part: it is stale.
async fn moved(app: &TestApp) -> Vec<(&'static str, Option<String>)> {
    let MeasurementState::Stale(parts) = standing(app).await.state else {
        panic!("the run is not stale");
    };
    parts
        .iter()
        .map(|p| (p.part().word(), p.name().map(ToString::to_string)))
        .collect()
}

/// The messages of the findings of the rule `rule` in `app.check`, and in a run that asks every check (`ask`).
async fn said(app: &mut TestApp, rule: &str) -> (Vec<String>, Vec<String>) {
    let of = |found: Vec<chidev_core::app::Found>| -> Vec<String> {
        found
            .into_iter()
            .filter(|f| f.rule.as_str() == rule)
            .map(|f| f.message)
            .collect()
    };
    let checked = of(app.check(None, &[]).await.unwrap().findings);
    let asked = of(app.check_run().await.unwrap().findings);
    (checked, asked)
}

#[tokio::test]
async fn changing_the_sim_table_of_the_context_leaves_a_mutation_run_current() {
    let fixture = Fixture::new(TABLE);
    let app = measured(&fixture).await;
    assert_eq!(state(&app).await, "current");
    // what a mutation run does not read: the sim of the Context, a provider of items, a language it is not
    // written in, the other Harness's layout moved
    fixture.table(
        &TABLE
            .replace("seeds = 3", "seeds = 9\nsample = 1\ntimeout = 7")
            .replace(
                "[measure.mutants]\n",
                "[measure.items]\ntext = [\"sh\"]\n[measure.mutants]\ngo = [\"sh\"]\n",
            ),
    );
    let app = restarted(app, &fixture).await;
    assert_eq!(state(&app).await, "current");
    // what it does read moves it
    fixture.table(&with("max_mutants = 3"));
    let app = restarted(app, &fixture).await;
    assert_eq!(state(&app).await, "stale");
    assert_eq!(moved(&app).await, [("settings", None)]);
}

#[tokio::test]
async fn a_configured_runner_with_no_layout_for_the_interface_makes_the_run_stale_and_unreadable() {
    let fixture = Fixture::new(TABLE);
    let app = measured(&fixture).await;
    assert_eq!(state(&app).await, "current");
    let (checked, asked) = {
        let mut app = restarted(app, &fixture).await;
        let seen = said(&mut app, "mutation.stale").await;
        assert!(seen.0.is_empty() && seen.1.is_empty(), "{seen:?}");
        // the table loses the layout of the Interface the run was of
        fixture.table(&TABLE.replace(
            "[[measure.layout]]\ncontext = \"core\"\ninterface = \"api\"\nlang = \"text\"\npath = \"code.txt\"\n",
            "",
        ));
        let mut app = restarted(app, &fixture).await;
        assert_eq!(state(&app).await, "stale");
        assert_eq!(moved(&app).await, [("settings-unreadable", None)]);
        said(&mut app, "mutation.stale").await
    };
    // `check` and the run that asks every check read the same settings
    for messages in [checked, asked] {
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(
            messages[0].contains(
                "the settings cannot be read: [[measure.layout]] has no entry for the interface `api` of `core`"
            ),
            "{messages:?}"
        );
        assert!(!messages[0].contains("not configured"), "{messages:?}");
    }
}

#[tokio::test]
async fn removing_measure_after_a_run_makes_it_stale_and_says_measure_is_not_configured() {
    let fixture = Fixture::new(TABLE);
    let app = measured(&fixture).await;
    assert_eq!(state(&app).await, "current");
    let mut app = unconfigured(app);
    assert_eq!(state(&app).await, "stale");
    assert_eq!(moved(&app).await, [("settings-unconfigured", None)]);
    let (checked, asked) = said(&mut app, "mutation.stale").await;
    for messages in [checked, asked] {
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(
            messages[0].contains("measure is not configured"),
            "{messages:?}"
        );
        assert!(!messages[0].contains("cannot be read"), "{messages:?}");
    }
}

#[tokio::test]
async fn a_run_is_stamped_with_the_settings_it_ran_under_and_not_with_ones_asked_for_before() {
    let store = SqliteStore::open(
        Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!(
                "chidev-settings-racy-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("chidev.db"),
    )
    .await
    .unwrap();
    let mut app = App::new(store, Counter::default(), clock()).with_runner(Racy);
    pinned(&mut app).await;
    app.measure(MeasureKind::Mutate, "core", "api")
        .await
        .unwrap();
    let kept = app.store().latest_measurements().await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        kept[0].stamp().settings().to_string(),
        digest_of("ran under").to_string()
    );
    // and what the runner says now is another thing, so the run is not current
    assert_eq!(state(&app).await, "stale");
}

#[tokio::test]
async fn a_program_that_is_another_file_in_the_copy_fails_the_measure_as_settings_moved() {
    // the Harness `tests` is written over `build.sh` in the copy, where the build command finds it
    let table = TABLE
        .replace(
            "build = [\"sh\", \"-c\", \"true\"]",
            "build = [\"./build.sh\"]",
        )
        .replace("path = \"tests.txt\"", "path = \"build.sh\"");
    let fixture = Fixture::new(&table);
    fixture.script("build.sh", "#!/bin/sh\ntrue\n");
    let mut app =
        App::new(fixture.store().await, Counter::default(), clock()).with_runner(fixture.runner());
    pinned(&mut app).await;
    let done = app
        .measure(MeasureKind::Mutate, "core", "api")
        .await
        .unwrap();
    let failure = done.failure.expect("a failure");
    assert!(failure.starts_with("settings_moved: "), "{failure}");
    assert!(failure.contains("./build.sh"), "{failure}");
    assert!(done.mutants.is_empty() && done.evidence.is_empty());
    // the failure is kept as the run, stamped with the settings it resolved
    let status = app.status().await.unwrap();
    assert_eq!(status.measurements[0].outcome, "failed");
    assert_eq!(status.measurements[0].state, MeasurementState::Current);
}

#[tokio::test]
async fn a_subject_the_view_has_no_settings_for_is_stale() {
    let fixture = Fixture::new(TABLE);
    let app = measured(&fixture).await;
    let store = app.store();
    let project = store.project().await.unwrap().expect("a project");
    let pins = store.pins().await.unwrap();
    let runs = store.latest_measurements().await.unwrap();
    let at = store.revision().await.unwrap();
    let registry = Registry::defaults();
    let stale = |measure_settings: &BTreeMap<_, SettingsNow>| {
        registry
            .run(
                Moment::Check,
                at,
                &CheckView {
                    project: &project,
                    pins: &pins,
                    change_requests: &[],
                    cruxes: &[],
                    measurements: &runs,
                    measure_settings,
                    sim_settings: &BTreeMap::new(),
                    examples_settings: &BTreeMap::new(),
                    resolving: None,
                },
            )
            .findings
            .into_iter()
            .filter(|f| f.rule().as_str() == "mutation.stale")
            .map(|f| f.message().to_string())
            .collect::<Vec<_>>()
    };
    let interface = runs[0].interface().expect("a mutation run");
    // no entry for the Interface: its settings were not read
    let messages = stale(&BTreeMap::new());
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("the settings cannot be read: its settings were not read"));
    // the settings the run was made with: not stale
    let same = BTreeMap::from([(interface, SettingsNow::Known(*runs[0].stamp().settings()))]);
    assert!(stale(&same).is_empty());
    // the same Interface, unconfigured: stale, and for that reason
    let none = BTreeMap::from([(interface, SettingsNow::Unconfigured)]);
    assert!(stale(&none)[0].contains("measure is not configured"));
}

#[tokio::test]
async fn a_measure_and_the_settings_asked_for_later_pick_the_same_harnesses() {
    let fixture = Fixture::new(TABLE);
    let mut app = measured(&fixture).await;
    // a second Harness that checks the Interface: the pin's basis has it, and the recorded run is stale
    put_tests(&mut app, "more", "also").await;
    assert_eq!(state(&app).await, "stale");
    // what moved is the new Harness, by its name, and the settings, which now name both Harnesses
    assert_eq!(
        moved(&app).await,
        [("harness", Some("more".to_string())), ("settings", None)]
    );
    // measured again, it is stamped with the settings of both, and asking for them later gives those: current
    app.measure(MeasureKind::Mutate, "core", "api")
        .await
        .unwrap();
    assert_eq!(state(&app).await, "current");
    // the settings of the run that ran only `tests` are not those of the run that ran both
    let both = fixture
        .runner()
        .settings(
            &ContextName::try_from("core").unwrap(),
            &AssetName::try_from("api").unwrap(),
            &[
                AssetName::try_from("tests").unwrap(),
                AssetName::try_from("more").unwrap(),
            ],
        )
        .await
        .unwrap();
    let one = fixture
        .runner()
        .settings(
            &ContextName::try_from("core").unwrap(),
            &AssetName::try_from("api").unwrap(),
            &[AssetName::try_from("tests").unwrap()],
        )
        .await
        .unwrap();
    assert_ne!(both, one);
    let kept = app.store().latest_measurements().await.unwrap();
    assert_eq!(kept[0].stamp().settings().to_string(), both.to_string());
}

/// A store as it is, except that it knows of no Pin: a Project whose measurement outlived the Pin it was made for.
struct NoPins(SqliteStore);

impl ProjectRepository for NoPins {
    async fn project(&self) -> Result<Option<Project>, StoreError> {
        self.0.project().await
    }
}

impl PinRepository for NoPins {
    async fn pin_of(&self, _: InterfaceId) -> Result<Option<Pin>, StoreError> {
        Ok(None)
    }

    async fn pins(&self) -> Result<Vec<Pin>, StoreError> {
        Ok(Vec::new())
    }
}

impl ChangeRequestRepository for NoPins {
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

impl CruxRepository for NoPins {
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

impl AssetNames for NoPins {
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

impl JudgedRepository for NoPins {
    async fn judged_reports(&self) -> Result<Vec<JudgedReport>, StoreError> {
        self.0.judged_reports().await
    }
}

impl Store for NoPins {
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
        self.0.pin_code(pin).await
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

#[tokio::test]
async fn a_run_of_an_interface_with_no_pin_is_unpinned_and_not_stale() {
    let fixture = Fixture::new(TABLE);
    let app = measured(&fixture).await;
    assert_eq!(state(&app).await, "current");
    let app = App::new(NoPins(app.into_store()), Counter(10_000), clock())
        .with_runner(Resolving(fixture.runner()));
    let status = app.status().await.unwrap();
    assert_eq!(status.measurements.len(), 1, "{:?}", status.measurements);
    assert_eq!(status.measurements[0].state, MeasurementState::Unpinned);
    assert_eq!(status.measurements[0].state.word(), "unpinned");
}

#[tokio::test]
async fn a_run_of_an_interface_retired_since_stays_retired() {
    let fixture = Fixture::new(TABLE);
    let mut app = measured(&fixture).await;
    // the Harness cites the Interface's Item: it goes first
    app.retire_harness("core", "tests").await.unwrap();
    app.retire_interface("core", "api").await.unwrap();
    let standing = standing(&app).await;
    assert_eq!(standing.state, MeasurementState::Retired);
    assert_eq!(standing.state.word(), "retired");
    assert_eq!(standing.interface.as_deref(), Some("api (retired)"));
}

/// How a runner was asked for settings.
#[derive(Debug, Default)]
struct Asks {
    /// How many times it was asked for many at once, and how many targets those asks held.
    batches: AtomicUsize,
    targets: AtomicUsize,
    /// How many times it was asked about one subject.
    singles: AtomicUsize,
}

/// The real runner, which counts how it is asked for settings.
#[derive(Debug)]
struct Counting {
    runner: Resolving,
    asks: Arc<Asks>,
}

impl Runner for Counting {
    fn settings_of<'a>(&'a self, targets: &'a [SettingsTarget]) -> BoxFuture<'a, SettingsSaid> {
        self.asks.batches.fetch_add(1, Ordering::Relaxed);
        self.asks
            .targets
            .fetch_add(targets.len(), Ordering::Relaxed);
        self.runner.0.settings_of(targets)
    }

    fn settings<'a>(
        &'a self,
        context: &'a ContextName,
        interface: &'a AssetName,
        harnesses: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        self.asks.singles.fetch_add(1, Ordering::Relaxed);
        self.runner.settings(context, interface, harnesses)
    }

    fn mutate<'a>(
        &'a self,
        request: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        self.runner.mutate(request)
    }

    fn sim_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        self.asks.singles.fetch_add(1, Ordering::Relaxed);
        self.runner.sim_settings(context, harness, builds)
    }

    fn simulate<'a>(
        &'a self,
        request: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>> {
        self.runner.simulate(request)
    }

    fn examples_settings<'a>(
        &'a self,
        context: &'a ContextName,
        harness: &'a AssetName,
        builds: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        self.asks.singles.fetch_add(1, Ordering::Relaxed);
        self.runner.examples_settings(context, harness, builds)
    }

    fn run_examples<'a>(
        &'a self,
        request: &'a ExamplesRequest,
    ) -> BoxFuture<'a, Result<ExamplesReport, RunnerError>> {
        self.runner.run_examples(request)
    }
}

#[tokio::test]
async fn the_settings_now_of_every_run_are_asked_of_the_runner_once_and_together() {
    let fixture = Fixture::new(TABLE);
    let app = measured(&fixture).await;
    let asks = Arc::new(Asks::default());
    let app = App::new(app.into_store(), Counter(10_000), clock()).with_runner(Counting {
        runner: Resolving(fixture.runner()),
        asks: Arc::clone(&asks),
    });
    // the standing reads the measurements and what the settings are now: one ask, with the run's subject in it
    assert_eq!(state(&app).await, "current");
    assert_eq!(asks.batches.load(Ordering::Relaxed), 1);
    assert_eq!(asks.targets.load(Ordering::Relaxed), 1);
    assert_eq!(asks.singles.load(Ordering::Relaxed), 0);
    // and a check of the project reads them the same way: once more, and still never one at a time
    app.check(None, &[]).await.unwrap();
    assert!(asks.batches.load(Ordering::Relaxed) >= 2);
    assert_eq!(
        asks.batches.load(Ordering::Relaxed),
        asks.targets.load(Ordering::Relaxed)
    );
    assert_eq!(asks.singles.load(Ordering::Relaxed), 0);
}
