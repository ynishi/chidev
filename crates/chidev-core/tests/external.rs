//! External checks use a fake process port here so these tests cover application policy and store boundaries.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chidev_core::app::commit::{AskTrigger, Trigger};
use chidev_core::app::ports::external::{
    ExecutionEvidence, ExternalAttempt, ExternalCall, ExternalExecutor, ExternalFailure,
    ExternalFinding, ExternalReply, ExternalSubject,
};
use chidev_core::app::ports::{IdSource, Invocation, Store};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, DecisionMakerInput,
    FindingInput, Intent, ItemInput, ItemRef, Registry, ResolutionInput, SubjectInput, TargetInput,
};
use chidev_core::domain::check::external::ExternalRule;
use chidev_core::domain::check::{Executor, Level, Moment, Origin};
use chidev_core::domain::repository::{
    ChangeRequestRepository, PinRepository, ProjectRepository, StoreError,
};
use chidev_core::domain::vo::finding::RuleId;
use chidev_core::domain::vo::model_section::ModelSection;
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

#[derive(Clone, Debug, Default)]
struct FakeExecutor {
    calls: Arc<Mutex<Vec<ExternalCall>>>,
    replies: Arc<Mutex<VecDeque<Result<ExternalReply, ExternalFailure>>>>,
    concurrent_write: Option<PathBuf>,
}

impl FakeExecutor {
    fn answering(reply: Result<ExternalReply, ExternalFailure>) -> Self {
        let this = Self::default();
        this.replies.lock().unwrap().push_back(reply);
        this
    }

    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl ExternalExecutor for FakeExecutor {
    async fn execute(&self, rule: &RuleId, call: ExternalCall) -> ExternalAttempt {
        assert_eq!(rule.as_str(), "ext.contract");
        if let Some(path) = &self.concurrent_write {
            // Another process writes while this one waits for the program: a real use case of a second `App` over
            // the same file, with no check to run, so that it moves the store's revision and nothing else.
            let other = SqliteStore::open(path).await.unwrap();
            let registry = Registry::with_external(&[], vec![], vec![], &[]).unwrap();
            let mut other = App::with_registry(
                other,
                Counter(1_000),
                registry,
                FixedClock(Timestamp::from_millis(1_000)),
            );
            other.check_run().await.unwrap();
            other.into_store().close().await.unwrap();
        }
        self.calls.lock().unwrap().push(call);
        ExternalAttempt {
            evidence: ExecutionEvidence {
                command: Some("/test/checker".into()),
                command_digest: Some("a".repeat(64)),
                config_digest: "config-v1".into(),
                exit_code: Some(0),
                stderr: String::new(),
            },
            result: self.replies.lock().unwrap().pop_front().unwrap_or_else(|| {
                Ok(ExternalReply {
                    protocol: 1,
                    findings: vec![],
                })
            }),
        }
    }
}

fn rule(hooks: Vec<Moment>, gate: bool, config: &str) -> ExternalRule {
    ExternalRule::new(
        RuleId::from_static("ext.contract"),
        hooks,
        vec![ModelSection::Contracts],
        gate,
        config.into(),
    )
    .unwrap()
}

async fn app(
    name: &str,
    hooks: Vec<Moment>,
    gate: bool,
    executor: FakeExecutor,
) -> App<SqliteStore, Counter, FakeExecutor> {
    app_with_reads(name, hooks, gate, executor, vec![ModelSection::Contracts]).await
}

/// The runs the store has kept, without the identity of their rows.
async fn ran(app: &App<SqliteStore, Counter, FakeExecutor>) -> Vec<Invocation> {
    app.store()
        .invocations()
        .await
        .unwrap()
        .into_iter()
        .map(|(_, run)| run)
        .collect()
}

async fn app_with_reads(
    name: &str,
    hooks: Vec<Moment>,
    gate: bool,
    executor: FakeExecutor,
    reads: Vec<ModelSection>,
) -> App<SqliteStore, Counter, FakeExecutor> {
    let dir = std::env::temp_dir().join(format!("chidev-external-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
    let external = ExternalRule::new(
        RuleId::from_static("ext.contract"),
        hooks,
        reads,
        gate,
        "config-v1".into(),
    )
    .unwrap();
    // The project is set up with no check registered: adding the Context is an edit, and the registered program
    // would be asked about it (and its answer kept). The ids used here are the first two of the counter.
    let mut setup = App::with_registry(
        store,
        Counter::default(),
        Registry::with_external(&[], vec![], vec![], &[]).unwrap(),
        FixedClock(Timestamp::from_millis(1_000)),
    );
    setup.init().await.unwrap();
    setup.add_context("core", &[]).await.unwrap();
    let registry = Registry::with_external(&[], vec![], vec![external], &[]).unwrap();
    App::with_registry(
        setup.into_store(),
        Counter(2),
        registry,
        FixedClock(Timestamp::from_millis(1_000)),
    )
    .with_external(executor)
}

fn clauses(text: &str) -> Vec<ClauseInput> {
    vec![ClauseInput {
        label: "C1".into(),
        text: text.into(),
        replaces: None,
    }]
}

fn finding(message: &str) -> ExternalFinding {
    ExternalFinding {
        rule: "ext.contract".into(),
        subject: None,
        message: message.into(),
        level: None,
    }
}

fn reply(findings: Vec<ExternalFinding>) -> Result<ExternalReply, ExternalFailure> {
    Ok(ExternalReply {
        protocol: 1,
        findings,
    })
}

fn failed() -> Result<ExternalReply, ExternalFailure> {
    Err(ExternalFailure {
        kind: "timeout".into(),
        message: "test checker exceeded its deadline".into(),
    })
}

#[tokio::test]
async fn advisory_failure_is_visible_and_kept_with_the_accepted_change() {
    let executor = FakeExecutor::answering(failed());
    let mut app = app(
        "advisory-failure",
        vec![Moment::Edit],
        false,
        executor.clone(),
    )
    .await;
    let put = app
        .put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap();
    assert_eq!(executor.count(), 1);
    assert_eq!(put.findings.len(), 1);
    assert_eq!(put.findings[0].level, Level::Warning);
    let reports = app.store().external_reports().await.unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].failure.as_ref().unwrap().contains("timeout"));
    let invocations = ran(&app).await;
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].executor, Executor::External);
    assert_eq!(invocations[0].warnings, 1);
    assert_eq!(
        invocations[0].external.as_ref().unwrap().command.as_deref(),
        Some("/test/checker")
    );
}

#[tokio::test]
async fn a_gate_failure_refuses_the_whole_change_and_returns_its_evidence() {
    let executor = FakeExecutor::answering(failed());
    let mut app = app("gate-failure", vec![Moment::Edit], true, executor).await;
    let before = app.store().revision().await.unwrap();
    let project = app.store().project().await.unwrap();
    let result = app
        .put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await;
    match result.unwrap_err() {
        AppError::ExternalRefused { findings, external } => {
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].level, Level::Error);
            assert_eq!(external.len(), 1);
            assert!(external[0].failure.is_some());
            assert_eq!(
                external[0].evidence.command.as_deref(),
                Some("/test/checker")
            );
        }
        error => panic!("expected external gate refusal, got {error:?}"),
    }
    assert_eq!(app.store().revision().await.unwrap(), before);
    assert_eq!(app.store().project().await.unwrap(), project);
    assert!(app.store().external_reports().await.unwrap().is_empty());
    assert!(app.store().invocations().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_gate_also_refuses_a_finding_the_program_calls_a_note() {
    let mut note = finding("contract contradicts itself");
    note.level = Some("note".into());
    let mut app = app(
        "gate-note",
        vec![Moment::Edit],
        true,
        FakeExecutor::answering(reply(vec![note])),
    )
    .await;
    let error = app
        .put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap_err();
    match error {
        AppError::ExternalRefused { findings, external } => {
            assert_eq!(findings[0].level, Level::Error);
            assert!(external[0].failure.is_none());
        }
        error => panic!("expected external gate refusal, got {error:?}"),
    }
}

#[tokio::test]
async fn advisory_findings_cannot_promote_themselves_above_warning() {
    let mut error = finding("check this clause");
    error.level = Some("error".into());
    let mut app = app(
        "advisory-error",
        vec![Moment::Check],
        false,
        FakeExecutor::answering(reply(vec![error])),
    )
    .await;
    let checked = app.check_run().await.unwrap();
    assert_eq!(checked.findings[0].level, Level::Warning);
    assert!(checked.external[0].failure.is_none());
}

#[tokio::test]
async fn status_and_check_do_not_run_a_program_but_explicit_check_run_does() {
    let executor = FakeExecutor::default();
    let mut app = app("explicit-run", vec![Moment::Check], false, executor.clone()).await;
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "unrun"
    );
    assert!(app.check(None).await.unwrap().findings.is_empty());
    assert_eq!(executor.count(), 0);
    let model_revision = app.store().model_revision().await.unwrap();
    let checked = app.check_run().await.unwrap();
    assert_eq!(executor.count(), 1);
    assert!(checked.findings.is_empty());
    assert_eq!(checked.external.len(), 1);
    assert_eq!(app.store().model_revision().await.unwrap(), model_revision);
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current"
    );
    assert!(app.check(None).await.unwrap().findings.is_empty());
    assert_eq!(executor.count(), 1);
}

#[tokio::test]
async fn only_requested_assets_are_sent_and_a_changed_input_makes_the_report_stale() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "reads-and-stale",
        vec![Moment::Check],
        false,
        executor.clone(),
    )
    .await;
    app.put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap();
    app.put_interface(
        "core",
        "stack-api",
        "fn push()",
        &[ItemInput {
            key: "push".into(),
            lines: (1, 1),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    app.check_run().await.unwrap();
    let call = executor.calls.lock().unwrap()[0].clone();
    assert_eq!(call.protocol, 2);
    assert_eq!(call.moment, "check");
    assert_eq!(call.rules, vec!["ext.contract"]);
    // a program reads the model as JSON in another process, and so does this test
    let model = serde_json::to_value(&call.model).unwrap();
    assert!(model.get("change_requests").is_none());
    let context = &model["contexts"][0];
    assert_eq!(
        context["contracts"][0]["clauses"][0]["text"],
        "push then pop"
    );
    assert!(context.get("interfaces").is_none());
    assert!(context.get("harnesses").is_none());
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current"
    );
    app.put_interface(
        "core",
        "stack-api",
        "fn push(value: u32)",
        &[ItemInput {
            key: "push".into(),
            lines: (1, 1),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current",
        "an unread interface is not part of the check's input"
    );
    app.put_contract(
        "core",
        "stack",
        "Stack",
        &clauses("pop removes the last push"),
    )
    .await
    .unwrap();
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "stale"
    );
    assert_eq!(executor.count(), 1);
}

#[tokio::test]
async fn malformed_responses_are_failed_attempts_instead_of_successful_checks() {
    let mut wrong_rule = finding("wrong namespace");
    wrong_rule.rule = "ext.other".into();
    let mut bad_level = finding("unknown severity");
    bad_level.level = Some("fatal".into());
    let mut blank = finding("valid before trimming");
    blank.message = "  \n ".into();
    for (name, response) in [
        ("wrong-rule", reply(vec![wrong_rule])),
        ("bad-level", reply(vec![bad_level])),
        ("blank-message", reply(vec![blank])),
        (
            "wrong-version",
            Ok(ExternalReply {
                protocol: 3,
                findings: vec![],
            }),
        ),
    ] {
        let mut app = app(
            name,
            vec![Moment::Check],
            false,
            FakeExecutor::answering(response),
        )
        .await;
        let checked = app.check_run().await.unwrap();
        assert!(checked.external[0].failure.is_some(), "{name}");
        assert_eq!(checked.findings.len(), 1, "{name}");
        assert_eq!(checked.findings[0].level, Level::Warning, "{name}");
        assert_eq!(checked.findings[0].subject, None, "{name}");
    }
}

#[tokio::test]
async fn a_reply_of_version_one_or_two_is_accepted_and_the_request_names_the_moment() {
    for protocol in [1, 2] {
        let executor = FakeExecutor::answering(Ok(ExternalReply {
            protocol,
            findings: vec![finding("worth a look")],
        }));
        let mut app = app(
            &format!("reply-v{protocol}"),
            vec![Moment::Edit],
            false,
            executor.clone(),
        )
        .await;
        let put = app
            .put_contract("core", "rules", "Rules", &clauses("a rule"))
            .await
            .unwrap();
        assert_eq!(put.findings.len(), 1, "version {protocol}");
        let call = executor.calls.lock().unwrap()[0].clone();
        // The request is version 2 whatever the reply is, and names the moment by its word.
        assert_eq!(call.protocol, 2);
        assert_eq!(call.moment, "edit");
        let sent = serde_json::to_value(&call).unwrap();
        assert!(sent.get("hook").is_none(), "{sent}");
        assert_eq!(sent["moment"], "edit");
    }
}

#[tokio::test]
async fn a_subject_that_exists_but_was_not_sent_is_refused() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "hidden-subject",
        vec![Moment::Check],
        false,
        executor.clone(),
    )
    .await;
    let interface = app
        .put_interface(
            "core",
            "api",
            "fn push()",
            &[ItemInput {
                key: "push".into(),
                lines: (1, 1),
                replaces: None,
            }],
        )
        .await
        .unwrap();
    let mut hidden = finding("not allowed to report an unseen interface");
    hidden.subject = Some(ExternalSubject {
        kind: "interface".into(),
        id: interface.value.id.to_string(),
    });
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![hidden]));
    let checked = app.check_run().await.unwrap();
    assert!(checked.external[0].failure.is_some());
    assert_eq!(checked.findings[0].subject, None);
}

#[tokio::test]
async fn a_valid_clause_finding_is_kept_and_an_empty_success_replaces_it() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "valid-subject",
        vec![Moment::Check],
        false,
        executor.clone(),
    )
    .await;
    let contract = app
        .put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap();
    let project = app.store().project().await.unwrap().unwrap();
    let clause = project
        .contexts()
        .flat_map(|context| context.contracts())
        .find(|candidate| candidate.id() == contract.value.id)
        .unwrap()
        .clauses()[0]
        .id();
    let mut reported = finding("the contract needs clarification");
    reported.subject = Some(ExternalSubject {
        kind: "clause".into(),
        id: clause.to_string(),
    });
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![reported]));
    let checked = app.check_run().await.unwrap();
    assert!(checked.external[0].failure.is_none());
    assert_eq!(checked.findings.len(), 1);
    assert!(checked.findings[0].subject.is_some());
    assert_eq!(app.check(None).await.unwrap().findings.len(), 1);
    let checked = app.check_run().await.unwrap();
    assert!(checked.findings.is_empty());
    assert!(app.check(None).await.unwrap().findings.is_empty());
    let reports = app.store().external_reports().await.unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].findings.is_empty());
    assert_eq!(app.store().invocations().await.unwrap().len(), 2);
}

#[tokio::test]
async fn explicit_check_records_a_failed_gate_without_changing_the_model() {
    let mut app = app(
        "explicit-gate",
        vec![Moment::Check],
        true,
        FakeExecutor::answering(failed()),
    )
    .await;
    let project = app.store().project().await.unwrap();
    let model_revision = app.store().model_revision().await.unwrap();
    let checked = app.check_run().await.unwrap();
    assert_eq!(checked.findings[0].level, Level::Error);
    assert!(checked.external[0].failure.is_some());
    assert_eq!(app.store().external_reports().await.unwrap().len(), 1);
    assert_eq!(app.store().project().await.unwrap(), project);
    assert_eq!(app.store().model_revision().await.unwrap(), model_revision);
    // The run is kept as the run of the question that was asked.
    let runs = ran(&app).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].moment, Moment::Check);
    assert_eq!(runs[0].trigger, Some(Trigger::Ask(AskTrigger::CheckRun)));
}

#[tokio::test]
async fn changing_the_registered_process_configuration_invalidates_its_previous_result() {
    let executor = FakeExecutor::default();
    let mut app = app("config-stale", vec![Moment::Check], false, executor.clone()).await;
    app.check_run().await.unwrap();
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current"
    );
    let store = app.into_store();
    let registry = Registry::with_external(
        &[],
        vec![],
        vec![rule(vec![Moment::Check], false, "config-v2")],
        &[],
    )
    .unwrap();
    let app = App::with_registry(
        store,
        Counter::default(),
        registry,
        FixedClock(Timestamp::from_millis(1_000)),
    )
    .with_external(executor.clone());
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "stale"
    );
    assert_eq!(executor.count(), 1);
}

#[tokio::test]
async fn a_policy_disabled_rule_is_not_executed_and_a_gate_cannot_be_disabled() {
    let layer = chidev_core::infra::policy::parse(
        "[checks.\"ext.contract\"]\nlevel = \"off\"\n",
        Origin::Project,
    )
    .unwrap();
    let registry = Registry::with_external(
        &[],
        vec![],
        vec![rule(vec![Moment::Check], true, "config-v1")],
        std::slice::from_ref(&layer),
    );
    assert!(registry.is_err());

    let executor = FakeExecutor::default();
    let app = app("policy-off", vec![Moment::Check], false, executor.clone()).await;
    let registry = Registry::with_external(
        &[],
        vec![],
        vec![rule(vec![Moment::Check], false, "config-v1")],
        &[layer],
    )
    .unwrap();
    let mut app = App::with_registry(
        app.into_store(),
        Counter::default(),
        registry,
        FixedClock(Timestamp::from_millis(1_000)),
    )
    .with_external(executor.clone());
    let checked = app.check_run().await.unwrap();
    assert!(checked.findings.is_empty());
    assert!(checked.external.is_empty());
    assert_eq!(executor.count(), 0);
}

#[tokio::test]
async fn a_concurrent_write_rejects_the_result_without_retrying_the_external_program() {
    let name = "concurrent-write";
    let path = std::env::temp_dir()
        .join(format!("chidev-external-{name}-{}", std::process::id()))
        .join("chidev.db");
    let executor = FakeExecutor {
        concurrent_write: Some(path),
        ..FakeExecutor::default()
    };
    let mut app = app(name, vec![Moment::Check], false, executor.clone()).await;
    let revision = app.store().revision().await.unwrap();
    let model_revision = app.store().model_revision().await.unwrap();
    let error = app.check_run().await.unwrap_err();
    assert_eq!(error, AppError::Store(StoreError::Conflict));
    assert_eq!(executor.count(), 1);
    assert_eq!(app.store().revision().await.unwrap().0, revision.0 + 1);
    assert_eq!(app.store().model_revision().await.unwrap(), model_revision);
    assert!(app.store().external_reports().await.unwrap().is_empty());
    assert!(app.store().invocations().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_pin_hook_refuses_then_commits_the_pin_and_its_check_together() {
    let executor = FakeExecutor::answering(failed());
    let mut app = app("pin-hook", vec![Moment::Pin], true, executor.clone()).await;
    let interface = app
        .put_interface(
            "core",
            "api",
            "fn push()",
            &[ItemInput {
                key: "push".into(),
                lines: (1, 1),
                replaces: None,
            }],
        )
        .await
        .unwrap();
    app.put_harness(
        "core",
        "stack-tests",
        "fn checks_push()",
        &[CaseInput {
            name: "checks_push".into(),
            lines: (1, 1),
            items: vec![ItemRef {
                context: None,
                interface: "api".into(),
                key: "push".into(),
            }],
            clauses: vec![],
        }],
    )
    .await
    .unwrap();
    let revision = app.store().revision().await.unwrap();
    assert!(matches!(
        app.pin("core", "api", "implementation-v1").await,
        Err(AppError::ExternalRefused { .. })
    ));
    assert_eq!(app.store().revision().await.unwrap(), revision);
    assert!(
        app.store()
            .pin_of(interface.value.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(app.store().external_reports().await.unwrap().is_empty());
    assert!(app.store().invocations().await.unwrap().is_empty());

    let pinned = app.pin("core", "api", "implementation-v1").await.unwrap();
    assert_eq!(
        app.store()
            .pin_of(interface.value.id)
            .await
            .unwrap()
            .unwrap()
            .id(),
        pinned.value
    );
    assert_eq!(app.store().revision().await.unwrap().0, revision.0 + 1);
    assert_eq!(app.store().external_reports().await.unwrap().len(), 1);
    let invocations = ran(&app).await;
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].moment, Moment::Pin);
    assert_eq!(invocations[0].read_at, revision);
    assert_eq!(executor.count(), 2);
    assert!(
        executor
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call.moment == "pin")
    );
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current"
    );
}

#[tokio::test]
async fn the_resolve_hook_sees_the_candidate_cr_and_commits_it_with_the_check() {
    let executor = FakeExecutor::answering(failed());
    let mut app = app_with_reads(
        "resolve-hook",
        vec![Moment::Resolve],
        true,
        executor.clone(),
        vec![ModelSection::ChangeRequests],
    )
    .await;
    let cr = app
        .open_change_request(&ChangeRequestInput {
            title: "accept current behavior".into(),
            situation: None,
            intent: Intent::KeepBehavior,
            parent: None,
            issuer: None,
            targets: vec![],
            keep: vec![],
            assumptions: vec![],
            addresses: vec![],
        })
        .await
        .unwrap();
    let revision = app.store().revision().await.unwrap();
    let requests_before = app.store().change_requests().await.unwrap();
    let how = ResolutionInput::Done;
    let by = DecisionMakerInput::Person("alice".to_string());
    assert!(matches!(
        app.resolve_change_request("accept current behavior", &how, "settled", &by)
            .await,
        Err(AppError::ExternalRefused { .. })
    ));
    assert_eq!(app.store().revision().await.unwrap(), revision);
    assert_eq!(
        app.store().change_requests().await.unwrap(),
        requests_before
    );
    assert!(app.store().external_reports().await.unwrap().is_empty());
    assert!(app.store().invocations().await.unwrap().is_empty());
    let call = executor.calls.lock().unwrap()[0].clone();
    assert_eq!(call.moment, "resolve");
    let model = serde_json::to_value(&call.model).unwrap();
    let candidate = &model["change_requests"][0];
    assert_eq!(candidate["id"], cr.value.id.to_string());
    assert_eq!(candidate["state"], "resolved");

    app.resolve_change_request("accept current behavior", &how, "settled", &by)
        .await
        .unwrap();
    assert!(app.store().change_requests().await.unwrap()[0].is_resolved());
    assert_eq!(app.store().revision().await.unwrap().0, revision.0 + 1);
    assert_eq!(app.store().external_reports().await.unwrap().len(), 1);
    let invocations = ran(&app).await;
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].moment, Moment::Resolve);
    assert_eq!(invocations[0].read_at, revision);
    assert_eq!(executor.count(), 2);
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current"
    );
}

/// The id of the only Clause of the Contract `contract` of the Project.
async fn clause_of(
    app: &App<SqliteStore, Counter, FakeExecutor>,
    contract: chidev_core::domain::vo::id::ContractId,
) -> Uuid {
    let project = app.store().project().await.unwrap().unwrap();
    project
        .contexts()
        .flat_map(|context| context.contracts())
        .find(|candidate| candidate.id() == contract)
        .unwrap()
        .clauses()[0]
        .id()
        .as_uuid()
}

#[tokio::test]
async fn a_put_is_refused_by_a_gate_for_what_it_introduced_and_not_for_what_the_saved_report_had() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "gate-introduced",
        vec![Moment::Edit, Moment::Check],
        true,
        executor.clone(),
    )
    .await;
    let contract = app
        .put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap();
    // An explicit run records a finding about the model as it is.
    let known = finding("the contract contradicts itself");
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![known.clone()]));
    app.check_run().await.unwrap();
    // A put that leaves the program saying the same of a model it described is not refused for it, and does not
    // say it again.
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![known.clone()]));
    let kept = app
        .put_contract("core", "stack", "Stack", &clauses("push, then pop"))
        .await
        .unwrap();
    assert!(kept.findings.is_empty(), "{:?}", kept.findings);
    // What is recorded is what the program said, all of it.
    let reports = app.store().external_reports().await.unwrap();
    assert_eq!(reports[0].findings.len(), 1);
    // A finding it did not have before refuses the put, and only that one is named.
    let clause = clause_of(&app, contract.value.id).await;
    let mut fresh = finding("the clause is vague");
    fresh.subject = Some(ExternalSubject {
        kind: "clause".into(),
        id: clause.to_string(),
    });
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![known, fresh]));
    let revision = app.store().revision().await.unwrap();
    match app
        .put_contract("core", "stack", "Stack", &clauses("push, then pop, once"))
        .await
        .unwrap_err()
    {
        AppError::ExternalRefused { findings, external } => {
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert!(findings[0].subject.is_some());
            // The attempts return whole.
            assert_eq!(external[0].findings.len(), 2);
        }
        error => panic!("expected external gate refusal, got {error:?}"),
    }
    assert_eq!(app.store().revision().await.unwrap(), revision);
}

#[tokio::test]
async fn a_new_finding_about_the_whole_project_refuses_a_put_beside_one_the_saved_report_had() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "gate-project-wide",
        vec![Moment::Edit, Moment::Check],
        true,
        executor.clone(),
    )
    .await;
    app.put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap();
    let known = finding("the project has no glossary");
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![known.clone()]));
    app.check_run().await.unwrap();
    // Both are about the whole Project (no subject); only the message tells them apart.
    let fresh = finding("the project names two owners");
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![known, fresh]));
    let revision = app.store().revision().await.unwrap();
    match app
        .put_contract("core", "stack", "Stack", &clauses("push, then pop"))
        .await
        .unwrap_err()
    {
        AppError::ExternalRefused { findings, .. } => {
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert!(findings[0].subject.is_none());
            assert!(findings[0].message.contains("two owners"), "{findings:?}");
        }
        error => panic!("expected external gate refusal, got {error:?}"),
    }
    assert_eq!(app.store().revision().await.unwrap(), revision);
}

#[tokio::test]
async fn a_gate_judges_every_finding_when_the_saved_report_does_not_describe_the_model_before() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "gate-no-baseline",
        vec![Moment::Edit, Moment::Check],
        true,
        executor.clone(),
    )
    .await;
    app.put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap();
    let known = finding("the contract contradicts itself");
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![known.clone()]));
    app.check_run().await.unwrap();
    // The same program under another configuration: what it saved says nothing of this one.
    let store = app.into_store();
    let registry = Registry::with_external(
        &[],
        vec![],
        vec![rule(vec![Moment::Edit, Moment::Check], true, "config-v2")],
        &[],
    )
    .unwrap();
    let mut app = App::with_registry(
        store,
        Counter(1_000),
        registry,
        FixedClock(Timestamp::from_millis(1_000)),
    )
    .with_external(executor.clone());
    executor
        .replies
        .lock()
        .unwrap()
        .push_back(reply(vec![known.clone()]));
    assert!(matches!(
        app.put_contract("core", "stack", "Stack", &clauses("push, then pop"))
            .await,
        Err(AppError::ExternalRefused { .. })
    ));
    // And with no saved report at all: all of it is judged.
    let mut app = self::app(
        "gate-no-report",
        vec![Moment::Edit],
        true,
        FakeExecutor::answering(reply(vec![known])),
    )
    .await;
    assert!(matches!(
        app.put_contract("core", "stack", "Stack", &clauses("push then pop"))
            .await,
        Err(AppError::ExternalRefused { .. })
    ));
}

#[tokio::test]
async fn a_failed_run_refuses_a_gate_put_even_when_the_saved_report_failed_the_same_way() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "gate-failed-again",
        vec![Moment::Edit, Moment::Check],
        true,
        executor.clone(),
    )
    .await;
    executor.replies.lock().unwrap().push_back(failed());
    app.check_run().await.unwrap();
    assert!(
        app.store().external_reports().await.unwrap()[0]
            .failure
            .is_some()
    );
    executor.replies.lock().unwrap().push_back(failed());
    assert!(matches!(
        app.put_contract("core", "stack", "Stack", &clauses("push then pop"))
            .await,
        Err(AppError::ExternalRefused { .. })
    ));
}

/// The reads of a rule, from the words a person writes in `chidev.toml`.
fn reads(words: &[&str]) -> Vec<ModelSection> {
    words
        .iter()
        .map(|w| ModelSection::from_word(w).unwrap())
        .collect()
}

/// A model with every kind of asset and two change requests, so that what a program is sent has something in each
/// section: two Contexts, a Contract, an Interface with Items, a Harness with a Case that cites both.
async fn populate(app: &mut App<SqliteStore, Counter, FakeExecutor>) {
    app.add_context("util", &["core".to_string()])
        .await
        .unwrap();
    app.put_contract(
        "core",
        "rules",
        "Rules",
        &[
            ClauseInput {
                label: "C1".into(),
                text: "push then pop returns the value".into(),
                replaces: None,
            },
            ClauseInput {
                label: "C2".into(),
                text: "pop on empty is an error".into(),
                replaces: None,
            },
        ],
    )
    .await
    .unwrap();
    app.put_contract(
        "util",
        "misc",
        "Misc",
        &clauses("a \"quoted\" <word> & more"),
    )
    .await
    .unwrap();
    let items = ["push", "pop"]
        .iter()
        .enumerate()
        .map(|(k, key)| ItemInput {
            key: (*key).into(),
            lines: (k + 1, k + 1),
            replaces: None,
        })
        .collect::<Vec<_>>();
    app.put_interface("core", "stack", "fn push()\nfn pop()\n", &items)
        .await
        .unwrap();
    app.put_harness(
        "core",
        "tests",
        "fn pushes() {}\n",
        &[CaseInput {
            name: "pushes".into(),
            lines: (1, 1),
            items: vec![ItemRef {
                context: None,
                interface: "stack".into(),
                key: "push".into(),
            }],
            clauses: vec![ClauseRef {
                context: None,
                contract: "rules".into(),
                label: "C1".into(),
            }],
        }],
    )
    .await
    .unwrap();
    app.open_change_request(&ChangeRequestInput {
        title: "keep push".into(),
        situation: None,
        intent: Intent::KeepBehavior,
        parent: None,
        issuer: None,
        targets: vec![],
        keep: vec![TargetInput::Item(ItemRef {
            context: Some("core".into()),
            interface: "stack".into(),
            key: "push".into(),
        })],
        assumptions: vec![],
        addresses: vec![FindingInput {
            rule: "cite.uncited-clause".into(),
            subject: Some(SubjectInput::Clause(ClauseRef {
                context: Some("core".into()),
                contract: "rules".into(),
                label: "C2".into(),
            })),
        }],
    })
    .await
    .unwrap();
    app.open_change_request(&ChangeRequestInput {
        title: "reword C2".into(),
        situation: Some("it reads as two rules".into()),
        intent: Intent::ChangeBehavior,
        parent: None,
        issuer: Some("alice".into()),
        targets: vec![TargetInput::Clause(ClauseRef {
            context: Some("core".into()),
            contract: "rules".into(),
            label: "C2".into(),
        })],
        keep: vec![],
        assumptions: vec![],
        addresses: vec![],
    })
    .await
    .unwrap();
}

/// What a program is sent on its standard input, and the two stamps kept with its report, for a rule that reads
/// `words`.
async fn sent_to(name: &str, words: &[&str]) -> (String, String, String) {
    let executor = FakeExecutor::default();
    let mut app = app_with_reads(
        name,
        vec![Moment::Check],
        false,
        executor.clone(),
        reads(words),
    )
    .await;
    populate(&mut app).await;
    app.check_run().await.unwrap();
    let call = executor.calls.lock().unwrap()[0].clone();
    let report = app.store().external_reports().await.unwrap().remove(0);
    (
        String::from_utf8(serde_json::to_vec(&call).unwrap()).unwrap(),
        report.input_stamp,
        report.config_stamp,
    )
}

/// The standard input of a program that reads every section, listed out of order and with one twice. A saved
/// `input_stamp` is compared with this document's digest, so changing one byte of it makes every saved external
/// run stale: these values were taken before the model was typed and are what it must keep writing.
const SENT_ALL: &str = r#"{"protocol":2,"moment":"check","rules":["ext.contract"],"read_at":9,"params":{},"model":{"version":4,"contexts":[{"basis":"interface","contracts":[{"clauses":[{"id":"00000000-0000-0000-0000-000000000005","label":"C1","revision":1,"text":"push then pop returns the value"},{"id":"00000000-0000-0000-0000-000000000006","label":"C2","revision":1,"text":"pop on empty is an error"}],"id":"00000000-0000-0000-0000-000000000004","name":"rules","revision":1,"title":"Rules"}],"harnesses":[{"cases":[{"cites":{"clauses":[{"id":"00000000-0000-0000-0000-000000000005","revision":1}],"items":[{"id":"00000000-0000-0000-0000-00000000000a","revision":1}]},"id":"00000000-0000-0000-0000-00000000000d","lines":[1,1],"name":"pushes","revision":1}],"id":"00000000-0000-0000-0000-00000000000c","name":"tests","revision":1,"source":"fn pushes() {}\n"}],"id":"00000000-0000-0000-0000-000000000002","interfaces":[{"id":"00000000-0000-0000-0000-000000000009","items":[{"id":"00000000-0000-0000-0000-00000000000a","key":"push","lines":[1,1],"revision":1},{"id":"00000000-0000-0000-0000-00000000000b","key":"pop","lines":[2,2],"revision":1}],"name":"stack","revision":1,"source":"fn push()\nfn pop()\n"}],"name":"core","uses":[]},{"basis":"interface","contracts":[{"clauses":[{"id":"00000000-0000-0000-0000-000000000008","label":"C1","revision":1,"text":"a \"quoted\" <word> & more"}],"id":"00000000-0000-0000-0000-000000000007","name":"misc","revision":1,"title":"Misc"}],"harnesses":[],"id":"00000000-0000-0000-0000-000000000003","interfaces":[],"name":"util","uses":["core"]}],"change_requests":[{"addresses":[{"rule":"cite.uncited-clause","subject":{"id":"00000000-0000-0000-0000-000000000006","kind":"clause"}}],"expects":[{"id":"00000000-0000-0000-0000-00000000000a","kind":"keep","recorded":1,"target":"item"}],"id":"00000000-0000-0000-0000-00000000000e","intent":"keep_behavior","state":"open","title":"keep push"},{"addresses":[],"expects":[{"id":"00000000-0000-0000-0000-000000000006","kind":"target","recorded":1,"target":"clause"}],"id":"00000000-0000-0000-0000-00000000000f","intent":"change_behavior","state":"open","title":"reword C2"}]}}"#;

/// The standard input of a program that reads the contracts only: no other section, and no `change_requests`.
const SENT_CONTRACTS: &str = r#"{"protocol":2,"moment":"check","rules":["ext.contract"],"read_at":9,"params":{},"model":{"version":4,"contexts":[{"basis":"interface","contracts":[{"clauses":[{"id":"00000000-0000-0000-0000-000000000005","label":"C1","revision":1,"text":"push then pop returns the value"},{"id":"00000000-0000-0000-0000-000000000006","label":"C2","revision":1,"text":"pop on empty is an error"}],"id":"00000000-0000-0000-0000-000000000004","name":"rules","revision":1,"title":"Rules"}],"id":"00000000-0000-0000-0000-000000000002","name":"core","uses":[]},{"basis":"interface","contracts":[{"clauses":[{"id":"00000000-0000-0000-0000-000000000008","label":"C1","revision":1,"text":"a \"quoted\" <word> & more"}],"id":"00000000-0000-0000-0000-000000000007","name":"misc","revision":1,"title":"Misc"}],"id":"00000000-0000-0000-0000-000000000003","name":"util","uses":["core"]}]}}"#;

/// The whole view a judged rule is given (`check_rules`) for the same model: the declared order of the fields, every
/// section of every Context.
const WHOLE_VIEW: &str = r#"{"version":4,"store_revision":9,"contexts":[{"id":"00000000-0000-0000-0000-000000000002","name":"core","basis":"interface","uses":[],"contracts":[{"id":"00000000-0000-0000-0000-000000000004","name":"rules","title":"Rules","revision":1,"clauses":[{"id":"00000000-0000-0000-0000-000000000005","label":"C1","text":"push then pop returns the value","revision":1},{"id":"00000000-0000-0000-0000-000000000006","label":"C2","text":"pop on empty is an error","revision":1}]}],"interfaces":[{"id":"00000000-0000-0000-0000-000000000009","name":"stack","revision":1,"source":"fn push()\nfn pop()\n","items":[{"id":"00000000-0000-0000-0000-00000000000a","key":"push","lines":[1,1],"revision":1},{"id":"00000000-0000-0000-0000-00000000000b","key":"pop","lines":[2,2],"revision":1}]}],"harnesses":[{"id":"00000000-0000-0000-0000-00000000000c","name":"tests","revision":1,"source":"fn pushes() {}\n","cases":[{"id":"00000000-0000-0000-0000-00000000000d","name":"pushes","lines":[1,1],"revision":1,"cites":{"items":[{"id":"00000000-0000-0000-0000-00000000000a","revision":1}],"clauses":[{"id":"00000000-0000-0000-0000-000000000005","revision":1}]}}]}]},{"id":"00000000-0000-0000-0000-000000000003","name":"util","basis":"interface","uses":["core"],"contracts":[{"id":"00000000-0000-0000-0000-000000000007","name":"misc","title":"Misc","revision":1,"clauses":[{"id":"00000000-0000-0000-0000-000000000008","label":"C1","text":"a \"quoted\" <word> & more","revision":1}]}],"interfaces":[],"harnesses":[]}],"change_requests":[{"id":"00000000-0000-0000-0000-00000000000e","title":"keep push","intent":"keep_behavior","state":"open","expects":[{"target":"item","id":"00000000-0000-0000-0000-00000000000a","kind":"keep","recorded":1}],"addresses":[{"rule":"cite.uncited-clause","subject":{"kind":"clause","id":"00000000-0000-0000-0000-000000000006"}}]},{"id":"00000000-0000-0000-0000-00000000000f","title":"reword C2","intent":"change_behavior","state":"open","expects":[{"target":"clause","id":"00000000-0000-0000-0000-000000000006","kind":"target","recorded":1}],"addresses":[]}]}"#;

#[tokio::test]
async fn the_view_a_judged_rule_is_given_is_the_whole_model_in_declared_order() {
    let mut app = app_with_reads(
        "pinned-view",
        vec![Moment::Check],
        false,
        FakeExecutor::default(),
        reads(&["contracts"]),
    )
    .await;
    populate(&mut app).await;
    let view = app.check_rules(None, None).await.unwrap();
    assert_eq!(serde_json::to_string(&view.model).unwrap(), WHOLE_VIEW);
}

#[tokio::test]
async fn what_a_program_is_sent_and_the_stamps_kept_are_what_they_always_were() {
    let (sent, input, config) = sent_to(
        "pinned-all",
        &[
            "interfaces",
            "contracts",
            "interfaces",
            "harnesses",
            "change_requests",
        ],
    )
    .await;
    assert_eq!(sent, SENT_ALL);
    assert_eq!(
        input,
        "02ed0bbdc5de2565fe7799f871f5c164b28dc525be12ea6373363ba050b7f62b"
    );
    assert_eq!(
        config,
        "4566f047cd0525e39c0c46f13779f46359eb367bf1ac7b94908a3a3cdfe91fa5"
    );

    let (sent, input, config) = sent_to("pinned-contracts", &["contracts"]).await;
    assert_eq!(sent, SENT_CONTRACTS);
    assert_eq!(
        input,
        "ac81f2e40a9ac2f509729a5458f9efaa5af18c7ad4efd70cc196256ec5fa4375"
    );
    assert_eq!(
        config,
        "213868a14489d87ad3161e0c80eb9307a3ede852fc99e75d01ac97b11f4b1bb3"
    );
}

/// A program that reads the `cruxes` is sent them, after the change requests and with the keys in the order of their
/// names, and the stamp kept with its report moves when a Crux is decided: the report is stale then.
#[tokio::test]
async fn a_program_that_reads_the_cruxes_is_sent_them_and_their_decision_makes_its_report_stale() {
    use chidev_core::app::{
        BasisInput, CruxKind, DecisionInput, OptionInput, RaiseCruxInput, RecommendationInput,
    };

    let executor = FakeExecutor::default();
    let mut app = app_with_reads(
        "reads-cruxes",
        vec![Moment::Check],
        false,
        executor.clone(),
        reads(&["change_requests", "cruxes"]),
    )
    .await;
    populate(&mut app).await;
    let raised = app
        .raise_crux(&RaiseCruxInput {
            cr: "reword C2".into(),
            kind: CruxKind::Ambiguity,
            question: "Is an empty pop an error?".into(),
            findings: vec![],
            options: vec![OptionInput {
                choice: "yes".into(),
                change: None,
                targets: vec![],
                keep: vec![],
                consequence: None,
                recommended: Some(RecommendationInput {
                    reason: None,
                    confidence: None,
                }),
            }],
            importance: None,
            supersedes: None,
        })
        .await
        .unwrap()
        .value;
    app.check_run().await.unwrap();
    let sent = |executor: &FakeExecutor, n: usize| -> serde_json::Value {
        serde_json::to_value(&executor.calls.lock().unwrap()[n]).unwrap()
    };
    let first = sent(&executor, 0);
    let model = first["model"].as_object().unwrap();
    // `cruxes` follows `change_requests`; no asset section was read
    let keys: Vec<&str> = model.keys().map(String::as_str).collect();
    assert_eq!(keys, ["change_requests", "contexts", "cruxes", "version"]);
    assert!(model["contexts"][0].get("contracts").is_none());
    let crux = &model["cruxes"][0];
    assert_eq!(crux["id"], raised.id.to_string().as_str());
    assert_eq!(crux["state"], "open");
    assert_eq!(crux["raised_in"], model["change_requests"][1]["id"]);
    assert_eq!(
        crux["recommended"]["option"],
        raised.options[0].id.to_string().as_str()
    );
    let before = app.store().external_reports().await.unwrap().remove(0);

    app.decide_crux(
        &raised.id.to_string(),
        &DecisionInput::Chosen {
            option: raised.options[0].id.to_string(),
            reason: None,
        },
        None,
        &BasisInput::Arbitrary("settled".into()),
        &DecisionMakerInput::Person("alice".into()),
    )
    .await
    .unwrap();
    app.check_run().await.unwrap();
    let second = sent(&executor, 1);
    assert_eq!(second["model"]["cruxes"][0]["state"], "answered");
    let after = app.store().external_reports().await.unwrap().remove(0);
    assert_ne!(before.input_stamp, after.input_stamp);
    assert_eq!(before.config_stamp, after.config_stamp);
}

#[tokio::test]
async fn adding_a_context_setting_its_uses_and_basing_it_on_its_contracts_are_edits_a_gate_sees() {
    let executor = FakeExecutor::default();
    let mut app = app("gate-context", vec![Moment::Edit], true, executor.clone()).await;
    app.put_contract("core", "stack", "Stack", &clauses("push then pop"))
        .await
        .unwrap();
    app.add_context("util", &[]).await.unwrap();
    let asked = executor.count();
    let revision = app.store().revision().await.unwrap();
    let project = app.store().project().await.unwrap();

    let say = |message: &str| {
        executor
            .replies
            .lock()
            .unwrap()
            .push_back(reply(vec![finding(message)]));
    };
    let refused = |result: Result<_, AppError>| match result {
        Err(AppError::ExternalRefused { findings, .. }) => assert_eq!(findings.len(), 1),
        other => panic!("expected the gate to refuse, got {other:?}"),
    };
    say("a context too many");
    refused(app.add_context("billing", &[]).await.map(|_| ()));
    say("the uses are wrong");
    refused(
        app.set_uses("core", &["util".to_string()])
            .await
            .map(|_| ()),
    );
    say("not on its contracts");
    refused(app.base_on_contract("core").await.map(|_| ()));
    // Each was asked, and none kept anything.
    assert_eq!(executor.count(), asked + 3);
    assert_eq!(app.store().revision().await.unwrap(), revision);
    assert_eq!(app.store().project().await.unwrap(), project);

    // When the program has nothing to say they are kept, and the answer carries what the checks said (nothing).
    let added = app.add_context("billing", &[]).await.unwrap();
    assert!(added.findings.is_empty());
    assert!(
        app.set_uses("core", &["util".to_string()])
            .await
            .unwrap()
            .findings
            .is_empty()
    );
    assert!(
        app.base_on_contract("core")
            .await
            .unwrap()
            .findings
            .is_empty()
    );
}

/// The digests the first version made: every word of a registration that was only renamed counts as it was.
mod renamed_words {
    use super::*;

    /// The configuration stamp (SHA-256 of the JSON of what the rule is configured with) of the rule
    /// `rule(vec![Edit, Check], true, "config-v1")` with no parameters, as version 0.1.0 made it: its moments
    /// counted as `put` and `check` under the key `hooks`.
    const STAMP_MADE_BY_0_1_0: &str =
        "f145fde5e08d11b1fbd75467e29d06c051fc6728df5db2800a3eac4dc0971bc7";

    #[tokio::test]
    async fn the_word_of_an_edit_is_not_part_of_what_makes_a_saved_result_stale() {
        let executor = FakeExecutor::default();
        let mut app = app(
            "stamp-0-1-0",
            vec![Moment::Edit, Moment::Check],
            true,
            executor.clone(),
        )
        .await;
        app.put_contract("core", "stack", "Stack", &clauses("push then pop"))
            .await
            .unwrap();
        let first = finding("the contract contradicts itself");
        let second = finding("the project has no glossary");
        executor
            .replies
            .lock()
            .unwrap()
            .push_back(reply(vec![first.clone(), second]));
        app.check_run().await.unwrap();
        let saved = app.store().external_reports().await.unwrap();
        // What this version saves is what the first version saved for the same registration.
        assert_eq!(saved[0].config_stamp, STAMP_MADE_BY_0_1_0);
        assert_eq!(saved[0].findings.len(), 2);

        // So the result describes the model, and an edit that mends one of the two findings is not refused for
        // the one that was there.
        executor
            .replies
            .lock()
            .unwrap()
            .push_back(reply(vec![first.clone()]));
        let kept = app
            .put_contract("core", "stack", "Stack", &clauses("push, then pop"))
            .await
            .unwrap();
        assert!(kept.findings.is_empty(), "{:?}", kept.findings);
        assert!(
            app.store().external_reports().await.unwrap()[0].config_stamp == STAMP_MADE_BY_0_1_0
        );

        // A change of anything the registration says is another matter: the result is stale, and every finding
        // counts again.
        let store = app.into_store();
        let registry = Registry::with_external(
            &[],
            vec![],
            vec![rule(vec![Moment::Edit, Moment::Check], true, "config-v2")],
            &[],
        )
        .unwrap();
        let mut changed = App::with_registry(
            store,
            Counter(1_000),
            registry,
            FixedClock(Timestamp::from_millis(1_000)),
        )
        .with_external(executor.clone());
        executor
            .replies
            .lock()
            .unwrap()
            .push_back(reply(vec![first]));
        assert!(matches!(
            changed
                .put_contract("core", "stack", "Stack", &clauses("push, then pop, once"))
                .await,
            Err(AppError::ExternalRefused { .. })
        ));
    }
}
