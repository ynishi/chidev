//! A judgment identifies the rule the agent actually received, independently of model revisions.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chidev_core::app::ports::{IdSource, JudgedRepository, Store};
use chidev_core::app::{
    App, ClauseInput, ClauseRef, JudgedState, Registry, ReportInput, ReportedFinding, SubjectInput,
};
use chidev_core::domain::check::{Executor, FindingState, Level, Origin};
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::{judged, policy, sqlite::SqliteStore};
use uuid::Uuid;

#[derive(Default)]
struct Counter(u128);

impl IdSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_u128(self.0)
    }
}

const RULE: &str = "judged.clause-review";

fn rule_text(question: &str) -> String {
    format!(
        "id='{RULE}'\nsummary='review a clause'\nsubject='clause'\nmoments=['check','edit']\nlevel='warning'\nquestion='{question}'\ntrue='no observable result'\nfalse='an observable result'\nfix='state the result'\nwhen=['should']\n"
    )
}

fn registry(text: &str, level: Option<&str>) -> Registry {
    let layers: Vec<_> = level
        .into_iter()
        .map(|level| {
            policy::parse(
                &format!("[checks.\"{RULE}\"]\nlevel='{level}'\n"),
                Origin::Person,
            )
            .unwrap()
        })
        .collect();
    Registry::builtin_with_judged(vec![judged::parse(text).unwrap()], &layers).unwrap()
}

async fn open(path: &Path, text: &str, level: Option<&str>) -> App<SqliteStore, Counter> {
    App::with_registry(
        SqliteStore::open(path).await.unwrap(),
        Counter(1000),
        registry(text, level),
        FixedClock(Timestamp::from_millis(1_000)),
    )
}

async fn fixture() -> (App<SqliteStore, Counter>, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "chidev-rule-version-{}-{}/state.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut app = App::with_registry(
        SqliteStore::open(&path).await.unwrap(),
        Counter::default(),
        registry(&rule_text("can this be tested?"), None),
        FixedClock(Timestamp::from_millis(1_000)),
    );
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    put_clauses(&mut app, "stores the item", "returns the item").await;
    (app, path)
}

async fn put_clauses(app: &mut App<SqliteStore, Counter>, first: &str, second: &str) {
    app.put_contract(
        "core",
        "rules",
        "Rules",
        &[
            ClauseInput {
                label: "C1".into(),
                text: first.into(),
                replaces: None,
            },
            ClauseInput {
                label: "C2".into(),
                text: second.into(),
                replaces: None,
            },
        ],
    )
    .await
    .unwrap();
}

async fn report(app: &App<SqliteStore, Counter>, labels: &[&str]) -> ReportInput {
    let view = app.check_rules(None, Some(RULE)).await.unwrap();
    ReportInput {
        rule: RULE.into(),
        rule_digest: view.rules[0].rule_digest.to_string(),
        judge: "reviewer".into(),
        read_at: view.store_revision.0,
        findings: labels
            .iter()
            .map(|label| ReportedFinding {
                subject: Some(SubjectInput::Clause(ClauseRef {
                    context: Some("core".into()),
                    contract: "rules".into(),
                    label: (*label).into(),
                })),
                message: format!("review {label}"),
                level: None,
            })
            .collect(),
    }
}

async fn finding_states(
    app: &App<SqliteStore, Counter>,
) -> Vec<(String, bool, FindingState, Level)> {
    app.check(None)
        .await
        .unwrap()
        .findings
        .into_iter()
        .filter(|f| f.rule.as_str() == RULE)
        .map(|f| (f.message, f.stale, f.state, f.level))
        .collect()
}

#[tokio::test]
async fn the_supplied_rule_version_is_saved_with_the_judgment_and_invocation() {
    let (mut app, path) = fixture().await;
    let input = report(&app, &["C1", "C2"]).await;
    let digest = app.check_rules(None, Some(RULE)).await.unwrap().rules[0].rule_digest;
    let model_at = app.store().model_revision().await.unwrap();
    let result = app.finding_report(&input).await.unwrap();
    assert_eq!(result.rule_digest, digest);
    assert_eq!(result.state, JudgedState::Current);
    assert_eq!(app.store().model_revision().await.unwrap(), model_at);
    assert_eq!(
        app.store().judged_reports().await.unwrap()[0].rule_digest,
        Some(digest)
    );
    let invocations: Vec<_> = app
        .store()
        .invocations()
        .await
        .unwrap()
        .into_iter()
        .map(|(_, run)| run)
        .collect();
    let judged = invocations
        .iter()
        .filter(|r| r.executor == Executor::Judged)
        .collect::<Vec<_>>();
    assert_eq!(judged.len(), 1);
    assert_eq!(judged[0].rule_digest, Some(digest));
    assert!(
        invocations
            .iter()
            .filter(|r| r.executor == Executor::Builtin)
            .all(|r| r.rule_digest.is_none())
    );
    assert!(
        finding_states(&app)
            .await
            .iter()
            .all(|(_, stale, _, _)| !stale)
    );

    app.into_store().close().await.unwrap();
    let app = open(&path, &rule_text("can this be tested?"), None).await;
    let shown = app.check_rules(None, Some(RULE)).await.unwrap();
    assert_eq!(shown.rules[0].state, JudgedState::Current);
    assert_eq!(
        shown.rules[0].judgment.as_ref().unwrap().rule_digest,
        Some(digest)
    );
    assert!(
        app.check(None)
            .await
            .unwrap()
            .findings
            .iter()
            .filter(|f| f.rule.as_str() == RULE)
            .all(|f| f.judgment.as_ref().unwrap().rule_digest == Some(digest) && !f.stale)
    );
    app.into_store().close().await.unwrap();
}

#[tokio::test]
async fn changing_the_rule_stales_every_finding_and_rejects_the_old_inflight_version() {
    let (mut app, path) = fixture().await;
    let input = report(&app, &["C1", "C2"]).await;
    app.finding_report(&input).await.unwrap();
    let old_digest = input.rule_digest.clone();
    app.into_store().close().await.unwrap();

    let mut app = open(
        &path,
        &rule_text("can this be tested without guessing?"),
        None,
    )
    .await;
    let current = app.check_rules(None, Some(RULE)).await.unwrap();
    assert_eq!(current.rules[0].state, JudgedState::Stale);
    assert_ne!(current.rules[0].rule_digest.to_string(), old_digest);
    assert_eq!(
        current.rules[0]
            .judgment
            .as_ref()
            .unwrap()
            .rule_digest
            .map(|d| d.to_string()),
        Some(old_digest)
    );
    let found = finding_states(&app).await;
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|(_, stale, _, _)| *stale));
    let at = app.store().revision().await.unwrap();
    let previous = app.store().judged_reports().await.unwrap();
    let invocations = app.store().invocations().await.unwrap();
    // Both a populated and an empty late response must leave the old record intact.
    for empty in [false, true] {
        let mut late = input.clone();
        late.read_at = at.0;
        if empty {
            late.findings.clear();
        }
        assert!(app.finding_report(&late).await.is_err());
        assert_eq!(app.store().revision().await.unwrap(), at);
        assert_eq!(app.store().judged_reports().await.unwrap(), previous);
        assert_eq!(app.store().invocations().await.unwrap(), invocations);
    }
    let fresh = report(&app, &["C1", "C2"]).await;
    assert_eq!(
        app.finding_report(&fresh).await.unwrap().state,
        JudgedState::Current
    );
    assert!(
        finding_states(&app)
            .await
            .iter()
            .all(|(_, stale, _, _)| !stale)
    );
}

#[tokio::test]
async fn a_missing_or_malformed_version_never_replaces_a_previous_judgment() {
    let (mut app, _) = fixture().await;
    let initial = report(&app, &["C1"]).await;
    app.finding_report(&initial).await.unwrap();
    let before = app.store().judged_reports().await.unwrap();
    let at = app.store().revision().await.unwrap();
    let invocations = app.store().invocations().await.unwrap();
    for malformed in [
        String::new(), // a missing digest reaches the App as empty text
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        "é".repeat(32),
    ] {
        let mut input = initial.clone();
        input.read_at = at.0;
        input.rule_digest = malformed;
        assert!(app.finding_report(&input).await.is_err());
        assert_eq!(app.store().revision().await.unwrap(), at);
        assert_eq!(app.store().judged_reports().await.unwrap(), before);
        assert_eq!(app.store().invocations().await.unwrap(), invocations);
    }
}

#[tokio::test]
async fn an_old_row_without_a_version_stays_readable_and_stale_until_rejudged() {
    let (mut app, path) = fixture().await;
    let input = report(&app, &["C1"]).await;
    app.finding_report(&input).await.unwrap();
    app.into_store().close().await.unwrap();
    // A database written before digests existed holds NULL; a definition cannot be established after the fact.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("UPDATE judged_reports SET rule_digest = NULL", [])
        .unwrap();
    drop(conn);

    let mut app = open(&path, &rule_text("can this be tested?"), None).await;
    let shown = app.check_rules(None, Some(RULE)).await.unwrap();
    assert_eq!(shown.rules[0].state, JudgedState::Stale);
    assert_eq!(shown.rules[0].judgment.as_ref().unwrap().rule_digest, None);
    let found = finding_states(&app).await;
    assert_eq!(found.len(), 1);
    assert!(found[0].1);
    let again = report(&app, &["C1"]).await;
    assert_eq!(
        app.finding_report(&again).await.unwrap().state,
        JudgedState::Current
    );
    assert!(!finding_states(&app).await[0].1);
}

#[tokio::test]
async fn editing_what_does_not_decide_the_judgment_leaves_it_current() {
    let (mut app, path) = fixture().await;
    let input = report(&app, &["C1"]).await;
    app.finding_report(&input).await.unwrap();
    app.into_store().close().await.unwrap();
    let edited = rule_text("can this be tested?")
        .replace("review a clause", "a reworded summary")
        .replace("state the result", "a reworded fix")
        .replace("['check','edit']", "['check']")
        .replace("['should']", "['must']")
        .replace("level='warning'", "level='error'");
    let app = open(&path, &edited, None).await;
    let shown = app.check_rules(None, Some(RULE)).await.unwrap();
    assert_eq!(shown.rules[0].rule_digest.to_string(), input.rule_digest);
    assert_eq!(shown.rules[0].state, JudgedState::Current);
    assert!(finding_states(&app).await.iter().all(|f| !f.1));
}

#[tokio::test]
async fn empty_versioned_judgments_are_invalidated_by_a_changed_rule() {
    let (mut app, path) = fixture().await;
    let input = report(&app, &[]).await;
    app.finding_report(&input).await.unwrap();
    assert_eq!(
        app.check_rules(None, Some(RULE)).await.unwrap().rules[0].state,
        JudgedState::Current
    );
    app.into_store().close().await.unwrap();
    let mut app = open(&path, &rule_text("a different question"), None).await;
    assert!(finding_states(&app).await.is_empty());
    assert_eq!(
        app.check_rules(None, Some(RULE)).await.unwrap().rules[0].state,
        JudgedState::Stale
    );
    assert!(
        app.status()
            .await
            .unwrap()
            .judged
            .iter()
            .any(|r| r.rule.as_str() == RULE)
    );
    let input = report(&app, &[]).await;
    assert_eq!(
        app.finding_report(&input).await.unwrap().state,
        JudgedState::Current
    );
    assert!(
        app.status()
            .await
            .unwrap()
            .judged
            .iter()
            .all(|r| r.rule.as_str() != RULE)
    );
}

#[tokio::test]
async fn effective_policy_changes_reweight_findings_without_changing_the_rule_version() {
    let (mut app, path) = fixture().await;
    let input = report(&app, &["C1"]).await;
    app.finding_report(&input).await.unwrap();
    let digest = input.rule_digest;
    app.into_store().close().await.unwrap();
    let app = open(&path, &rule_text("can this be tested?"), Some("note")).await;
    let shown = app.check_rules(None, Some(RULE)).await.unwrap();
    assert_eq!(shown.rules[0].rule_digest.to_string(), digest);
    assert_eq!(shown.rules[0].state, JudgedState::Current);
    assert_eq!(
        finding_states(&app).await,
        vec![("review C1".into(), false, FindingState::Open, Level::Note)]
    );
}

#[tokio::test]
async fn model_scope_staleness_does_not_stale_an_unchanged_subject() {
    let (mut app, _) = fixture().await;
    let input = report(&app, &["C1", "C2"]).await;
    app.finding_report(&input).await.unwrap();
    put_clauses(&mut app, "stores the item differently", "returns the item").await;
    let shown = app.check_rules(None, Some(RULE)).await.unwrap();
    assert_eq!(shown.rules[0].state, JudgedState::Stale);
    let found = finding_states(&app).await;
    assert!(
        found
            .iter()
            .find(|(message, _, _, _)| message == "review C1")
            .unwrap()
            .1
    );
    assert!(
        !found
            .iter()
            .find(|(message, _, _, _)| message == "review C2")
            .unwrap()
            .1
    );
}

#[tokio::test]
async fn a_matching_rule_version_does_not_hide_a_report_on_an_older_model() {
    let (mut app, _) = fixture().await;
    let input = report(&app, &["C2"]).await;
    put_clauses(&mut app, "a changed first clause", "returns the item").await;
    let result = app.finding_report(&input).await.unwrap();
    assert!(result.on_older_revision);
    assert_eq!(result.state, JudgedState::Stale);
    assert!(finding_states(&app).await[0].1);
}
