//! Change requests address findings by rule and typed subject, independently of the executor.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chidev_core::app::ports::external::{
    ExecutionEvidence, ExternalAttempt, ExternalCall, ExternalExecutor, ExternalFinding,
    ExternalReply, ExternalSubject,
};
use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, BasisInput, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, CruxInput,
    CruxKind, DecisionInput, DecisionMakerInput, FindingInput, Intent, ItemInput, ItemRef,
    OptionInput, ReasonedInput, Registry, ReportInput, ReportedFinding, ResolutionInput,
    SubjectInput,
};
use chidev_core::domain::check::external::ExternalRule;
use chidev_core::domain::check::{FindingState, Moment};
use chidev_core::domain::repository::{ChangeRequestRepository, ProjectRepository};
use chidev_core::domain::vo::finding::{RuleId, SubjectKind};
use chidev_core::domain::vo::model_section::ModelSection;
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::{judged, sqlite::SqliteStore};
use uuid::Uuid;

#[derive(Default)]
struct Counter(u128);

impl IdSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_u128(self.0)
    }
}

fn fresh_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "chidev-addresses-{}-{}/state.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn request(title: &str, addresses: Vec<FindingInput>) -> ChangeRequestInput {
    ChangeRequestInput {
        title: title.into(),
        situation: None,
        intent: Intent::KeepBehavior,
        parent: None,
        issuer: None,
        targets: vec![],
        keep: vec![],
        crux: None,
        assumptions: vec![],
        addresses,
    }
}

/// Resolves the change request titled `title` as done, by a person.
async fn resolve_done<S: Store, G: IdSource, E: ExternalExecutor>(
    app: &mut App<S, G, E>,
    title: &str,
) {
    app.resolve_change_request(
        title,
        &ResolutionInput::Done,
        "settled",
        &DecisionMakerInput::Person("alice".to_string()),
    )
    .await
    .unwrap();
}

fn item() -> ItemRef {
    ItemRef {
        context: Some("core".into()),
        interface: "stack".into(),
        key: "push".into(),
    }
}

fn clause() -> ClauseRef {
    ClauseRef {
        context: Some("core".into()),
        contract: "rules".into(),
        label: "C1".into(),
    }
}

fn subjects() -> Vec<(&'static str, Option<SubjectInput>)> {
    vec![
        ("context", Some(SubjectInput::Context("core".into()))),
        (
            "interface",
            Some(SubjectInput::Interface {
                context: "core".into(),
                interface: "stack".into(),
            }),
        ),
        ("item", Some(SubjectInput::Item(item()))),
        ("clause", Some(SubjectInput::Clause(clause()))),
        (
            "case",
            Some(SubjectInput::Case {
                context: "core".into(),
                harness: "tests".into(),
                name: "pushes".into(),
            }),
        ),
        (
            "change_request",
            Some(SubjectInput::ChangeRequest("target".into())),
        ),
        ("project", None),
    ]
}

fn judged_registry() -> Registry {
    let rules = subjects().into_iter().map(|(kind, _)| {
        judged::parse(&format!(
            "id='judged.{kind}'\nsummary='review'\nsubject='{kind}'\nmoments=['check']\nlevel='warning'\nquestion='q'\ntrue='t'\nfalse='f'\nfix='x'\n"
        )).unwrap()
    }).collect();
    Registry::builtin_with_judged(rules, &[]).unwrap()
}

async fn fixture(registry: Registry) -> (App<SqliteStore, Counter>, PathBuf) {
    let path = fresh_path();
    let store = SqliteStore::open(&path).await.unwrap();
    let mut app = App::with_registry(
        store,
        Counter::default(),
        registry,
        FixedClock(Timestamp::from_millis(1_000)),
    );
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract(
        "core",
        "rules",
        "Rules",
        &[ClauseInput {
            label: "C1".into(),
            text: "push stores an item".into(),
        }],
    )
    .await
    .unwrap();
    app.put_interface(
        "core",
        "stack",
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
        "tests",
        "pushes",
        &[CaseInput {
            name: "pushes".into(),
            lines: (1, 1),
            items: vec![item()],
            clauses: vec![clause()],
        }],
    )
    .await
    .unwrap();
    app.open_change_request(&request("target", vec![]))
        .await
        .unwrap();
    (app, path)
}

async fn report_every_subject(app: &mut App<SqliteStore, Counter>) {
    for (kind, subject) in subjects() {
        let at = app.store().revision().await.unwrap().0;
        let rule = format!("judged.{kind}");
        let rule_digest = app.check_rules(None, Some(&rule)).await.unwrap().rules[0]
            .rule_digest
            .to_string();
        app.finding_report(&ReportInput {
            rule,
            rule_digest,
            judge: "reviewer".into(),
            read_at: at,
            findings: vec![ReportedFinding {
                subject,
                message: "requires review".into(),
                level: None,
            }],
        })
        .await
        .unwrap();
    }
}

async fn assert_judged_states(app: &App<SqliteStore, Counter>, state: FindingState) {
    let findings = app.check(None).await.unwrap().findings;
    let judged: Vec<_> = findings
        .iter()
        .filter(|f| f.rule.as_str().starts_with("judged."))
        .collect();
    assert_eq!(judged.len(), subjects().len());
    for f in judged {
        assert_eq!(f.state, state, "{}", f.rule);
    }
}

#[tokio::test]
async fn every_subject_is_addressed_deduplicated_persisted_and_resolved() {
    let (mut app, path) = fixture(judged_registry()).await;
    report_every_subject(&mut app).await;
    assert_judged_states(&app, FindingState::Open).await;
    let mut addresses: Vec<_> = subjects()
        .into_iter()
        .map(|(kind, subject)| FindingInput {
            rule: format!("judged.{kind}"),
            subject,
        })
        .collect();
    addresses.push(addresses[0].clone());
    app.open_change_request(&request("review findings", addresses))
        .await
        .unwrap();
    assert_judged_states(&app, FindingState::Addressed).await;
    let crs = app.store().change_requests().await.unwrap();
    let saved = crs
        .iter()
        .find(|cr| cr.title().as_str() == "review findings")
        .unwrap()
        .clone();
    assert_eq!(saved.addresses().len(), subjects().len());
    assert_eq!(
        saved
            .addresses()
            .iter()
            .map(|a| SubjectKind::of(a.subject()))
            .collect::<BTreeSet<_>>(),
        SubjectKind::ALL.into_iter().collect()
    );

    app.into_store().close().await.unwrap();
    let mut app = App::with_registry(
        SqliteStore::open(&path).await.unwrap(),
        Counter(1000),
        judged_registry(),
        FixedClock(Timestamp::from_millis(1_000)),
    );
    let restored = app.store().change_requests().await.unwrap();
    assert_eq!(
        restored.iter().find(|cr| cr.id() == saved.id()),
        Some(&saved)
    );
    assert_judged_states(&app, FindingState::Addressed).await;
    resolve_done(&mut app, "review findings").await;
    assert_judged_states(&app, FindingState::Decided).await;
    // A further unresolved request takes precedence over an older resolved one.
    app.open_change_request(&request(
        "review context again",
        vec![FindingInput {
            rule: "judged.context".into(),
            subject: Some(SubjectInput::Context("core".into())),
        }],
    ))
    .await
    .unwrap();
    let found = app.check(None).await.unwrap().findings;
    assert_eq!(
        found
            .iter()
            .find(|f| f.rule.as_str() == "judged.context")
            .unwrap()
            .state,
        FindingState::Addressed
    );
    assert_eq!(
        found
            .iter()
            .find(|f| f.rule.as_str() == "judged.case")
            .unwrap()
            .state,
        FindingState::Decided
    );
}

#[tokio::test]
async fn nonexistent_targets_and_invalid_rule_ids_keep_nothing() {
    let (mut app, _) = fixture(Registry::defaults()).await;
    let invalid = vec![
        Some(SubjectInput::Context("missing".into())),
        Some(SubjectInput::Interface {
            context: "core".into(),
            interface: "missing".into(),
        }),
        Some(SubjectInput::Item(ItemRef {
            key: "missing".into(),
            ..item()
        })),
        Some(SubjectInput::Clause(ClauseRef {
            label: "C9".into(),
            ..clause()
        })),
        Some(SubjectInput::Case {
            context: "core".into(),
            harness: "tests".into(),
            name: "missing".into(),
        }),
        Some(SubjectInput::ChangeRequest("missing".into())),
    ];
    let before = app.store().revision().await.unwrap();
    let crs = app.store().change_requests().await.unwrap();
    for subject in invalid {
        let failed = app
            .open_change_request(&request(
                "bad address",
                vec![FindingInput {
                    rule: "retired.rule".into(),
                    subject,
                }],
            ))
            .await;
        assert!(failed.is_err(), "{failed:?}");
        assert_eq!(app.store().revision().await.unwrap(), before);
        assert_eq!(app.store().change_requests().await.unwrap(), crs);
    }
    let failed = app
        .open_change_request(&request(
            "bad rule",
            vec![FindingInput {
                rule: "Not A Rule".into(),
                subject: None,
            }],
        ))
        .await;
    assert!(matches!(failed, Err(AppError::Input(_))));
    assert_eq!(app.store().revision().await.unwrap(), before);

    // Registration and a currently materialized finding are not prerequisites.
    app.open_change_request(&request(
        "historical finding",
        vec![FindingInput {
            rule: "retired.rule".into(),
            subject: Some(SubjectInput::Context("core".into())),
        }],
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn a_case_name_that_does_not_identify_one_case_is_refused_atomically() {
    let (mut app, _) = fixture(Registry::defaults()).await;
    let case = |lines| CaseInput {
        name: "same".into(),
        lines,
        items: vec![item()],
        clauses: vec![],
    };
    app.put_harness(
        "core",
        "ambiguous",
        "one\ntwo",
        &[case((1, 1)), case((2, 2))],
    )
    .await
    .unwrap();
    let before = app.store().revision().await.unwrap();
    let crs = app.store().change_requests().await.unwrap();
    let result = app
        .open_change_request(&request(
            "ambiguous target",
            vec![FindingInput {
                rule: "retired.rule".into(),
                subject: Some(SubjectInput::Case {
                    context: "core".into(),
                    harness: "ambiguous".into(),
                    name: "same".into(),
                }),
            }],
        ))
        .await;
    assert!(
        matches!(result, Err(AppError::Input(ref message)) if message.contains("several cases"))
    );
    assert_eq!(app.store().revision().await.unwrap(), before);
    assert_eq!(app.store().change_requests().await.unwrap(), crs);
}

#[tokio::test]
async fn deciding_a_crux_does_not_resolve_the_addressed_finding() {
    let (mut app, _) = fixture(judged_registry()).await;
    report_every_subject(&mut app).await;
    let mut review = request(
        "human review",
        vec![FindingInput {
            rule: "judged.context".into(),
            subject: Some(SubjectInput::Context("core".into())),
        }],
    );
    review.crux = Some(CruxInput {
        kind: CruxKind::Ambiguity,
        options: vec![OptionInput {
            choice: "keep".into(),
            change: None,
            targets: vec![],
            keep: vec![],
            consequence: None,
        }],
        recommended: None,
        importance: None,
        supersedes: None,
    });
    app.open_change_request(&review).await.unwrap();
    app.decide_change_request(
        "human review",
        &DecisionInput::Chosen(ReasonedInput {
            choice: "keep".into(),
            reason: None,
        }),
        &BasisInput::Arbitrary("nothing derives it".into()),
        &DecisionMakerInput::Person("alice".into()),
    )
    .await
    .unwrap();
    let findings = app.check(None).await.unwrap().findings;
    let context = findings
        .iter()
        .find(|f| f.rule.as_str() == "judged.context")
        .unwrap();
    assert_eq!(context.state, FindingState::Addressed);
    let project = findings
        .iter()
        .find(|f| f.rule.as_str() == "judged.project")
        .unwrap();
    assert!(
        project.stale,
        "a changed model independently stales the project judgment"
    );
    assert_eq!(project.state, FindingState::Open);
    resolve_done(&mut app, "human review").await;
    assert_eq!(
        app.check(None)
            .await
            .unwrap()
            .findings
            .iter()
            .find(|f| f.rule.as_str() == "judged.context")
            .unwrap()
            .state,
        FindingState::Decided
    );
}

#[tokio::test]
async fn builtin_context_findings_use_the_same_subject_reference() {
    let (mut app, _) = fixture(Registry::defaults()).await;
    app.add_context("empty", &[]).await.unwrap();
    let rule = "context.no-interface";
    let state = |fs: Vec<chidev_core::app::Found>| {
        fs.into_iter()
            .find(|f| f.rule.as_str() == rule)
            .unwrap()
            .state
    };
    assert_eq!(
        state(app.check(None).await.unwrap().findings),
        FindingState::Open
    );
    app.open_change_request(&request(
        "keep empty",
        vec![FindingInput {
            rule: rule.into(),
            subject: Some(SubjectInput::Context("empty".into())),
        }],
    ))
    .await
    .unwrap();
    assert_eq!(
        state(app.check(None).await.unwrap().findings),
        FindingState::Addressed
    );
    resolve_done(&mut app, "keep empty").await;
    assert_eq!(
        state(app.check(None).await.unwrap().findings),
        FindingState::Decided
    );
}

#[derive(Debug)]
struct CaseExecutor;

impl ExternalExecutor for CaseExecutor {
    async fn execute(&self, rule: &RuleId, call: ExternalCall) -> ExternalAttempt {
        let model = serde_json::to_value(&call.model).unwrap();
        let case = &model["contexts"][0]["harnesses"][0]["cases"][0];
        ExternalAttempt {
            evidence: ExecutionEvidence::default(),
            result: Ok(ExternalReply {
                protocol: 1,
                findings: vec![ExternalFinding {
                    rule: rule.to_string(),
                    subject: Some(ExternalSubject {
                        kind: "case".into(),
                        id: case["id"].as_str().unwrap().into(),
                    }),
                    message: "review the case".into(),
                    level: None,
                }],
            }),
        }
    }
}

#[tokio::test]
async fn an_external_case_finding_is_addressed_without_changing_its_staleness() {
    let rule = ExternalRule::new(
        RuleId::from_static("ext.case"),
        vec![Moment::Check],
        vec![ModelSection::Harnesses],
        false,
        "config".into(),
    )
    .unwrap();
    let (app, _) = fixture(Registry::with_external(&[], vec![], vec![rule], &[]).unwrap()).await;
    let mut app = app.with_external(CaseExecutor);
    app.check_run().await.unwrap();
    let finding = app.check(None).await.unwrap().findings.pop().unwrap();
    assert_eq!(finding.state, FindingState::Open);
    assert!(!finding.stale);
    app.open_change_request(&request(
        "review test",
        vec![FindingInput {
            rule: "ext.case".into(),
            subject: Some(SubjectInput::Case {
                context: "core".into(),
                harness: "tests".into(),
                name: "pushes".into(),
            }),
        }],
    ))
    .await
    .unwrap();
    let finding = app.check(None).await.unwrap().findings.pop().unwrap();
    assert_eq!(finding.state, FindingState::Addressed);
    assert!(
        !finding.stale,
        "the external rule did not read change requests"
    );
    resolve_done(&mut app, "review test").await;
    let finding = app.check(None).await.unwrap().findings.pop().unwrap();
    assert_eq!(finding.state, FindingState::Decided);
    assert!(!finding.stale);
}

#[tokio::test]
async fn removing_a_case_preserves_its_historical_address() {
    let (mut app, path) = fixture(Registry::defaults()).await;
    app.open_change_request(&request(
        "historical case",
        vec![FindingInput {
            rule: "retired.case".into(),
            subject: Some(SubjectInput::Case {
                context: "core".into(),
                harness: "tests".into(),
                name: "pushes".into(),
            }),
        }],
    ))
    .await
    .unwrap();
    let before = app.store().change_requests().await.unwrap();
    app.put_harness("core", "tests", "", &[]).await.unwrap();
    assert_eq!(app.store().change_requests().await.unwrap(), before);
    let project = app.store().project().await.unwrap().unwrap();
    assert!(
        project
            .contexts()
            .next()
            .unwrap()
            .harnesses()
            .next()
            .unwrap()
            .cases()
            .is_empty()
    );
    app.into_store().close().await.unwrap();
    let store = SqliteStore::open(path).await.unwrap();
    assert_eq!(store.change_requests().await.unwrap(), before);
    store.close().await.unwrap();
}
