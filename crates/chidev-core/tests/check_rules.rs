//! `App::check` with rule selectors: the findings are narrowed to the rules selected and the rest is counted in
//! `outside`, and a selector that matches no registered check, of any of the three kinds, is refused before the run.
//! The judged rule is data and the external one is registered with no program behind it (`check` starts none), so the
//! registry has one of each kind.

use std::path::PathBuf;

use chidev_core::app::ports::IdSource;
use chidev_core::app::{App, AppError, CheckResult, ClauseInput, Found, Registry};
use chidev_core::domain::check::Moment;
use chidev_core::domain::check::builtin;
use chidev_core::domain::check::external::ExternalRule;
use chidev_core::domain::vo::finding::{RuleId, RuleSelector};
use chidev_core::domain::vo::model_section::ModelSection;
use chidev_core::domain::vo::timestamp::Timestamp;
use chidev_core::infra::clock::FixedClock;
use chidev_core::infra::judged;
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

const BUILTIN: &str = "cite.uncited-clause";
const JUDGED: &str = "judged.clause-review";
const EXTERNAL: &str = "ext.contract";

fn selector(text: &str) -> RuleSelector {
    RuleSelector::try_from(text).unwrap()
}

fn selectors(texts: &[&str]) -> Vec<RuleSelector> {
    texts.iter().map(|t| selector(t)).collect()
}

/// A registry of the builtin checks, one judged rule and one external rule, all at the defaults.
fn registry() -> Registry {
    let judged = judged::parse(&format!(
        "id='{JUDGED}'\nsummary='review a clause'\nsubject='clause'\nmoments=['check','edit']\nlevel='warning'\nquestion='can this be tested?'\ntrue='no observable result'\nfalse='an observable result'\nfix='state the result'\nwhen=['should']\n"
    ))
    .unwrap();
    let external = ExternalRule::new(
        RuleId::from_static(EXTERNAL),
        vec![Moment::Check],
        vec![ModelSection::Contracts],
        false,
        "config-v1".into(),
    )
    .unwrap();
    Registry::with_external(builtin::all(), vec![judged], vec![external], &[]).unwrap()
}

/// An app over a store of its own, with no project yet.
async fn bare(name: &str) -> App<SqliteStore, Counter> {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("chidev-check-rules-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    App::with_registry(
        SqliteStore::open(dir.join("chidev.db")).await.unwrap(),
        Counter::default(),
        registry(),
        FixedClock(Timestamp::from_millis(1_000)),
    )
}

/// An app with a project of two Contexts: `core` has a Contract whose Clauses nothing cites, and `billing` has
/// nothing, so there are findings of several rules, about each.
async fn app(name: &str) -> App<SqliteStore, Counter> {
    let mut app = bare(name).await;
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.add_context("billing", &[]).await.unwrap();
    app.put_contract(
        "core",
        "stack",
        "Stack",
        &[ClauseInput {
            label: "C1".into(),
            text: "a stack pops what it pushed".into(),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    app
}

fn rules(result: &CheckResult) -> Vec<&str> {
    result.findings.iter().map(|f| f.rule.as_str()).collect()
}

fn of_rules(found: &[Found], keep: impl Fn(&str) -> bool) -> Vec<Found> {
    found
        .iter()
        .filter(|f| keep(f.rule.as_str()))
        .cloned()
        .collect()
}

#[tokio::test]
async fn rules_narrow_the_findings_and_the_rest_is_counted_outside() {
    let app = app("narrow").await;
    let all = app.check(None, &[]).await.unwrap();
    assert_eq!(all.outside, 0);
    // the project has findings of more than one rule, so a selector has something to leave out
    for rule in [BUILTIN, "context.no-interface", "context.no-harness"] {
        assert!(rules(&all).contains(&rule), "{rule} in {:?}", rules(&all));
    }

    // by id
    let one = app.check(None, &selectors(&[BUILTIN])).await.unwrap();
    assert_eq!(one.findings, of_rules(&all.findings, |r| r == BUILTIN));
    assert!(!one.findings.is_empty());
    assert_eq!(one.outside, all.findings.len() - one.findings.len());
    assert!(one.outside > 0);

    // by prefix, whole segments
    let family = app.check(None, &selectors(&["context"])).await.unwrap();
    assert_eq!(
        family.findings,
        of_rules(&all.findings, |r| r.starts_with("context."))
    );
    assert!(rules(&family).contains(&"context.no-interface"));
    assert_eq!(family.outside, all.findings.len() - family.findings.len());

    // several selectors are a union, and a finding two of them match is returned once
    let union = app
        .check(
            None,
            &selectors(&[BUILTIN, "context.no-harness", "context"]),
        )
        .await
        .unwrap();
    assert_eq!(
        union.findings,
        of_rules(&all.findings, |r| r == BUILTIN || r.starts_with("context."))
    );
}

#[tokio::test]
async fn a_context_and_rules_narrow_together_and_a_finding_left_out_by_both_is_counted_once() {
    let app = app("both").await;
    let all = app.check(None, &[]).await.unwrap();
    let billing = app.check(Some("billing"), &[]).await.unwrap();
    assert!(billing.outside > 0);

    let both = app
        .check(Some("billing"), &selectors(&["context.no-harness"]))
        .await
        .unwrap();
    assert_eq!(
        both.findings,
        of_rules(&billing.findings, |r| r == "context.no-harness")
    );
    assert!(!both.findings.is_empty());
    assert_eq!(both.outside, all.findings.len() - both.findings.len());
}

#[tokio::test]
async fn a_selector_of_a_rule_with_no_finding_is_known_and_gives_none() {
    let app = app("quiet").await;
    // `pin.stale` is a builtin check; nothing is pinned, so it has nothing to say
    let none = app.check(None, &selectors(&["pin"])).await.unwrap();
    assert!(none.findings.is_empty(), "{:?}", rules(&none));
    let all = app.check(None, &[]).await.unwrap();
    assert_eq!(none.outside, all.findings.len());
}

#[tokio::test]
async fn a_builtin_judged_or_external_rule_is_a_known_selector() {
    let app = app("known").await;
    for (kind, id, family) in [
        ("builtin", BUILTIN, "cite"),
        ("judged", JUDGED, "judged"),
        ("external", EXTERNAL, "ext"),
    ] {
        for text in [id, family] {
            app.check(None, &selectors(&[text]))
                .await
                .unwrap_or_else(|e| panic!("the {kind} selector `{text}`: {e}"));
        }
    }
}

#[tokio::test]
async fn a_selector_that_matches_no_check_is_refused_with_the_names_near_it() {
    let app = app("unknown").await;

    // a typo of an id
    match app
        .check(None, &selectors(&["cite.uncited-claus"]))
        .await
        .unwrap_err()
    {
        AppError::UnknownRuleSelector { given, near } => {
            assert_eq!(given, "cite.uncited-claus");
            // nearest first, as `did_you_mean` gives them
            assert_eq!(near.first().map(String::as_str), Some(BUILTIN), "{near:?}");
        }
        e => panic!("expected an unknown rule selector, got {e:?}"),
    }

    // a typo of a family
    match app.check(None, &selectors(&["judge"])).await.unwrap_err() {
        AppError::UnknownRuleSelector { given, near } => {
            assert_eq!(given, "judge");
            assert!(near.contains(&"judged".to_string()), "{near:?}");
        }
        e => panic!("expected an unknown rule selector, got {e:?}"),
    }

    // a part of a segment is no family, though it begins an id; the family it is a part of is near
    match app.check(None, &selectors(&["cit"])).await.unwrap_err() {
        AppError::UnknownRuleSelector { given, near } => {
            assert_eq!(given, "cit");
            assert!(near.contains(&"cite".to_string()), "{near:?}");
        }
        e => panic!("expected an unknown rule selector, got {e:?}"),
    }

    // the first that matches nothing is the one said, whatever stands before it
    match app
        .check(None, &selectors(&[BUILTIN, "nothing", "other"]))
        .await
        .unwrap_err()
    {
        AppError::UnknownRuleSelector { given, near } => {
            assert_eq!(given, "nothing");
            assert!(near.is_empty());
        }
        e => panic!("expected an unknown rule selector, got {e:?}"),
    }
}

#[tokio::test]
async fn a_selector_is_refused_before_the_run_reads_anything() {
    // no project: a run would say so, and a context that is not there would be refused, but the selector comes first
    let app = bare("before").await;
    assert!(matches!(
        app.check(Some("nowhere"), &selectors(&["nothing"]))
            .await
            .unwrap_err(),
        AppError::UnknownRuleSelector { .. }
    ));
    assert_eq!(app.check(None, &[]).await.unwrap_err(), AppError::NoProject);
}

#[tokio::test]
async fn the_refusal_is_an_unknown_name_with_the_names_near_it() {
    let app = app("diagnosed").await;
    let error = app
        .check(None, &selectors(&["cite.uncited-claus"]))
        .await
        .unwrap_err();
    let diagnostics = app.diagnose(&error).await;
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "input.unknown-name");
    let AppError::UnknownRuleSelector { near, .. } = &error else {
        panic!("expected an unknown rule selector, got {error:?}");
    };
    assert_eq!(&diagnostics[0].did_you_mean, near);
    assert_eq!(diagnostics[0].did_you_mean[0], BUILTIN);
}
