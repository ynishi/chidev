//! Every use case that changes what the store keeps is a trigger, enters through the entry of its kind, and is kept
//! only after the checks its kind asks for. These tests hold the promises that follow: the use cases that used to keep
//! without a check now keep through one, a record runs none, what was checked is what is kept, and what a use case
//! ran is recorded under its name.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chidev_core::app::commit::{
    AskTrigger, ConfirmTrigger, EditTrigger, Gated, RecordTrigger, Trigger,
};
use chidev_core::app::ports::external::{
    ExecutionEvidence, ExternalAttempt, ExternalCall, ExternalExecutor, ExternalFailure,
};
use chidev_core::app::ports::measure::{
    BoxFuture, MutateReport, MutateRequest, Runner, RunnerError, SimulateReport, SimulateRequest,
};
use chidev_core::app::ports::{
    AssetKind, AssetNames, IdSource, JudgedReport, JudgedRepository, Store, StoreRevision,
};
use chidev_core::app::{
    App, AppError, AssumptionInput, BasisInput, CaseInput, ChangeRequestInput, ClauseInput,
    ClauseRef, CruxKind, DecisionInput, DecisionMakerInput, Intent, ItemInput, ItemRef,
    OptionInput, RaiseCruxInput, Raised, Registry, ReportInput, ReportedFinding, ResolutionInput,
    SubjectInput,
};
use chidev_core::domain::check::external::ExternalRule;
use chidev_core::domain::check::{
    Check, CheckView, Descriptor, Executor, Finding, Level, Moment, Params,
};
use chidev_core::domain::entity::change_request::ChangeRequest;
use chidev_core::domain::entity::crux::Crux;
use chidev_core::domain::entity::measurement::Measurement;
use chidev_core::domain::entity::pin::Pin;
use chidev_core::domain::entity::project::Project;
use chidev_core::domain::repository::{
    ChangeRequestRepository, CruxRepository, PinRepository, ProjectRepository, StoreError,
};
use chidev_core::domain::service::change_request::raised_in;
use chidev_core::domain::vo::finding::{RuleId, Subject};
use chidev_core::domain::vo::id::{ChangeRequestId, ContextId, CruxId, InterfaceId, PinId};
use chidev_core::domain::vo::measure::MeasureKind;
use chidev_core::domain::vo::model_section::ModelSection;
use chidev_core::domain::vo::resolution::Transition;
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::judged;
use chidev_core::infra::sqlite::SqliteStore;
use uuid::Uuid;

/// Ids 1, 2, 3, ... from where it is started.
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

fn fresh_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir()
        .join(format!(
            "chidev-commit-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
        .join("chidev.db")
}

async fn open(name: &str) -> SqliteStore {
    SqliteStore::open(fresh_path(name)).await.unwrap()
}

fn clauses() -> Vec<ClauseInput> {
    vec![ClauseInput {
        label: "C1".into(),
        text: "push then pop".into(),
        replaces: None,
    }]
}

// ---- the checks the tests run use cases under ----

/// A check of a given kind over the change requests: one finding for each that `applies` to.
macro_rules! change_request_check {
    ($check:ident, $descriptor:ident, $id:literal, $level:expr, $mandatory:expr, $applies:expr) => {
        struct $check;

        static $descriptor: Descriptor = Descriptor {
            id: RuleId::from_static($id),
            summary: Cow::Borrowed("a change request is in a state"),
            help: Cow::Borrowed("only for the test"),
            moments: Cow::Borrowed(&[Moment::Edit]),
            default_level: $level,
            mandatory: $mandatory,
            params: Cow::Borrowed(&[]),
            executor: Executor::Builtin,
        };

        impl Check for $check {
            fn descriptor(&self) -> &'static Descriptor {
                &$descriptor
            }

            fn run(&self, view: &CheckView<'_>, _: &Params) -> Vec<Finding> {
                let applies: fn(&ChangeRequest, &[Crux]) -> bool = $applies;
                view.change_requests
                    .iter()
                    .filter(|cr| applies(cr, view.cruxes))
                    .map(|cr| {
                        Finding::new(
                            &$descriptor,
                            Some(Subject::ChangeRequest(cr.id())),
                            format!("the change request {}", cr.title()),
                        )
                    })
                    .collect()
            }
        }
    };
}

fn exists(_: &ChangeRequest, _: &[Crux]) -> bool {
    true
}

fn is_decided(cr: &ChangeRequest, cruxes: &[Crux]) -> bool {
    raised_in(cr, cruxes).any(|q| !q.is_open())
}

fn is_assumed(cr: &ChangeRequest, _: &[Crux]) -> bool {
    !cr.assumptions().is_empty()
}

fn was_reopened(cr: &ChangeRequest, _: &[Crux]) -> bool {
    cr.history()
        .iter()
        .any(|t| matches!(t, Transition::Reopened(_)))
}

change_request_check!(
    Opened,
    OPENED,
    "test.cr-opened",
    Level::Warning,
    false,
    exists
);
change_request_check!(
    Decided,
    DECIDED,
    "test.cr-decided",
    Level::Warning,
    false,
    is_decided
);
change_request_check!(
    Assumed,
    ASSUMED,
    "test.cr-assumed",
    Level::Warning,
    false,
    is_assumed
);
change_request_check!(
    Reopened,
    REOPENED,
    "test.cr-reopened",
    Level::Warning,
    false,
    was_reopened
);

change_request_check!(
    NoOpened,
    NO_OPENED,
    "test.no-cr-opened",
    Level::Error,
    true,
    exists
);
change_request_check!(
    NoDecided,
    NO_DECIDED,
    "test.no-cr-decided",
    Level::Error,
    true,
    is_decided
);
change_request_check!(
    NoAssumed,
    NO_ASSUMED,
    "test.no-cr-assumed",
    Level::Error,
    true,
    is_assumed
);
change_request_check!(
    NoReopened,
    NO_REOPENED,
    "test.no-cr-reopened",
    Level::Error,
    true,
    was_reopened
);

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

/// A question for the change request titled `cr`, with the one option `keep`.
fn question(cr: &str) -> RaiseCruxInput {
    RaiseCruxInput {
        cr: cr.into(),
        kind: CruxKind::Ambiguity,
        question: "Is it kept?".into(),
        findings: vec![],
        options: vec![OptionInput {
            choice: "keep".into(),
            change: None,
            targets: vec![],
            keep: vec![],
            consequence: None,
            recommended: None,
        }],
        importance: None,
        supersedes: None,
    }
}

/// The decision of the one option of a Crux just raised.
fn chose(raised: &Raised) -> DecisionInput {
    DecisionInput::Chosen {
        option: raised.options[0].id.to_string(),
        reason: None,
    }
}

fn arbitrary() -> BasisInput {
    BasisInput::Arbitrary("nothing derives it".into())
}

fn alice() -> DecisionMakerInput {
    DecisionMakerInput::Person("alice".into())
}

fn assumption() -> AssumptionInput {
    AssumptionInput {
        statement: "the clause means this".into(),
        reason: "it is how it is used".into(),
        evidence: vec![],
    }
}

/// An App over a new store, its Project made, and the checks of `registry`.
async fn app_with(name: &str, registry: Registry) -> App<SqliteStore, Counter> {
    let mut app = App::with_registry(open(name).await, Counter::default(), registry, clock());
    app.init().await.unwrap();
    app
}

/// The same store, under other checks: how a project is read again with other rules in force.
fn under(
    app: App<SqliteStore, Counter>,
    registry: Registry,
    ids_from: u128,
) -> App<SqliteStore, Counter> {
    App::with_registry(app.into_store(), Counter(ids_from), registry, clock())
}

fn registry(checks: &[&'static dyn Check]) -> Registry {
    Registry::new(checks, &[]).unwrap()
}

/// What the edits of change requests introduce is what they are told, and nothing the request already was.
#[tokio::test]
async fn the_edits_of_a_change_request_run_the_checks_of_an_edit_and_report_only_what_they_introduce()
 {
    let mut app = app_with(
        "cr-introduced",
        registry(&[&Opened, &Decided, &Assumed, &Reopened]),
    )
    .await;
    let rules = |found: &[chidev_core::app::Found]| -> Vec<(String, Option<String>)> {
        found
            .iter()
            .map(|f| (f.rule.to_string(), f.about.clone()))
            .collect()
    };

    let a = app.open_change_request(&request("a")).await.unwrap();
    assert_eq!(
        rules(&a.findings),
        [(
            "test.cr-opened".to_string(),
            Some("change request `a`".to_string())
        )]
    );
    // The second request is told of itself; the first is still found, and was introduced by no one now.
    let b = app.open_change_request(&request("b")).await.unwrap();
    assert_eq!(
        rules(&b.findings),
        [(
            "test.cr-opened".to_string(),
            Some("change request `b`".to_string())
        )]
    );

    let assumed = app.assume("a", &assumption()).await.unwrap();
    assert_eq!(
        rules(&assumed.findings),
        [(
            "test.cr-assumed".to_string(),
            Some("change request `a`".to_string())
        )]
    );
    let again = app.assume("a", &assumption()).await.unwrap();
    assert!(again.findings.is_empty(), "{:?}", again.findings);

    // A question raised in the request is an edit of its record, and nothing of what it checks is introduced yet.
    let raised = app.raise_crux(&question("a")).await.unwrap();
    assert!(raised.findings.is_empty(), "{:?}", raised.findings);
    let decided = app
        .decide_crux(
            &raised.value.id.to_string(),
            &chose(&raised.value),
            None,
            &arbitrary(),
            &alice(),
        )
        .await
        .unwrap();
    assert_eq!(
        rules(&decided.findings),
        [(
            "test.cr-decided".to_string(),
            Some("change request `a`".to_string())
        )]
    );

    app.resolve_change_request("a", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();
    let reopened = app
        .reopen_change_request("a", "it came back", &alice())
        .await
        .unwrap();
    assert_eq!(
        rules(&reopened.findings),
        [(
            "test.cr-reopened".to_string(),
            Some("change request `a`".to_string())
        )]
    );
}

/// A mandatory check refuses each of the four edits for what it introduced, and nothing is kept.
#[tokio::test]
async fn a_mandatory_check_refuses_each_edit_of_a_change_request_that_introduces_what_it_finds() {
    async fn refused<T: std::fmt::Debug>(
        app: &App<SqliteStore, Counter>,
        rule: &str,
        result: Result<T, AppError>,
        revision: StoreRevision,
    ) {
        match result {
            Err(AppError::Refused(found)) => {
                assert_eq!(found[0].rule.as_str(), rule);
                assert_eq!(found[0].level, Level::Error);
            }
            other => panic!("expected {rule} to refuse, got {other:?}"),
        }
        assert_eq!(app.store().revision().await.unwrap(), revision, "{rule}");
    }

    // opening
    let mut app = app_with("cr-refuse-open", registry(&[&NoOpened])).await;
    let revision = app.store().revision().await.unwrap();
    let result = app.open_change_request(&request("a")).await;
    refused(&app, "test.no-cr-opened", result, revision).await;
    assert!(app.store().change_requests().await.unwrap().is_empty());

    // the rest need a request, made while nothing is mandatory
    let mut open = app_with("cr-refuse-rest", registry(&[])).await;
    open.open_change_request(&request("a")).await.unwrap();
    let raised = open.raise_crux(&question("a")).await.unwrap().value;
    open.open_change_request(&request("b")).await.unwrap();
    open.resolve_change_request("b", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();

    let mut app = under(open, registry(&[&NoDecided]), 100);
    let revision = app.store().revision().await.unwrap();
    let result = app
        .decide_crux(
            &raised.id.to_string(),
            &chose(&raised),
            None,
            &arbitrary(),
            &alice(),
        )
        .await;
    refused(&app, "test.no-cr-decided", result, revision).await;

    let mut app = under(app, registry(&[&NoAssumed]), 200);
    let result = app.assume("a", &assumption()).await;
    refused(&app, "test.no-cr-assumed", result, revision).await;

    let mut app = under(app, registry(&[&NoReopened]), 300);
    let result = app
        .reopen_change_request("b", "it came back", &alice())
        .await;
    refused(&app, "test.no-cr-reopened", result, revision).await;
    // none of the four changed the requests
    let cruxes = app.store().cruxes().await.unwrap();
    for cr in app.store().change_requests().await.unwrap() {
        assert!(
            !is_decided(&cr, &cruxes) && !is_assumed(&cr, &cruxes) && !was_reopened(&cr, &cruxes)
        );
    }
}

/// A check that finds the Project as a whole, mandatory, at every moment.
struct Always;

static ALWAYS: Descriptor = Descriptor {
    id: RuleId::from_static("test.always"),
    summary: Cow::Borrowed("always something"),
    help: Cow::Borrowed("only for the test"),
    moments: Cow::Borrowed(&[Moment::Edit, Moment::Pin, Moment::Resolve, Moment::Check]),
    default_level: Level::Error,
    mandatory: true,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for Always {
    fn descriptor(&self) -> &'static Descriptor {
        &ALWAYS
    }

    fn run(&self, _: &CheckView<'_>, _: &Params) -> Vec<Finding> {
        vec![Finding::new(&ALWAYS, None, "something is wrong")]
    }
}

/// Counts the times it is asked, and finds something every time.
#[derive(Debug, Clone, Default)]
struct Asked(Arc<AtomicUsize>);

impl ExternalExecutor for Asked {
    async fn execute(&self, _: &RuleId, _: ExternalCall) -> ExternalAttempt {
        self.0.fetch_add(1, Ordering::SeqCst);
        ExternalAttempt {
            evidence: ExecutionEvidence::default(),
            result: Err(ExternalFailure {
                kind: "exit".into(),
                message: "refuses everything".into(),
            }),
        }
    }
}

/// Making the Project is a record, and records run no check: no mandatory check, no registered program.
#[tokio::test]
async fn init_runs_no_check_and_no_external_gate() {
    let gate = ExternalRule::new(
        RuleId::from_static("ext.gate"),
        vec![Moment::Edit, Moment::Pin, Moment::Resolve, Moment::Check],
        vec![ModelSection::Contracts],
        true,
        "config".into(),
    )
    .unwrap();
    let registry = Registry::with_external(&[&Always], vec![], vec![gate], &[]).unwrap();
    let asked = Asked::default();
    let mut app = App::with_registry(open("init").await, Counter::default(), registry, clock())
        .with_external(asked.clone());
    app.init().await.unwrap();
    assert!(app.store().project().await.unwrap().is_some());
    assert_eq!(asked.0.load(Ordering::SeqCst), 0);
    assert!(app.store().invocations().await.unwrap().is_empty());
    assert!(app.store().external_reports().await.unwrap().is_empty());

    // The same checks do hold an edit: the program is asked, and the gate refuses.
    assert!(matches!(
        app.add_context("core", &[]).await,
        Err(AppError::ExternalRefused { .. })
    ));
    assert_eq!(asked.0.load(Ordering::SeqCst), 1);
}

/// An explicit question is kept even when what it finds would have refused an edit.
#[tokio::test]
async fn check_run_keeps_what_it_found_when_a_mandatory_check_finds_an_error() {
    let mut app = app_with("ask", registry(&[&Always])).await;
    let revision = app.store().revision().await.unwrap();
    let asked = app.check_run().await.unwrap();
    assert_eq!(asked.findings[0].rule.as_str(), "test.always");
    assert_eq!(asked.findings[0].level, Level::Error);
    assert_eq!(app.store().revision().await.unwrap().0, revision.0 + 1);
    let kept: Vec<_> = app
        .store()
        .invocations()
        .await
        .unwrap()
        .into_iter()
        .map(|(_, run)| run)
        .collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].moment, Moment::Check);
    assert_eq!(kept[0].trigger, Some(Trigger::Ask(AskTrigger::CheckRun)));
}

// ---- what is kept is what was checked ----

/// The model as a check sees it: the Project, the Pins and the change requests.
type State = (Project, Vec<Pin>, Vec<ChangeRequest>);

/// Remembers the states the checks were run on.
#[derive(Default)]
struct Seen {
    states: Mutex<Vec<State>>,
}

impl Seen {
    fn last(&self) -> State {
        self.states.lock().unwrap().last().cloned().unwrap()
    }
}

struct Recorder(&'static Seen);

static RECORDER: Descriptor = Descriptor {
    id: RuleId::from_static("test.recorder"),
    summary: Cow::Borrowed("looks at what it is given"),
    help: Cow::Borrowed("only for the test"),
    moments: Cow::Borrowed(&[Moment::Edit, Moment::Pin, Moment::Resolve, Moment::Check]),
    default_level: Level::Note,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for Recorder {
    fn descriptor(&self) -> &'static Descriptor {
        &RECORDER
    }

    fn run(&self, view: &CheckView<'_>, _: &Params) -> Vec<Finding> {
        self.0.states.lock().unwrap().push((
            view.project.clone(),
            view.pins.to_vec(),
            view.change_requests.to_vec(),
        ));
        Vec::new()
    }
}

fn by_interface(mut pins: Vec<Pin>) -> Vec<Pin> {
    pins.sort_by_key(|p| p.interface());
    pins
}

fn by_id(mut crs: Vec<ChangeRequest>) -> Vec<ChangeRequest> {
    crs.sort_by_key(|c| c.id());
    crs
}

/// The model as the store holds it.
async fn held(store: &SqliteStore) -> State {
    (
        store.project().await.unwrap().unwrap(),
        by_interface(store.pins().await.unwrap()),
        by_id(store.change_requests().await.unwrap()),
    )
}

/// For an edit and a confirmation: the state the checks ran on last is the state the store holds afterwards.
#[tokio::test]
async fn what_the_store_holds_after_an_edit_or_a_confirmation_is_the_state_that_was_checked() {
    let seen: &'static Seen = Box::leak(Box::default());
    let recorder: &'static Recorder = Box::leak(Box::new(Recorder(seen)));
    let mut app = app_with("reproduce", registry(&[recorder])).await;
    let checked = || {
        let (p, pins, crs) = seen.last();
        (p, by_interface(pins), by_id(crs))
    };

    // edits of the model
    app.add_context("core", &[]).await.unwrap();
    assert_eq!(held(app.store()).await, checked());
    app.put_contract("core", "stack", "Stack", &clauses())
        .await
        .unwrap();
    assert_eq!(held(app.store()).await, checked());
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
    assert_eq!(held(app.store()).await, checked());
    // an edit of a change request
    app.open_change_request(&request("a")).await.unwrap();
    assert_eq!(held(app.store()).await, checked());
    harness(&mut app).await;
    assert_eq!(held(app.store()).await, checked());
    // a confirmation that makes a Pin
    let pinned = app.pin("core", "api", "fn push() {}").await.unwrap();
    let (_, pins, _) = held(app.store()).await;
    assert_eq!(held(app.store()).await, checked());
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].id(), pinned.value);
    assert_eq!(
        app.store().pin_code(pinned.value).await.unwrap().as_deref(),
        Some("fn push() {}")
    );
    // the question of a change request, and its decision, which is kept with the change request it was raised in
    let raised = app.raise_crux(&question("a")).await.unwrap().value;
    assert_eq!(held(app.store()).await, checked());
    app.decide_crux(
        &raised.id.to_string(),
        &chose(&raised),
        None,
        &arbitrary(),
        &alice(),
    )
    .await
    .unwrap();
    assert_eq!(held(app.store()).await, checked());
    // and one that closes a change request
    app.resolve_change_request("a", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();
    assert_eq!(held(app.store()).await, checked());
}

/// A Pin changes no part of the model an agent is shown, so it leaves the model's revision where it was, and a
/// report made on a view read before it is not taken for one made on an older model.
#[tokio::test]
async fn a_pin_leaves_the_revision_of_the_model_and_a_report_made_before_it_is_not_on_an_older_model()
 {
    let rule = judged::parse(
        "id = \"judged.vague\"\nsummary = \"s\"\nsubject = \"clause\"\nmoments = [\"edit\"]\nlevel = \"warning\"\n\
         question = \"q\"\ntrue = \"t\"\nfalse = \"f\"\nfix = \"x\"\n",
    )
    .unwrap();
    let digest = rule.digest().to_string();
    let registry = Registry::builtin_with_judged(vec![rule], &[]).unwrap();
    let mut app = app_with("pin-model-revision", registry).await;
    stack(&mut app).await;

    let read = app.check_rules(None, None).await.unwrap();
    let model = app.store().model_revision().await.unwrap();
    app.pin("core", "api", "fn push() {}").await.unwrap();
    assert_eq!(app.store().model_revision().await.unwrap(), model);
    assert!(app.store().revision().await.unwrap() > read.store_revision);

    let reported = app
        .finding_report(&ReportInput {
            rule: "judged.vague".into(),
            rule_digest: digest,
            judge: "agent-1".into(),
            read_at: read.store_revision.0,
            findings: vec![ReportedFinding {
                subject: Some(SubjectInput::Clause(ClauseRef {
                    context: Some("core".into()),
                    contract: "stack".into(),
                    label: "C1".into(),
                })),
                message: "vague".into(),
                level: None,
            }],
        })
        .await
        .unwrap();
    assert!(!reported.on_older_revision);
}

// ---- the record of the checks a use case ran ----

/// The runs of the checks are kept under the use case that made them.
#[tokio::test]
async fn the_checks_a_use_case_ran_are_recorded_under_its_trigger() {
    let rule = judged::parse(
        "id = \"judged.vague\"\nsummary = \"s\"\nsubject = \"clause\"\nmoments = [\"edit\"]\nlevel = \"warning\"\n\
         question = \"q\"\ntrue = \"t\"\nfalse = \"f\"\nfix = \"x\"\n",
    )
    .unwrap();
    let digest = rule.digest().to_string();
    let mut app = app_with(
        "triggers",
        Registry::builtin_with_judged(vec![rule], &[]).unwrap(),
    )
    .await;

    async fn new_runs(
        app: &App<SqliteStore, Counter>,
        seen: &mut usize,
    ) -> Vec<(Moment, Option<Trigger>)> {
        let all = app.store().invocations().await.unwrap();
        let now = all[*seen..]
            .iter()
            .map(|(_, i)| (i.moment, i.trigger))
            .collect::<Vec<_>>();
        *seen = all.len();
        now
    }
    let mut seen = 0;
    let only = |runs: &[(Moment, Option<Trigger>)], moment: Moment, trigger: Trigger| {
        assert!(!runs.is_empty(), "{trigger:?} ran no check");
        for run in runs {
            assert_eq!(*run, (moment, Some(trigger)));
        }
    };

    app.add_context("core", &[]).await.unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(&runs, Moment::Edit, Trigger::Edit(EditTrigger::AddContext));
    app.put_contract("core", "stack", "Stack", &clauses())
        .await
        .unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(&runs, Moment::Edit, Trigger::Edit(EditTrigger::PutContract));
    app.put_clauses(
        "core",
        "stack",
        &[ClauseInput {
            label: "C2".into(),
            text: "pop of an empty stack fails".into(),
            replaces: None,
        }],
        &[],
    )
    .await
    .unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(&runs, Moment::Edit, Trigger::Edit(EditTrigger::PutClauses));
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
    let runs = new_runs(&app, &mut seen).await;
    only(
        &runs,
        Moment::Edit,
        Trigger::Edit(EditTrigger::PutInterface),
    );
    harness(&mut app).await;
    new_runs(&app, &mut seen).await;
    app.pin("core", "api", "fn push() {}").await.unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(&runs, Moment::Pin, Trigger::Confirm(ConfirmTrigger::Pin));
    app.open_change_request(&request("a")).await.unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(
        &runs,
        Moment::Edit,
        Trigger::Edit(EditTrigger::OpenChangeRequest),
    );
    app.raise_crux(&question("a")).await.unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(&runs, Moment::Edit, Trigger::Edit(EditTrigger::RaiseCrux));
    let raised = app.raise_crux(&question("a")).await.unwrap().value;
    new_runs(&app, &mut seen).await;
    // The first question is withdrawn, the second chosen: a decision is an edit under a trigger of its own.
    let open = app.store().cruxes().await.unwrap();
    assert_eq!(open.len(), 2);
    let first = open.iter().find(|q| q.id() != raised.id).unwrap();
    app.decide_crux(
        &first.id().to_string(),
        &DecisionInput::Withdrawn,
        None,
        &arbitrary(),
        &alice(),
    )
    .await
    .unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(&runs, Moment::Edit, Trigger::Edit(EditTrigger::DecideCrux));
    app.decide_crux(
        &raised.id.to_string(),
        &chose(&raised),
        None,
        &arbitrary(),
        &alice(),
    )
    .await
    .unwrap();
    app.resolve_change_request("a", &ResolutionInput::Done, "settled", &alice())
        .await
        .unwrap();
    let runs = new_runs(&app, &mut seen).await;
    assert!(runs.contains(&(
        Moment::Resolve,
        Some(Trigger::Confirm(ConfirmTrigger::ResolveChangeRequest))
    )));
    app.check_run().await.unwrap();
    let runs = new_runs(&app, &mut seen).await;
    only(&runs, Moment::Check, Trigger::Ask(AskTrigger::CheckRun));

    // What an agent judged is the run of that rule, recorded at the moment of a question.
    let read = app.check_rules(None, None).await.unwrap();
    app.finding_report(&ReportInput {
        rule: "judged.vague".into(),
        rule_digest: digest,
        judge: "agent-1".into(),
        read_at: read.store_revision.0,
        findings: vec![],
    })
    .await
    .unwrap();
    let runs = new_runs(&app, &mut seen).await;
    assert_eq!(
        runs,
        [(
            Moment::Check,
            Some(Trigger::Record(RecordTrigger::ReportFinding))
        )]
    );
}

// ---- a record is tried again when it loses a race ----

/// A store that refuses the first commits it is given, as a store another process is writing to would.
struct Flaky {
    inner: SqliteStore,
    refuse: AtomicUsize,
    attempts: AtomicUsize,
    /// When set, every revision asked for after the first one reads one later: another process wrote in between.
    moved: AtomicBool,
    asked: AtomicUsize,
}

impl Flaky {
    fn new(inner: SqliteStore) -> Flaky {
        Flaky {
            inner,
            refuse: AtomicUsize::new(0),
            attempts: AtomicUsize::new(0),
            moved: AtomicBool::new(false),
            asked: AtomicUsize::new(0),
        }
    }

    /// From now on, the store moves on once the use case has read its revision the first time.
    fn moves_after_the_first_read(&self) {
        self.asked.store(0, Ordering::SeqCst);
        self.moved.store(true, Ordering::SeqCst);
    }

    /// The next `times` commits are refused, and the count of the commits asked for starts again.
    fn refuse(&self, times: usize) {
        self.refuse.store(times, Ordering::SeqCst);
        self.attempts.store(0, Ordering::SeqCst);
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl ProjectRepository for Flaky {
    async fn project(&self) -> Result<Option<Project>, StoreError> {
        self.inner.project().await
    }
}

impl PinRepository for Flaky {
    async fn pin_of(&self, interface: InterfaceId) -> Result<Option<Pin>, StoreError> {
        self.inner.pin_of(interface).await
    }

    async fn pins(&self) -> Result<Vec<Pin>, StoreError> {
        self.inner.pins().await
    }
}

impl ChangeRequestRepository for Flaky {
    async fn change_request(
        &self,
        id: ChangeRequestId,
    ) -> Result<Option<ChangeRequest>, StoreError> {
        self.inner.change_request(id).await
    }

    async fn change_requests(&self) -> Result<Vec<ChangeRequest>, StoreError> {
        self.inner.change_requests().await
    }
}

impl CruxRepository for Flaky {
    async fn crux(&self, id: CruxId) -> Result<Option<Crux>, StoreError> {
        self.inner.crux(id).await
    }

    async fn cruxes(&self) -> Result<Vec<Crux>, StoreError> {
        self.inner.cruxes().await
    }

    async fn cruxes_raised_in(&self, raised_in: ChangeRequestId) -> Result<Vec<Crux>, StoreError> {
        self.inner.cruxes_raised_in(raised_in).await
    }
}

impl AssetNames for Flaky {
    async fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        self.inner.asset_id(context, kind, name).await
    }

    async fn asset_name(&self, id: Uuid) -> Result<Option<String>, StoreError> {
        self.inner.asset_name(id).await
    }
}

impl JudgedRepository for Flaky {
    async fn judged_reports(&self) -> Result<Vec<JudgedReport>, StoreError> {
        self.inner.judged_reports().await
    }
}

impl Store for Flaky {
    async fn external_reports(
        &self,
    ) -> Result<Vec<chidev_core::app::ports::external::ExternalReport>, StoreError> {
        self.inner.external_reports().await
    }

    async fn latest_measurements(&self) -> Result<Vec<Measurement>, StoreError> {
        self.inner.latest_measurements().await
    }

    async fn measurements(&self) -> Result<Vec<Measurement>, StoreError> {
        self.inner.measurements().await
    }

    async fn invocations(
        &self,
    ) -> Result<
        Vec<(
            chidev_core::app::ports::InvocationSeq,
            chidev_core::app::ports::Invocation,
        )>,
        StoreError,
    > {
        self.inner.invocations().await
    }

    async fn clear_freed(&self) -> Result<(), StoreError> {
        self.inner.clear_freed().await
    }

    async fn pin_code(&self, pin: PinId) -> Result<Option<String>, StoreError> {
        self.inner.pin_code(pin).await
    }

    async fn revision(&self) -> Result<StoreRevision, StoreError> {
        let at = self.inner.revision().await?;
        if self.moved.load(Ordering::SeqCst) && self.asked.fetch_add(1, Ordering::SeqCst) > 0 {
            return Ok(StoreRevision(at.0 + 1));
        }
        Ok(at)
    }

    async fn model_revision(&self) -> Result<StoreRevision, StoreError> {
        self.inner.model_revision().await
    }

    async fn commit(&self, read_at: StoreRevision, gated: Gated) -> Result<(), StoreError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let left = self.refuse.load(Ordering::SeqCst);
        if left > 0 {
            self.refuse.store(left - 1, Ordering::SeqCst);
            return Err(StoreError::Conflict);
        }
        self.inner.commit(read_at, gated).await
    }
}

/// A runner that measures nothing and finds it fine.
#[derive(Debug)]
struct Quiet;

impl Runner for Quiet {
    fn settings<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, RunnerError>> {
        Box::pin(async { Ok("settings".to_string()) })
    }

    fn mutate<'a>(
        &'a self,
        _: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(async {
            Ok(MutateReport {
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
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }

    fn simulate<'a>(
        &'a self,
        _: &'a SimulateRequest,
    ) -> BoxFuture<'a, Result<SimulateReport, RunnerError>> {
        Box::pin(async { Err(RunnerError::Unconfigured) })
    }
}

/// A Context `core` with a Contract `stack`, an Interface `api` and a Harness `tests` that checks it.
async fn stack<S: Store>(app: &mut App<S, Counter>) {
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
    harness(app).await;
}

/// A Harness `tests` of the Context `core` that checks the Interface `api` and the Contract `stack`.
async fn harness<S: Store>(app: &mut App<S, Counter>) {
    app.put_harness(
        "core",
        "tests",
        "fn pushes() {}\n",
        &[CaseInput {
            name: "pushes".into(),
            lines: (1, 1),
            items: vec![ItemRef {
                context: None,
                interface: "api".into(),
                key: "push".into(),
            }],
            clauses: vec![ClauseRef {
                context: None,
                contract: "stack".into(),
                label: "C1".into(),
            }],
        }],
    )
    .await
    .unwrap();
}

/// A Project with a pinned Interface that a Harness checks: what `measure` needs.
async fn pinned(store: Flaky) -> App<Flaky, Counter> {
    let mut app = App::new(store, Counter::default(), clock()).with_runner(Quiet);
    app.init().await.unwrap();
    stack(&mut app).await;
    app.pin("core", "api", "fn push() {}").await.unwrap();
    app
}

#[tokio::test]
async fn a_measurement_that_loses_a_race_is_kept_when_it_is_tried_again() {
    let mut app = pinned(Flaky::new(open("measure-retry").await)).await;
    app.store().refuse(2);
    app.measure(MeasureKind::Mutate, "core", "api")
        .await
        .unwrap();
    assert_eq!(app.store().attempts(), 3);
    assert_eq!(app.store().latest_measurements().await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_edit_whose_store_moved_while_it_read_is_a_conflict_before_anything_is_judged() {
    let mut app = pinned(Flaky::new(open("edit-moved").await)).await;
    let revision = app.store().inner.revision().await.unwrap();
    let attempts = app.store().attempts();
    app.store().moves_after_the_first_read();
    let error = app
        .put_contract("core", "stack", "Stack", &clauses())
        .await
        .unwrap_err();
    assert_eq!(error, AppError::Store(StoreError::Conflict));
    // Nothing was kept, and the store was not asked to keep anything.
    assert_eq!(app.store().attempts(), attempts);
    assert_eq!(app.store().inner.revision().await.unwrap(), revision);
}

#[tokio::test]
async fn a_measurement_that_keeps_losing_the_race_is_refused_after_a_few_tries() {
    let mut app = pinned(Flaky::new(open("measure-exhausted").await)).await;
    app.store().refuse(1_000);
    let error = app
        .measure(MeasureKind::Mutate, "core", "api")
        .await
        .unwrap_err();
    assert_eq!(error, AppError::Store(StoreError::Conflict));
    let tries = app.store().attempts();
    assert!((2..=10).contains(&tries), "{tries} tries");
    assert!(app.store().latest_measurements().await.unwrap().is_empty());
}
