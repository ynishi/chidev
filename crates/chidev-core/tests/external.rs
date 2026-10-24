//! External checks use a fake process port here so these tests cover application policy and store boundaries.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chidev_core::app::ports::external::{
    ExecutionEvidence, ExternalAttempt, ExternalCall, ExternalExecutor, ExternalFailure,
    ExternalFinding, ExternalReply, ExternalSubject,
};
use chidev_core::app::ports::{Changes, IdSource, Store, StoreRevision};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, DecisionMakerInput, Intent,
    ItemInput, ItemRef, Registry, ResolutionInput,
};
use chidev_core::domain::check::external::ExternalRule;
use chidev_core::domain::check::{Executor, Hook, Level, Origin};
use chidev_core::domain::repository::{
    ChangeRequestRepository, PinRepository, ProjectRepository, StoreError,
};
use chidev_core::domain::vo::finding::RuleId;
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
            let other_writer = SqliteStore::open(path).await.unwrap();
            other_writer
                .commit(StoreRevision(call.read_at), Changes::default())
                .await
                .unwrap();
            other_writer.close().await.unwrap();
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

fn rule(hooks: Vec<Hook>, gate: bool, config: &str) -> ExternalRule {
    ExternalRule::new(
        RuleId::from_static("ext.contract"),
        hooks,
        vec!["contracts".into()],
        gate,
        config.into(),
    )
    .unwrap()
}

async fn app(
    name: &str,
    hooks: Vec<Hook>,
    gate: bool,
    executor: FakeExecutor,
) -> App<SqliteStore, Counter, FakeExecutor> {
    app_with_reads(name, hooks, gate, executor, vec!["contracts".into()]).await
}

async fn app_with_reads(
    name: &str,
    hooks: Vec<Hook>,
    gate: bool,
    executor: FakeExecutor,
    reads: Vec<String>,
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
    let registry = Registry::with_external(&[], vec![], vec![external], &[]).unwrap();
    let mut app = App::with_registry(
        store,
        Counter::default(),
        registry,
        FixedClock(Timestamp::from_millis(1_000)),
    )
    .with_external(executor);
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app
}

fn clauses(text: &str) -> Vec<ClauseInput> {
    vec![ClauseInput {
        label: "C1".into(),
        text: text.into(),
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
    let mut app = app("advisory-failure", vec![Hook::Put], false, executor.clone()).await;
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
    let invocations = app.store().invocations().await.unwrap();
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
    let mut app = app("gate-failure", vec![Hook::Put], true, executor).await;
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
        vec![Hook::Put],
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
        vec![Hook::Check],
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
    let mut app = app("explicit-run", vec![Hook::Check], false, executor.clone()).await;
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "unrun"
    );
    assert!(app.check().await.unwrap().is_empty());
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
    assert!(app.check().await.unwrap().is_empty());
    assert_eq!(executor.count(), 1);
}

#[tokio::test]
async fn only_requested_assets_are_sent_and_a_changed_input_makes_the_report_stale() {
    let executor = FakeExecutor::default();
    let mut app = app(
        "reads-and-stale",
        vec![Hook::Check],
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
            signature: "fn push()".into(),
            lines: (1, 1),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    app.check_run().await.unwrap();
    let call = executor.calls.lock().unwrap()[0].clone();
    assert_eq!(call.protocol, 1);
    assert_eq!(call.hook, "check");
    assert_eq!(call.rules, vec!["ext.contract"]);
    assert!(call.model.change_requests.is_none());
    let context = &call.model.contexts[0];
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
            signature: "fn push(value: u32)".into(),
            lines: (1, 1),
            replaces: Some("fn push()".into()),
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
                protocol: 2,
                findings: vec![],
            }),
        ),
    ] {
        let mut app = app(
            name,
            vec![Hook::Check],
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
async fn a_subject_that_exists_but_was_not_sent_is_refused() {
    let executor = FakeExecutor::default();
    let mut app = app("hidden-subject", vec![Hook::Check], false, executor.clone()).await;
    let interface = app
        .put_interface(
            "core",
            "api",
            "fn push()",
            &[ItemInput {
                signature: "fn push()".into(),
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
    let mut app = app("valid-subject", vec![Hook::Check], false, executor.clone()).await;
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
    assert_eq!(app.check().await.unwrap().len(), 1);
    let checked = app.check_run().await.unwrap();
    assert!(checked.findings.is_empty());
    assert!(app.check().await.unwrap().is_empty());
    let reports = app.store().external_reports().await.unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].findings.is_empty());
    assert_eq!(app.store().invocations().await.unwrap().len(), 2);
}

#[tokio::test]
async fn explicit_check_records_a_failed_gate_without_changing_the_model() {
    let mut app = app(
        "explicit-gate",
        vec![Hook::Check],
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
}

#[tokio::test]
async fn changing_the_registered_process_configuration_invalidates_its_previous_result() {
    let executor = FakeExecutor::default();
    let mut app = app("config-stale", vec![Hook::Check], false, executor.clone()).await;
    app.check_run().await.unwrap();
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current"
    );
    let store = app.into_store();
    let registry = Registry::with_external(
        &[],
        vec![],
        vec![rule(vec![Hook::Check], false, "config-v2")],
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
        vec![rule(vec![Hook::Check], true, "config-v1")],
        std::slice::from_ref(&layer),
    );
    assert!(registry.is_err());

    let executor = FakeExecutor::default();
    let app = app("policy-off", vec![Hook::Check], false, executor.clone()).await;
    let registry = Registry::with_external(
        &[],
        vec![],
        vec![rule(vec![Hook::Check], false, "config-v1")],
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
    let mut app = app(name, vec![Hook::Check], false, executor.clone()).await;
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
    let mut app = app("pin-hook", vec![Hook::Pin], true, executor.clone()).await;
    let interface = app
        .put_interface(
            "core",
            "api",
            "fn push()",
            &[ItemInput {
                signature: "fn push()".into(),
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
                signature: "fn push()".into(),
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
    let invocations = app.store().invocations().await.unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].hook, Hook::Pin);
    assert_eq!(invocations[0].read_at, revision);
    assert_eq!(executor.count(), 2);
    assert!(
        executor
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call.hook == "pin")
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
        vec![Hook::Resolve],
        true,
        executor.clone(),
        vec!["change_requests".into()],
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
            crux: None,
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
    assert_eq!(call.hook, "resolve");
    let candidate = &call.model.change_requests.as_ref().unwrap()[0];
    assert_eq!(candidate["id"], cr.id.to_string());
    assert_eq!(candidate["state"], "resolved");

    app.resolve_change_request("accept current behavior", &how, "settled", &by)
        .await
        .unwrap();
    assert_eq!(
        app.store().change_requests().await.unwrap()[0]
            .state()
            .word(),
        "resolved"
    );
    assert_eq!(app.store().revision().await.unwrap().0, revision.0 + 1);
    assert_eq!(app.store().external_reports().await.unwrap().len(), 1);
    let invocations = app.store().invocations().await.unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].hook, Hook::Resolve);
    assert_eq!(invocations[0].read_at, revision);
    assert_eq!(executor.count(), 2);
    assert_eq!(
        app.status().await.unwrap().external[0].state.word(),
        "current"
    );
}
