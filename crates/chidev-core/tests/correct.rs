//! A forced correction of records over the real store: where a text is found, what replaces it, what the records
//! and the file keep of it, and what a refusal shows.
//!
//! The text sought is a token that must not be anywhere after a correction: not in a record, not in an answer, not
//! in a refusal, and not in the bytes of the store's file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chidev_core::app::commit::{CorrectTrigger, Trigger};
use chidev_core::app::ports::measure::{
    BoxFuture, MutateReport, MutateRequest, Runner, RunnerError, SimulateReport, SimulateRequest,
};
use chidev_core::app::ports::{IdSource, InvocationField, Store};
use chidev_core::app::{
    App, AppError, AssumptionInput, CaseInput, ChangeRequestInput, ClauseInput, ClauseRef, Cleared,
    CorrectInput, CruxKind, DecisionMakerInput, Intent, ItemInput, ItemRef, OptionInput, Place,
    RaiseCruxInput, RecommendationInput, Registry, Remaining, ReportInput, ResolutionInput,
    TransitionStatus, ViewInput,
};
use chidev_core::domain::entity::change_request::ChangeRequestField;
use chidev_core::domain::entity::crux::CruxField;
use chidev_core::domain::entity::measurement::MeasurementField;
use chidev_core::domain::repository::{ChangeRequestRepository, CruxRepository};
use chidev_core::domain::vo::digest::Digest;
use chidev_core::domain::vo::id::{ChangeRequestId, CruxId};
use chidev_core::domain::vo::measure::{MeasureKind, SettingsDigest, StepEvidence};
use chidev_core::domain::vo::model_section::ModelSection;
use chidev_core::domain::vo::name::{AssetName, ContextName};
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::judged;
use chidev_core::infra::sqlite::SqliteStore;
use uuid::Uuid;

/// The text that must not stay.
const TOKEN: &str = "tok-SECRET-123";

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

fn fresh_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir()
        .join(format!(
            "chidev-correct-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
        .join("chidev.db")
}

/// An App over a new store with its Project made, under the checks a judged rule adds. Returns where the file is,
/// and the digest of the rule, which a report echoes.
async fn new_app(name: &str) -> (TestApp, PathBuf, String) {
    let path = fresh_path(name);
    let store = SqliteStore::open(&path).await.unwrap();
    let rule = judged::parse(
        "id = \"judged.vague\"\nsummary = \"s\"\nsubject = \"clause\"\nmoments = [\"edit\"]\nlevel = \"warning\"\n\
         question = \"q\"\ntrue = \"t\"\nfalse = \"f\"\nfix = \"x\"\n",
    )
    .unwrap();
    let digest = rule.digest().to_string();
    let registry = Registry::builtin_with_judged(vec![rule], &[]).unwrap();
    let mut app = App::with_registry(
        store,
        Counter::default(),
        registry,
        FixedClock(Timestamp::from_millis(1_000)),
    );
    app.init().await.unwrap();
    (app, path, digest)
}

fn request(title: &str, situation: Option<String>) -> ChangeRequestInput {
    ChangeRequestInput {
        title: title.into(),
        situation,
        intent: Intent::KeepBehavior,
        parent: None,
        issuer: None,
        targets: vec![],
        keep: vec![],
        assumptions: vec![],
        addresses: vec![],
    }
}

fn alice() -> DecisionMakerInput {
    DecisionMakerInput::Person("alice".into())
}

fn carol() -> DecisionMakerInput {
    DecisionMakerInput::Person("carol".into())
}

/// A correction of `places`, as carol says it.
fn correction(text: &str, places: Vec<Place>) -> CorrectInput {
    CorrectInput {
        text: text.into(),
        reason: "a token was pasted".into(),
        by: carol(),
        places,
    }
}

async fn id_of(app: &TestApp, title: &str) -> ChangeRequestId {
    app.store()
        .change_requests()
        .await
        .unwrap()
        .iter()
        .find(|c| c.title().as_str() == title)
        .unwrap_or_else(|| panic!("no change request titled {title}"))
        .id()
}

async fn title_of(app: &TestApp, id: ChangeRequestId) -> String {
    app.store()
        .change_requests()
        .await
        .unwrap()
        .iter()
        .find(|c| c.id() == id)
        .unwrap()
        .title()
        .to_string()
}

/// The bytes of the files the store keeps: the database and, when they exist, its journal and write-ahead log.
fn files_of(path: &Path) -> Vec<(String, Vec<u8>)> {
    ["", "-journal", "-wal"]
        .iter()
        .filter_map(|suffix| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            std::fs::read(&name)
                .ok()
                .map(|bytes| (format!("the database{suffix}"), bytes))
        })
        .collect()
}

fn holds_token(bytes: &[u8]) -> bool {
    bytes.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes())
}

/// A Crux in `cr` whose one option is recommended for a reason that holds the token.
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
            recommended: Some(RecommendationInput {
                reason: Some(format!("{TOKEN} is why")),
                confidence: Some(60),
            }),
        }],
        importance: None,
        supersedes: None,
    }
}

/// What a refusal shows anywhere: its message, its debug print and its diagnostics, and the rule ids of those.
async fn shown(app: &TestApp, err: &AppError) -> (String, Vec<String>) {
    let diagnostics = app.diagnose(err).await;
    let codes = diagnostics.iter().map(|d| d.code.clone()).collect();
    (format!("{err} {err:?} {diagnostics:?}"), codes)
}

#[tokio::test]
async fn a_text_is_found_where_it_is_kept_and_replaced_there_and_nowhere_in_the_file() {
    let (mut app, path, digest) = new_app("everywhere").await;
    // a situation and an assumption, a Crux with a recommendation, a resolution
    app.open_change_request(&ChangeRequestInput {
        assumptions: vec![AssumptionInput {
            statement: "it is kept".into(),
            reason: format!("because {TOKEN}"),
            evidence: vec![],
        }],
        ..request("plain", Some(format!("it holds {TOKEN}")))
    })
    .await
    .unwrap();
    app.raise_crux(&question("plain")).await.unwrap();
    app.open_change_request(&request("closed", None))
        .await
        .unwrap();
    app.resolve_change_request(
        "closed",
        &ResolutionInput::Done,
        &format!("closed over {TOKEN}"),
        &alice(),
    )
    .await
    .unwrap();
    // two reports of a judged rule: the second replaces the first, which the record of runs keeps
    for judge in [format!("agent {TOKEN}"), "agent-2".to_string()] {
        let read = app.check_rules(None, None).await.unwrap();
        app.finding_report(&ReportInput {
            rule: "judged.vague".into(),
            rule_digest: digest.clone(),
            judge,
            read_at: read.store_revision.0,
            findings: vec![],
        })
        .await
        .unwrap();
    }

    let plain = id_of(&app, "plain").await;
    let closed = id_of(&app, "closed").await;
    let crux: CruxId = app.store().cruxes().await.unwrap()[0].id();
    let seq = app
        .store()
        .invocations()
        .await
        .unwrap()
        .into_iter()
        .find(|(_, run)| run.judge.as_deref() == Some(format!("agent {TOKEN}").as_str()))
        .map(|(seq, _)| seq)
        .expect("the run of the first report");
    let places = vec![
        Place::ChangeRequest {
            id: plain,
            field: ChangeRequestField::Situation,
        },
        Place::ChangeRequest {
            id: plain,
            field: ChangeRequestField::AssumptionReason { assumption: 0 },
        },
        Place::ChangeRequest {
            id: closed,
            field: ChangeRequestField::StepReason { step: 0 },
        },
        Place::Crux {
            id: crux,
            field: CruxField::RecommendationReason,
        },
        Place::Invocation {
            seq,
            field: InvocationField::Judge,
        },
    ];
    let mut want = places.clone();
    want.sort();

    // the text is in the file before, and find lists every place and nothing of the text
    assert!(files_of(&path).iter().any(|(_, bytes)| holds_token(bytes)));
    let mut found = app.find_text(TOKEN).await.unwrap();
    found.sort();
    assert_eq!(found, want);
    assert!(!format!("{found:?}").contains(TOKEN));

    let revision = app.store().revision().await.unwrap();
    let answer = app
        .correct_records(&correction(TOKEN, places))
        .await
        .unwrap();
    let mut corrected = answer.corrected.clone();
    corrected.sort();
    assert_eq!(corrected, want);
    assert_eq!(answer.remaining, Remaining::Found(vec![]));
    assert_eq!(answer.cleared, Cleared::Done);
    assert!(!format!("{answer:?}").contains(TOKEN));
    assert!(app.store().revision().await.unwrap() > revision);
    assert!(app.find_text(TOKEN).await.unwrap().is_empty());

    // status shows the mark, and a step that says which fields, why, who and when
    let status = app.status().await.unwrap();
    let plain_status = status
        .change_requests
        .iter()
        .find(|c| c.title == "plain")
        .unwrap();
    assert_eq!(
        plain_status.situation.as_deref(),
        Some("it holds [redacted]")
    );
    assert_eq!(plain_status.assumptions[0].reason, "because [redacted]");
    match plain_status.history.as_slice() {
        [TransitionStatus::Corrected(c)] => {
            assert_eq!(c.fields, ["situation", "assumption[0].reason"]);
            assert_eq!(c.reason, "a token was pasted");
            assert_eq!((c.by_kind, c.by.as_str()), ("person", "carol"));
            assert_eq!(c.at, 1_000);
        }
        other => panic!("expected one corrected step, got {other:?}"),
    }
    let closed_status = status
        .change_requests
        .iter()
        .find(|c| c.title == "closed")
        .unwrap();
    assert_eq!(closed_status.state, "resolved");
    match closed_status.history.as_slice() {
        [
            TransitionStatus::Resolved(r),
            TransitionStatus::Corrected(c),
        ] => {
            assert_eq!(r.reason, "closed over [redacted]");
            assert_eq!(c.fields, ["step[0].reason"]);
        }
        other => panic!("expected a resolution and a correction, got {other:?}"),
    }
    let crux_status = &status.cruxes[0];
    assert_eq!(
        crux_status
            .recommended
            .as_ref()
            .and_then(|r| r.reason.as_deref()),
        Some("[redacted] is why")
    );
    assert_eq!(crux_status.corrections.len(), 1);
    assert_eq!(crux_status.corrections[0].fields, ["recommendation.reason"]);
    let runs = app.store().invocations().await.unwrap();
    let (_, run) = runs.iter().find(|(s, _)| *s == seq).unwrap();
    assert_eq!(run.judge.as_deref(), Some("agent [redacted]"));
    assert_eq!(run.corrections.len(), 1);

    // and the file keeps nothing of it
    for (name, bytes) in files_of(&path) {
        assert!(!holds_token(&bytes), "{name} holds the text");
    }
}

#[tokio::test]
async fn a_correction_is_a_trigger_of_its_own_kind_read_back_by_its_word() {
    let trigger = Trigger::Correct(CorrectTrigger::CorrectRecords);
    assert_eq!(trigger.word(), "correct_records");
    assert_eq!(Trigger::from_word("correct_records"), Some(trigger));
    // it runs no check
    assert_eq!(trigger.moment(), None);
    assert!(Trigger::all().contains(&trigger));
    // and it records no run of a check: the correction of records is not a check
    let (mut app, _, _) = new_app("trigger").await;
    app.open_change_request(&request("plain", Some(format!("holds {TOKEN}"))))
        .await
        .unwrap();
    let before = app.store().invocations().await.unwrap().len();
    let id = id_of(&app, "plain").await;
    app.correct_records(&correction(
        TOKEN,
        vec![Place::ChangeRequest {
            id,
            field: ChangeRequestField::Situation,
        }],
    ))
    .await
    .unwrap();
    assert_eq!(app.store().invocations().await.unwrap().len(), before);
}

#[tokio::test]
async fn a_resolved_change_request_that_is_corrected_stays_resolved_and_can_be_reopened() {
    let (mut app, _, _) = new_app("reopen").await;
    app.open_change_request(&request("closed", None))
        .await
        .unwrap();
    app.resolve_change_request(
        "closed",
        &ResolutionInput::Done,
        &format!("closed over {TOKEN}"),
        &alice(),
    )
    .await
    .unwrap();
    let id = id_of(&app, "closed").await;
    app.correct_records(&correction(
        TOKEN,
        vec![Place::ChangeRequest {
            id,
            field: ChangeRequestField::StepReason { step: 0 },
        }],
    ))
    .await
    .unwrap();
    let status = app.status().await.unwrap();
    let closed = &status.change_requests[0];
    assert_eq!(closed.state, "resolved");
    assert_eq!(
        closed.resolution.as_ref().map(|r| r.reason.as_str()),
        Some("closed over [redacted]")
    );
    // resolving it again is still refused, and reopening it works
    assert!(
        app.resolve_change_request("closed", &ResolutionInput::Done, "again", &alice())
            .await
            .is_err()
    );
    app.reopen_change_request("closed", "it came back", &alice())
        .await
        .unwrap();
    let status = app.status().await.unwrap();
    assert_eq!(status.change_requests[0].state, "open");
    let history = &status.change_requests[0].history;
    assert_eq!(history.len(), 3);
    assert!(matches!(history[0], TransitionStatus::Resolved(_)));
    assert!(matches!(history[1], TransitionStatus::Corrected(_)));
    assert!(matches!(history[2], TransitionStatus::Reopened(_)));
}

#[tokio::test]
async fn a_title_that_would_be_taken_takes_a_numbered_mark_and_titles_stay_distinct() {
    // `tok` and `tok x`: both are corrected in one order, and stay different
    let (mut app, _, _) = new_app("titles").await;
    app.open_change_request(&request(TOKEN, None))
        .await
        .unwrap();
    app.open_change_request(&request(&format!("{TOKEN} x"), None))
        .await
        .unwrap();
    let places = app.find_text(TOKEN).await.unwrap();
    assert_eq!(places.len(), 2);
    app.correct_records(&correction(TOKEN, places))
        .await
        .unwrap();
    let mut titles: Vec<_> = app
        .status()
        .await
        .unwrap()
        .change_requests
        .into_iter()
        .map(|c| c.title)
        .collect();
    titles.sort();
    assert_eq!(titles, ["[redacted]", "[redacted] x"]);

    // `tok` and `[redacted]`: the one corrected cannot take the mark the other has
    let (mut app, _, _) = new_app("titles-taken").await;
    app.open_change_request(&request(TOKEN, None))
        .await
        .unwrap();
    app.open_change_request(&request("[redacted]", None))
        .await
        .unwrap();
    let places = app.find_text(TOKEN).await.unwrap();
    assert_eq!(places.len(), 1);
    let Place::ChangeRequest { id, .. } = places[0] else {
        panic!("a change request");
    };
    app.correct_records(&correction(TOKEN, places))
        .await
        .unwrap();
    assert_eq!(title_of(&app, id).await, "[redacted 2]");
}

#[tokio::test]
async fn a_refusal_carries_its_rule_and_never_a_text_of_a_record() {
    let (mut app, path, _) = new_app("refusals").await;
    // a request with the token in its situation; and one whose title holds it
    app.open_change_request(&request(
        "ordinary-request-xyz",
        Some(format!("it holds {TOKEN}")),
    ))
    .await
    .unwrap();
    app.open_change_request(&request(
        &format!("{TOKEN} request"),
        Some("nothing secret here".into()),
    ))
    .await
    .unwrap();
    let ordinary = id_of(&app, "ordinary-request-xyz").await;
    let titled = id_of(&app, &format!("{TOKEN} request")).await;
    let situation = |id| Place::ChangeRequest {
        id,
        field: ChangeRequestField::Situation,
    };
    let revision = app.store().revision().await.unwrap();

    // (what is asked, the rule it is refused by, what the refusal may not show)
    let unknown = ChangeRequestId::from_uuid(Uuid::from_u128(999));
    let cases: Vec<(CorrectInput, &str)> = vec![
        // the text is not in a field that is set
        (
            correction(
                TOKEN,
                vec![Place::ChangeRequest {
                    id: ordinary,
                    field: ChangeRequestField::Title,
                }],
            ),
            "correct.text-absent",
        ),
        // the record has no such field
        (
            correction(
                TOKEN,
                vec![Place::ChangeRequest {
                    id: ordinary,
                    field: ChangeRequestField::AssumptionReason { assumption: 9 },
                }],
            ),
            "correct.no-such-place",
        ),
        // a record that is not there
        (
            correction(TOKEN, vec![situation(unknown)]),
            "correct.unknown-record",
        ),
        // a place named twice
        (
            correction(TOKEN, vec![situation(ordinary), situation(ordinary)]),
            "correct.place-twice",
        ),
        // none at all
        (correction(TOKEN, vec![]), "correct.no-field"),
        // a text that is empty, and one that is a part of the mark
        (
            correction("", vec![situation(ordinary)]),
            "correct.text-empty",
        ),
        (
            correction("red", vec![situation(ordinary)]),
            "correct.text-in-mark",
        ),
        // a request whose title holds the text: it is named by its id, and the title is not shown
        (
            correction(TOKEN, vec![situation(titled)]),
            "correct.text-absent",
        ),
        (
            correction(
                TOKEN,
                vec![Place::ChangeRequest {
                    id: titled,
                    field: ChangeRequestField::AssumptionStatement { assumption: 0 },
                }],
            ),
            "correct.no-such-place",
        ),
    ];
    for (input, rule) in cases {
        let err = app.correct_records(&input).await.unwrap_err();
        assert!(matches!(err, AppError::Violated { .. }), "{rule}: {err:?}");
        let (text, codes) = shown(&app, &err).await;
        assert_eq!(codes, [rule], "{err:?}");
        assert!(!text.contains(TOKEN), "{rule} shows the text: {text}");
        assert!(
            !text.contains("ordinary-request-xyz"),
            "{rule} names a request by its title: {text}"
        );
        // nothing was changed by a refusal
        assert_eq!(app.store().revision().await.unwrap(), revision, "{rule}");
    }
    // a record is named by its id
    let err = app
        .correct_records(&correction(TOKEN, vec![situation(titled)]))
        .await
        .unwrap_err();
    assert!(shown(&app, &err).await.0.contains(&titled.to_string()));

    // the same texts are refused for a search
    for (text, rule) in [("", "correct.text-empty"), ("red", "correct.text-in-mark")] {
        let err = app.find_text(text).await.unwrap_err();
        assert_eq!(shown(&app, &err).await.1, [rule]);
    }

    // the reason and the name of who corrects are kept in the records, so they may not hold the text
    let mut input = correction(TOKEN, vec![situation(ordinary)]);
    input.reason = format!("removing {TOKEN}");
    let err = app.correct_records(&input).await.unwrap_err();
    assert!(matches!(err, AppError::Violated { .. }), "{err:?}");
    let (text, codes) = shown(&app, &err).await;
    assert_eq!(codes, ["correct.text-in-reason"]);
    assert!(!text.contains(TOKEN), "{text}");
    let diagnostics = app.diagnose(&err).await;
    assert_eq!(
        diagnostics[0].values["field"],
        chidev_core::app::DiagValue::Text("reason".into())
    );
    let mut input = correction(TOKEN, vec![situation(ordinary)]);
    input.by = DecisionMakerInput::Person(format!("{TOKEN} person"));
    let err = app.correct_records(&input).await.unwrap_err();
    let (text, codes) = shown(&app, &err).await;
    assert_eq!(codes, ["correct.text-in-reason"]);
    assert!(!text.contains(TOKEN), "{text}");
    let diagnostics = app.diagnose(&err).await;
    assert_eq!(
        diagnostics[0].values["field"],
        chidev_core::app::DiagValue::Text("by".into())
    );
    assert_eq!(app.store().revision().await.unwrap(), revision);
    // and the text is still there, where it was
    assert!(files_of(&path).iter().any(|(_, bytes)| holds_token(bytes)));
}

/// The settings every run of [`Leaky`] is made under, and that it says they are.
fn leaky_settings() -> SettingsDigest {
    SettingsDigest::new(Digest::of(b"settings"))
}

/// A runner whose one command wrote the token on standard error.
#[derive(Debug)]
struct Leaky;

impl Runner for Leaky {
    fn settings<'a>(
        &'a self,
        _: &'a ContextName,
        _: &'a AssetName,
        _: &'a [AssetName],
    ) -> BoxFuture<'a, Result<SettingsDigest, RunnerError>> {
        Box::pin(async { Ok(leaky_settings()) })
    }

    fn mutate<'a>(
        &'a self,
        _: &'a MutateRequest,
    ) -> BoxFuture<'a, Result<MutateReport, RunnerError>> {
        Box::pin(async {
            Ok(MutateReport {
                settings: leaky_settings(),
                sites: 0,
                planned: 0,
                mutants: Vec::new(),
                failure: None,
                evidence: vec![StepEvidence {
                    step: "baseline-test".into(),
                    command: "cargo test".into(),
                    command_digest: None,
                    exit_code: Some(0),
                    duration_ms: 3,
                    stderr: format!("warning: bad {TOKEN}"),
                }],
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
}

#[tokio::test]
async fn a_measurement_is_found_corrected_and_shown_with_its_corrections_by_status() {
    let path = fresh_path("measurement");
    let store = SqliteStore::open(&path).await.unwrap();
    let mut app: TestApp = App::new(
        store,
        Counter::default(),
        FixedClock(Timestamp::from_millis(1_000)),
    )
    .with_runner(Leaky);
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract(
        "core",
        "stack",
        "Stack",
        &[ClauseInput {
            label: "C1".into(),
            text: "push then pop".into(),
            replaces: None,
        }],
    )
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
    app.put_harness(
        "core",
        "tests",
        "fn pushes() {}\n",
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
    app.pin("core", "api", "fn push() {}").await.unwrap();
    app.measure(MeasureKind::Mutate, "core", "api")
        .await
        .unwrap();

    let places = app.find_text(TOKEN).await.unwrap();
    assert_eq!(places.len(), 1, "{places:?}");
    let Place::Measurement { id, field } = places[0] else {
        panic!("a place in a measurement, got {:?}", places[0]);
    };
    assert_eq!(field, MeasurementField::StepStderr { step: 0 });
    // before: status shows the measurement with no correction
    let before = app.status().await.unwrap();
    assert_eq!(before.measurements.len(), 1);
    assert!(before.measurements[0].corrections.is_empty());
    assert!(files_of(&path).iter().any(|(_, bytes)| holds_token(bytes)));

    let answer = app
        .correct_records(&correction(TOKEN, places.clone()))
        .await
        .unwrap();
    assert_eq!(answer.corrected, places);
    assert_eq!(answer.remaining, Remaining::Found(vec![]));
    assert_eq!(answer.cleared, Cleared::Done);

    let status = app.status().await.unwrap();
    let standing = &status.measurements[0];
    assert_eq!(standing.id, id.to_string());
    assert_eq!(standing.corrections.len(), 1);
    let c = &standing.corrections[0];
    assert_eq!(c.fields, ["step[0].stderr"]);
    assert_eq!(c.reason, "a token was pasted");
    assert_eq!((c.by_kind, c.by.as_str(), c.at), ("person", "carol", 1_000));
    // the measurement is the one it was: same id, same stamp, still current
    assert_eq!(standing.stamp, before.measurements[0].stamp);
    assert_eq!(standing.state, before.measurements[0].state);
    let stored = app.store().measurements().await.unwrap();
    assert_eq!(stored[0].evidence()[0].stderr, "warning: bad [redacted]");
    for (name, bytes) in files_of(&path) {
        assert!(!holds_token(&bytes), "{name} holds the text");
    }
}

#[tokio::test]
async fn the_view_shows_the_corrections_of_a_record_and_leaves_the_key_out_where_there_are_none() {
    let (mut app, _, _) = new_app("view").await;
    app.open_change_request(&request(&format!("{TOKEN} request"), None))
        .await
        .unwrap();
    app.open_change_request(&request("untouched", None))
        .await
        .unwrap();
    app.raise_crux(&question("untouched")).await.unwrap();
    let corrected = id_of(&app, &format!("{TOKEN} request")).await;
    app.correct_records(&correction(
        TOKEN,
        vec![
            Place::ChangeRequest {
                id: corrected,
                field: ChangeRequestField::Title,
            },
            Place::Crux {
                id: app.store().cruxes().await.unwrap()[0].id(),
                field: CruxField::RecommendationReason,
            },
        ],
    ))
    .await
    .unwrap();
    let read = app
        .view(&ViewInput {
            contexts: None,
            sections: BTreeSet::from([ModelSection::ChangeRequests, ModelSection::Cruxes]),
        })
        .await
        .unwrap();
    let view = serde_json::to_value(&read.view).unwrap();
    let requests = view["change_requests"].as_array().unwrap();
    let changed = requests
        .iter()
        .find(|c| c["title"] == "[redacted] request")
        .expect("the corrected title");
    assert_eq!(
        changed["corrections"][0]["fields"],
        serde_json::json!(["title"])
    );
    assert_eq!(changed["corrections"][0]["reason"], "a token was pasted");
    assert_eq!(
        changed["corrections"][0]["by"],
        serde_json::json!({"kind": "person", "name": "carol"})
    );
    let untouched = requests.iter().find(|c| c["title"] == "untouched").unwrap();
    assert!(untouched.get("corrections").is_none(), "{untouched}");
    assert_eq!(
        view["cruxes"][0]["corrections"][0]["fields"],
        serde_json::json!(["recommendation.reason"])
    );
    assert!(!view.to_string().contains(TOKEN));
}
