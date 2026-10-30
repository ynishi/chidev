//! A run of the examples of a Harness through the use cases: `run_examples` records it, the checks of the examples
//! judge the record, and a pin or a resolution reports what they find and is refused for none of it. The runner is
//! a double that says which examples fail and under which settings; what a real runner does is in
//! `examples_runner.rs`.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chidev_core::app::ports::measure::{
    BoxFuture, ExamplesReport, ExamplesRequest, MutateReport, MutateRequest, Runner, RunnerError,
    SimulateReport, SimulateRequest,
};
use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, CorrectInput,
    DecisionMakerInput, ExampleInput, ExampleTexts, ExamplesSummary, Found, Intent, ItemInput,
    ItemRef, MeasureSummary, MeasurementState, Place, ResolutionInput,
};
use chidev_core::domain::entity::measurement::MeasurementField;
use chidev_core::domain::repository::ProjectRepository;
use chidev_core::domain::vo::digest::Digest;
use chidev_core::domain::vo::example::{ExampleName, Verdict, Why};
use chidev_core::domain::vo::finding::Subject;
use chidev_core::domain::vo::measure::{FailureKind, MeasureFailure, MeasureKind, SettingsDigest};
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
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

type TestApp = App<SqliteStore, Counter>;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// What the double runner is told to do, and what it was asked.
#[derive(Debug, Default)]
struct Script {
    /// The examples that fail, by name.
    failing: BTreeSet<String>,
    /// The examples that get no answer from the program, by name, and what happened.
    abnormal: std::collections::BTreeMap<String, String>,
    /// The word the settings digest is made of.
    settings: String,
    /// Why the run fails, if it does.
    failure: Option<MeasureFailure>,
    /// An example the Harness has no Case of, reported all the same.
    stray: Option<String>,
    /// The examples of each request, in the order they were asked.
    asked: Vec<Vec<String>>,
}

fn digest_of(word: &str) -> SettingsDigest {
    SettingsDigest::new(Digest::of(word.as_bytes()))
}

/// A runner that runs no program: it judges the examples as it was told.
#[derive(Debug, Clone)]
struct Scripted(Arc<Mutex<Script>>);

impl Runner for Scripted {
    fn settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn mutate<'a>(
        &'a self,
        _: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
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
        Box::pin(async move { Ok(digest_of(&self.0.lock().unwrap().settings)) })
    }

    fn run_examples<'a>(
        &'a self,
        request: &'a ExamplesRequest,
    ) -> BoxFuture<'a, Result<ExamplesReport, RunnerError>> {
        Box::pin(async move {
            let mut script = self.0.lock().unwrap();
            let mut examples: Vec<(ExampleName, Verdict)> = request
                .examples
                .iter()
                .map(|(name, _)| {
                    let verdict = if let Some(why) = script.abnormal.get(name.as_str()) {
                        Verdict::fail(Why::Abnormal(why.clone()))
                    } else if script.failing.contains(name.as_str()) {
                        Verdict::fail(Why::Exit {
                            expected: 0,
                            got: 1,
                        })
                    } else {
                        Verdict::Ok
                    };
                    (name.clone(), verdict)
                })
                .collect();
            script
                .asked
                .push(examples.iter().map(|(n, _)| n.to_string()).collect());
            if let Some(stray) = &script.stray {
                examples.push((ExampleName::new(stray).unwrap(), Verdict::Ok));
            }
            Ok(ExamplesReport {
                settings: digest_of(&script.settings),
                examples,
                failure: script.failure.clone(),
                evidence: Vec::new(),
            })
        })
    }
}

struct Fixture {
    dir: PathBuf,
    script: Arc<Mutex<Script>>,
    app: TestApp,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn clause_ref(label: &str) -> ClauseRef {
    ClauseRef {
        context: None,
        contract: AssetName::try_from("stack").unwrap(),
        label: label.to_string(),
    }
}

fn tests_case(name: &str, interface: &str, key: &str) -> CaseInput {
    CaseInput {
        name: name.into(),
        lines: (1, 1),
        items: vec![ItemRef {
            context: None,
            interface: AssetName::try_from(interface).unwrap(),
            key: key.into(),
        }],
        clauses: vec![clause_ref("C1")],
    }
}

/// The Examples `a1` (citing C1) and `a2` (citing C1 and C2); the expected output of `a1` is `expected`.
fn examples(expected: &str) -> Vec<ExampleInput> {
    let example = |name: &str, clauses: &[&str], expected: &str| ExampleInput {
        name: name.to_string(),
        clauses: clauses.iter().map(|l| clause_ref(l)).collect(),
        example: ExampleTexts {
            args: String::new(),
            stdin: String::new(),
            expected: expected.to_string(),
            stderr: None,
        },
    };
    vec![
        example("a1", &["C1"], expected),
        example("a2", &["C1", "C2"], "#exit 0\n"),
    ]
}

impl Fixture {
    /// A Project with the Interfaces `api` and `lib`, both pinned (a tests Harness checks each), and a Harness of
    /// examples `acceptance` that builds `api`, under a runner that passes everything.
    async fn new() -> Fixture {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "chidev-examples-measure-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = Arc::new(Mutex::new(Script {
            settings: "settings".to_string(),
            ..Script::default()
        }));
        let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
        let mut app: TestApp = App::new(
            store,
            Counter::default(),
            FixedClock(Timestamp::from_millis(1_000)),
        )
        .with_runner(Scripted(script.clone()));
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.put_contract(
            "core",
            "stack",
            "Stack",
            &[
                ClauseInput {
                    label: "C1".into(),
                    text: "push then pop".into(),
                    replaces: None,
                },
                ClauseInput {
                    label: "C2".into(),
                    text: "pop on empty fails".into(),
                    replaces: None,
                },
            ],
        )
        .await
        .unwrap();
        for (name, key) in [("api", "push"), ("lib", "other")] {
            app.put_interface(
                "core",
                name,
                &format!("fn {key}();\n"),
                &[ItemInput {
                    key: key.into(),
                    lines: (1, 1),
                    replaces: None,
                }],
            )
            .await
            .unwrap();
        }
        app.put_harness(
            "core",
            "tests",
            "fn pushes() {}\nfn others() {}\n",
            &[
                tests_case("pushes", "api", "push"),
                tests_case("others", "lib", "other"),
            ],
        )
        .await
        .unwrap();
        app.pin("core", "api", "fn push() {}").await.unwrap();
        app.pin("core", "lib", "fn other() {}").await.unwrap();
        let mut fixture = Fixture { dir, script, app };
        fixture.put_examples("#exit 0\n").await;
        fixture
    }

    async fn put_examples(&mut self, expected: &str) {
        self.app
            .put_examples_harness(
                "core",
                "acceptance",
                &["api".to_string()],
                &examples(expected),
            )
            .await
            .unwrap();
    }

    async fn run(&mut self) -> chidev_core::app::MeasureDone {
        self.app.run_examples("core", "acceptance").await.unwrap()
    }

    async fn findings(&self) -> Vec<Found> {
        self.app.check(None, &[]).await.unwrap().findings
    }

    /// The findings of the rules of the examples, as `(rule, message)`.
    async fn examples_findings(&self) -> Vec<(String, String)> {
        only_examples(&self.findings().await)
    }
}

fn only_examples(found: &[Found]) -> Vec<(String, String)> {
    found
        .iter()
        .filter(|f| f.rule.as_str().starts_with("examples."))
        .map(|f| (f.rule.to_string(), f.message.clone()))
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
async fn a_harness_never_run_is_found_until_a_run_is_recorded() {
    let mut f = Fixture::new().await;
    let found = f.examples_findings().await;
    assert_eq!(
        found,
        [(
            "examples.unrun".to_string(),
            "the harness of examples was never run".to_string()
        )]
    );
    f.run().await;
    assert!(f.examples_findings().await.is_empty());
}

#[tokio::test]
async fn when_every_example_passes_the_run_is_current_and_finds_nothing() {
    let mut f = Fixture::new().await;
    let done = f.run().await;
    assert_eq!(done.kind, "examples");
    assert_eq!(done.failure, None);
    let lines: Vec<_> = done
        .examples
        .iter()
        .map(|e| (e.example.as_str(), e.passed, e.reasons.len()))
        .collect();
    assert_eq!(lines, [("a1", true, 0), ("a2", true, 0)]);
    assert!(f.examples_findings().await.is_empty());
    let status = f.app.status().await.unwrap();
    let standing = status
        .measurements
        .iter()
        .find(|m| m.kind == "examples")
        .expect("the run is listed");
    assert_eq!(standing.harness.as_deref(), Some("acceptance"));
    assert_eq!(standing.state, MeasurementState::Current);
    assert_eq!(standing.outcome, "ok");
    assert_eq!(
        standing.summary,
        MeasureSummary::Examples(ExamplesSummary {
            passed: 2,
            failed: 0
        })
    );
    // the runner was asked for the examples in the order of the Cases, as the model holds them
    assert_eq!(f.script.lock().unwrap().asked, [["a1", "a2"]]);
}

#[tokio::test]
async fn an_example_that_fails_is_a_finding_on_each_clause_its_case_cites() {
    let mut f = Fixture::new().await;
    f.script.lock().unwrap().failing.insert("a2".to_string());
    let done = f.run().await;
    assert_eq!(done.failure, None);
    assert_eq!(done.examples[1].example, "a2");
    assert!(!done.examples[1].passed);
    assert_eq!(done.examples[1].reasons, ["exit code: got 1"]);
    let found = f.findings().await;
    let failed: Vec<&Found> = found
        .iter()
        .filter(|f| f.rule.as_str() == "examples.failed")
        .collect();
    // a2 cites C1 and C2: two findings, one for each Clause
    assert_eq!(failed.len(), 2, "{found:#?}");
    assert!(
        failed
            .iter()
            .all(|f| matches!(f.subject, Some(Subject::Clause(_))))
    );
    let about: Vec<_> = failed.iter().map(|f| f.about.clone().unwrap()).collect();
    assert_eq!(about, ["stack C1", "stack C2"]);
    assert!(
        failed
            .iter()
            .all(|f| f.message == "example `a2` failed: exit code: got 1")
    );
    let status = f.app.status().await.unwrap();
    assert_eq!(
        status.measurements[0].summary,
        MeasureSummary::Examples(ExamplesSummary {
            passed: 1,
            failed: 1
        })
    );
}

#[tokio::test]
async fn a_record_is_of_the_case_by_its_id_and_a_name_the_harness_has_no_case_of_keeps_nothing() {
    let mut f = Fixture::new().await;
    f.run().await;
    let kept = f.app.store().latest_measurements().await.unwrap();
    let run = kept[0].examples().expect("a run of examples");
    let p = f.app.store().project().await.unwrap().unwrap();
    let harness = p
        .contexts()
        .flat_map(|c| c.harnesses())
        .find(|h| h.kind().word() == "examples")
        .unwrap();
    let ids: Vec<_> = harness.cases().iter().map(|c| c.id()).collect();
    let recorded: Vec<_> = run.records().iter().map(|r| r.case).collect();
    assert_eq!(recorded, ids);

    // a runner that reports an example the Harness has no Case of: the run is not recorded
    let before = f.app.store().latest_measurements().await.unwrap().len();
    f.script.lock().unwrap().stray = Some("nowhere".to_string());
    let error = f.app.run_examples("core", "acceptance").await.unwrap_err();
    match error {
        AppError::Input(message) => assert!(message.contains("nowhere"), "{message}"),
        other => panic!("not an input error: {other:?}"),
    }
    assert_eq!(
        f.app.store().latest_measurements().await.unwrap().len(),
        before
    );
}

#[tokio::test]
async fn a_changed_expected_output_makes_the_run_stale_naming_the_examples() {
    let mut f = Fixture::new().await;
    f.run().await;
    f.put_examples("1\n#exit 0\n").await;
    let found = f.examples_findings().await;
    assert_eq!(
        found,
        [(
            "examples.stale".to_string(),
            "the latest run of the examples is stale: examples".to_string()
        )]
    );
    let status = f.app.status().await.unwrap();
    let MeasurementState::Stale(parts) = &status.measurements[0].state else {
        panic!("not stale: {:?}", status.measurements[0].state);
    };
    let words: Vec<_> = parts.iter().map(|p| p.part().word()).collect();
    assert_eq!(words, ["examples"]);
    // run again: current
    f.run().await;
    assert!(f.examples_findings().await.is_empty());
}

#[tokio::test]
async fn pinned_again_the_code_of_a_built_interface_makes_the_run_stale_naming_the_built_code() {
    let mut f = Fixture::new().await;
    f.run().await;
    f.app.pin("core", "api", "fn push() { 2 }").await.unwrap();
    let found = f.examples_findings().await;
    assert_eq!(
        found,
        [(
            "examples.stale".to_string(),
            "the latest run of the examples is stale: built-code".to_string()
        )]
    );
    let status = f.app.status().await.unwrap();
    let MeasurementState::Stale(parts) = &status.measurements[0].state else {
        panic!("not stale");
    };
    assert_eq!(parts[0].part().word(), "built-code");
    assert_eq!(parts[0].name().map(|n| n.as_str()), Some("api"));
}

#[tokio::test]
async fn settings_that_moved_make_the_run_stale() {
    let mut f = Fixture::new().await;
    f.run().await;
    f.script.lock().unwrap().settings = "another".to_string();
    let found = f.examples_findings().await;
    assert_eq!(
        found,
        [(
            "examples.stale".to_string(),
            "the latest run of the examples is stale: settings".to_string()
        )]
    );
}

#[tokio::test]
async fn a_run_that_failed_is_an_error_and_nothing_else() {
    let mut f = Fixture::new().await;
    f.script.lock().unwrap().failure = Some(MeasureFailure::of(
        FailureKind::NoBuild,
        "the program does not build",
    ));
    f.script.lock().unwrap().failing.insert("a1".to_string());
    let done = f.run().await;
    assert_eq!(
        done.failure.as_deref(),
        Some("no_build: the program does not build")
    );
    assert_eq!(
        f.examples_findings().await,
        [(
            "examples.error".to_string(),
            "the latest run of the examples failed (no_build: the program does not build)"
                .to_string()
        )]
    );
}

#[tokio::test]
async fn a_pin_reports_what_the_latest_run_says_and_is_not_refused_for_it() {
    let mut f = Fixture::new().await;
    // before a run nothing of the examples is said at a pin: an unrun Harness is judged on demand only
    let pinned = f.app.pin("core", "lib", "fn other() { 1 }").await.unwrap();
    assert!(only_examples(&pinned.findings).is_empty());

    // a failing run that is current: reported, not refused
    f.script.lock().unwrap().failing.insert("a1".to_string());
    f.run().await;
    let pinned = f.app.pin("core", "lib", "fn other() { 2 }").await.unwrap();
    let rules: Vec<_> = only_examples(&pinned.findings)
        .into_iter()
        .map(|(rule, _)| rule)
        .collect();
    assert_eq!(rules, ["examples.failed"]);
    assert!(
        pinned
            .findings
            .iter()
            .any(|f| f.rule.as_str() == "examples.failed" && f.level.word() == "error")
    );

    // the code of a built Interface pinned again: the run is stale, which the pin itself reports
    let pinned = f.app.pin("core", "api", "fn push() { 3 }").await.unwrap();
    assert_eq!(
        only_examples(&pinned.findings),
        [(
            "examples.stale".to_string(),
            "the latest run of the examples is stale: built-code".to_string()
        )]
    );
}

#[tokio::test]
async fn a_resolution_reports_what_the_latest_run_says_and_is_not_refused_for_it() {
    let mut f = Fixture::new().await;
    f.script.lock().unwrap().failing.insert("a1".to_string());
    f.run().await;
    f.app.open_change_request(&request("r")).await.unwrap();
    let resolved = f
        .app
        .resolve_change_request("r", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();
    assert_eq!(
        only_examples(&resolved.findings)
            .into_iter()
            .map(|(rule, _)| rule)
            .collect::<Vec<_>>(),
        ["examples.failed"]
    );

    // a run that failed to finish, then an example changed: the error and not the stale
    f.script.lock().unwrap().failure = Some(MeasureFailure::of(
        FailureKind::Budget,
        "the time budget of the run is used up",
    ));
    f.run().await;
    f.put_examples("1\n#exit 0\n").await;
    f.app.open_change_request(&request("s")).await.unwrap();
    let resolved = f
        .app
        .resolve_change_request("s", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();
    let rules: Vec<_> = only_examples(&resolved.findings)
        .into_iter()
        .map(|(rule, _)| rule)
        .collect();
    assert_eq!(rules, ["examples.error"]);

    // and a run made again on the examples as they are says nothing
    f.script.lock().unwrap().failure = None;
    f.script.lock().unwrap().failing.clear();
    f.run().await;
    f.app.open_change_request(&request("t")).await.unwrap();
    let resolved = f
        .app
        .resolve_change_request("t", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();
    assert!(only_examples(&resolved.findings).is_empty());
}

#[tokio::test]
async fn a_reason_with_a_text_that_must_not_stay_is_found_and_corrected_in_the_store() {
    const TOKEN: &str = "tok-SECRET-123";
    let mut f = Fixture::new().await;
    f.script
        .lock()
        .unwrap()
        .abnormal
        .insert("a1".to_string(), format!("cannot start: {TOKEN}"));
    f.run().await;
    let places = f.app.find_text(TOKEN).await.unwrap();
    assert_eq!(places.len(), 1, "{places:?}");
    let Place::Measurement { field, .. } = places[0] else {
        panic!("a place in a measurement, got {:?}", places[0]);
    };
    assert_eq!(
        field,
        MeasurementField::ExampleReason {
            example: 0,
            reason: 0
        }
    );
    let before = f.app.status().await.unwrap().measurements[0].clone();
    assert!(before.corrections.is_empty());
    let answer = f
        .app
        .correct_records(&CorrectInput {
            text: TOKEN.to_string(),
            reason: "a token was pasted".to_string(),
            by: DecisionMakerInput::Person("carol".into()),
            places: places.clone(),
        })
        .await
        .unwrap();
    assert_eq!(answer.corrected, places);
    // what the store holds now: the text replaced, the rest of the run as it was
    let kept = f.app.store().latest_measurements().await.unwrap();
    let records = kept[0].examples().unwrap().records();
    assert_eq!(
        records[0].result,
        chidev_core::domain::vo::examples_run::ExampleResult::Failed(vec![
            chidev_core::domain::vo::examples_run::Reason::Abnormal(
                "cannot start: [redacted]".to_string()
            )
        ])
    );
    assert_eq!(kept[0].corrections().len(), 1);
    let after = f.app.status().await.unwrap().measurements[0].clone();
    assert_eq!(after.stamp, before.stamp);
    assert_eq!(after.state, before.state);
    assert_eq!(after.corrections[0].fields, ["example[0].reason[0]"]);
    assert!(f.app.find_text(TOKEN).await.unwrap().is_empty());
}

#[tokio::test]
async fn only_a_harness_of_examples_is_run_and_its_interfaces_must_be_pinned() {
    let mut f = Fixture::new().await;
    // a tests Harness is not run as examples
    match f.app.run_examples("core", "tests").await.unwrap_err() {
        AppError::Input(message) => assert!(message.contains("tests"), "{message}"),
        other => panic!("{other:?}"),
    }
    // a Harness that builds an Interface with no Pin
    f.app
        .put_interface(
            "core",
            "fresh",
            "fn fresh();\n",
            &[ItemInput {
                key: "fresh".into(),
                lines: (1, 1),
                replaces: None,
            }],
        )
        .await
        .unwrap();
    f.app
        .put_examples_harness(
            "core",
            "fresh-examples",
            &["fresh".to_string()],
            &examples("#exit 0\n"),
        )
        .await
        .unwrap();
    match f
        .app
        .run_examples("core", "fresh-examples")
        .await
        .unwrap_err()
    {
        AppError::NotPinned { interface, .. } => assert_eq!(interface.as_str(), "fresh"),
        other => panic!("{other:?}"),
    }
    assert!(
        f.app
            .store()
            .latest_measurements()
            .await
            .unwrap()
            .iter()
            .all(|m| m.kind() != MeasureKind::Examples)
    );
    // the other kinds of measure are told to use the right door
    match f
        .app
        .measure(MeasureKind::Examples, "core", "api")
        .await
        .unwrap_err()
    {
        AppError::Input(message) => assert!(message.contains("run_examples"), "{message}"),
        other => panic!("{other:?}"),
    }
}
