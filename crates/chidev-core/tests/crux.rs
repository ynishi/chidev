//! The Crux as a record of its own: raised in a change request, decided once, and the change request it stops waits
//! for it. These tests hold the use cases (`raise_crux`, `decide_crux`), the rules they are refused by, and what
//! `status` says of the Cruxes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use chidev_core::app::ports::{IdSource, Store};
use chidev_core::app::{
    App, AppError, BasisInput, ChangeRequestInput, ClauseInput, ClauseRef, CruxKind, DecisionInput,
    DecisionMakerInput, FindingInput, Importance, Intent, OptionInput, RaiseCruxInput, Raised,
    RecommendationInput, ResolutionInput, SubjectInput, TargetInput,
};
use chidev_core::domain::entity::change_request::ChangeRequestError;
use chidev_core::domain::repository::{ChangeRequestRepository, CruxRepository};
use chidev_core::domain::vo::crux::Decision;
use chidev_core::domain::vo::target::ExpectKind;
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

type Test = App<SqliteStore, Counter>;

fn fresh_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir()
        .join(format!(
            "chidev-crux-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
        .join("chidev.db")
}

fn request(title: &str) -> ChangeRequestInput {
    ChangeRequestInput {
        title: title.into(),
        situation: None,
        intent: Intent::ChangeBehavior,
        parent: None,
        issuer: None,
        targets: vec![],
        keep: vec![],
        assumptions: vec![],
        addresses: vec![],
    }
}

fn clause(label: &str) -> TargetInput {
    TargetInput::Clause(ClauseRef {
        context: Some("core".into()),
        contract: "rounding".into(),
        label: label.into(),
    })
}

/// A Project with the Context `core` and its Contract `rounding` (C1 and C2), and the change request
/// `Round halves`, which asks nothing of a target.
async fn project() -> Test {
    let store = SqliteStore::open(fresh_path()).await.unwrap();
    let mut app = App::new(
        store,
        Counter::default(),
        FixedClock(Timestamp::from_millis(1_000)),
    );
    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract(
        "core",
        "rounding",
        "Rounding",
        &[
            ClauseInput {
                label: "C1".into(),
                text: "a half rounds up".into(),
            },
            ClauseInput {
                label: "C2".into(),
                text: "a total is whole cents".into(),
            },
        ],
    )
    .await
    .unwrap();
    app.open_change_request(&request("Round halves"))
        .await
        .unwrap();
    app
}

fn option(choice: &str) -> OptionInput {
    OptionInput {
        choice: choice.into(),
        change: None,
        targets: vec![],
        keep: vec![],
        consequence: None,
        recommended: None,
    }
}

fn ask(cr: &str, kind: CruxKind, options: Vec<OptionInput>) -> RaiseCruxInput {
    RaiseCruxInput {
        cr: cr.into(),
        kind,
        question: "How is a half rounded?".into(),
        findings: vec![],
        options,
        importance: None,
        supersedes: None,
    }
}

async fn raise(app: &mut Test, input: &RaiseCruxInput) -> Raised {
    app.raise_crux(input).await.unwrap().value
}

fn arbitrary() -> BasisInput {
    BasisInput::Arbitrary("settled".into())
}

fn alice() -> DecisionMakerInput {
    DecisionMakerInput::Person("alice".into())
}

fn chosen(raised: &Raised, k: usize) -> DecisionInput {
    DecisionInput::Chosen {
        option: raised.options[k].id.to_string(),
        reason: Some("fairer".into()),
    }
}

async fn decide(
    app: &mut Test,
    crux: &str,
    decision: &DecisionInput,
) -> Result<chidev_core::app::Checked<()>, AppError> {
    app.decide_crux(crux, decision, &arbitrary(), &alice())
        .await
}

async fn codes(app: &Test, error: &AppError) -> Vec<String> {
    app.diagnose(error)
        .await
        .into_iter()
        .map(|d| d.code)
        .collect()
}

async fn revision(app: &Test) -> u64 {
    app.store().revision().await.unwrap().0
}

#[tokio::test]
async fn raising_a_crux_gives_back_its_id_and_the_id_of_each_option_and_records_the_recommendation()
{
    let mut app = project().await;
    let mut input = ask(
        "Round halves",
        CruxKind::Ambiguity,
        vec![option("up"), option("to even"), option("down")],
    );
    input.options[1].recommended = Some(RecommendationInput {
        reason: Some("the literal reading".into()),
        confidence: Some(85),
    });
    input.options[1].consequence = Some("the rounding tests change".into());
    input.importance = Some(Importance::High);
    let raised = raise(&mut app, &input).await;

    assert_eq!(raised.options.len(), 3);
    let choices: Vec<&str> = raised.options.iter().map(|o| o.choice.as_str()).collect();
    assert_eq!(choices, ["up", "to even", "down"]);
    let mut ids: Vec<_> = raised.options.iter().map(|o| o.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "each option has an id of its own");

    // what is stored is what was handed back
    let stored = app.store().crux(raised.id).await.unwrap().unwrap();
    assert!(stored.is_open());
    assert_eq!(stored.question().as_str(), "How is a half rounded?");
    assert_eq!(stored.kind(), CruxKind::Ambiguity);
    assert_eq!(stored.importance(), Some(Importance::High));
    let stored_ids: Vec<_> = stored.options().iter().map(|o| o.id()).collect();
    assert_eq!(
        stored_ids,
        raised.options.iter().map(|o| o.id).collect::<Vec<_>>()
    );
    let recommended = stored.recommended().unwrap();
    assert_eq!(recommended.option(), raised.options[1].id);
    assert_eq!(
        recommended.reason().map(|r| r.as_str()),
        Some("the literal reading")
    );
    assert_eq!(recommended.confidence().map(|c| c.percent()), Some(85));
    // it is raised in the change request, which does not hold it
    let cr = app.store().change_requests().await.unwrap().remove(0);
    assert_eq!(stored.raised_in(), cr.id());
    assert!(stored.findings().is_empty());
}

#[tokio::test]
async fn a_crux_carries_the_findings_it_is_raised_from_and_is_refused_when_it_is_not_well_formed() {
    let mut app = project().await;
    let mut input = ask("Round halves", CruxKind::Conflict, vec![option("up")]);
    input.findings = vec![
        FindingInput {
            rule: "cite.uncited-clause".into(),
            subject: Some(SubjectInput::Clause(ClauseRef {
                context: Some("core".into()),
                contract: "rounding".into(),
                label: "C1".into(),
            })),
        },
        FindingInput {
            rule: "context.no-harness".into(),
            subject: None,
        },
    ];
    let raised = raise(&mut app, &input).await;
    let stored = app.store().crux(raised.id).await.unwrap().unwrap();
    assert_eq!(stored.findings().len(), 2);
    assert_eq!(stored.findings()[0].rule().as_str(), "cite.uncited-clause");
    assert!(stored.findings()[0].subject().is_some());
    assert!(stored.findings()[1].subject().is_none());

    let at = revision(&app).await;
    // two options recommended
    let mut two = ask(
        "Round halves",
        CruxKind::Ambiguity,
        vec![option("a"), option("b")],
    );
    for o in &mut two.options {
        o.recommended = Some(RecommendationInput {
            reason: None,
            confidence: None,
        });
    }
    let refused = app.raise_crux(&two).await.unwrap_err();
    assert!(
        matches!(&refused, AppError::Input(m) if m.contains("at most one")),
        "{refused:?}"
    );
    // a choice twice, no option at all, a confidence out of range, and a change request that is not there
    let twice = ask(
        "Round halves",
        CruxKind::Ambiguity,
        vec![option("a"), option("a")],
    );
    let err = app.raise_crux(&twice).await.unwrap_err();
    assert!(
        matches!(
            err,
            AppError::ChangeRequestOf {
                error: ChangeRequestError::DuplicateOption(_),
                ..
            }
        ),
        "{err:?}"
    );
    let none = ask("Round halves", CruxKind::Ambiguity, vec![]);
    let err = app.raise_crux(&none).await.unwrap_err();
    assert!(
        matches!(
            err,
            AppError::ChangeRequestOf {
                error: ChangeRequestError::NoOption,
                ..
            }
        ),
        "{err:?}"
    );
    let mut sure = ask("Round halves", CruxKind::Ambiguity, vec![option("a")]);
    sure.options[0].recommended = Some(RecommendationInput {
        reason: None,
        confidence: Some(101),
    });
    assert!(matches!(
        app.raise_crux(&sure).await,
        Err(AppError::Input(_))
    ));
    let elsewhere = ask("No such request", CruxKind::Ambiguity, vec![option("a")]);
    assert_eq!(
        app.raise_crux(&elsewhere).await.unwrap_err(),
        AppError::UnknownChangeRequest("No such request".into())
    );
    assert_eq!(revision(&app).await, at, "a refusal keeps nothing");
}

#[tokio::test]
async fn a_resolved_change_request_takes_no_crux_until_it_is_reopened() {
    let mut app = project().await;
    app.resolve_change_request("Round halves", &ResolutionInput::Done, "done", &alice())
        .await
        .unwrap();
    let at = revision(&app).await;
    let input = ask("Round halves", CruxKind::Ambiguity, vec![option("up")]);
    let err = app.raise_crux(&input).await.unwrap_err();
    assert!(
        matches!(
            err,
            AppError::ChangeRequestOf {
                error: ChangeRequestError::AlreadyResolved,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(codes(&app, &err).await, ["cr.already-resolved"]);
    assert_eq!(revision(&app).await, at);
    assert!(app.store().cruxes().await.unwrap().is_empty());

    app.reopen_change_request("Round halves", "it came back", &alice())
        .await
        .unwrap();
    raise(&mut app, &input).await;
    assert_eq!(app.store().cruxes().await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_targets_of_an_option_are_read_against_what_the_change_request_asks() {
    let mut app = project().await;
    let mut keeps = request("Keeps C1");
    keeps.keep = vec![clause("C1")];
    app.open_change_request(&keeps).await.unwrap();
    let at = revision(&app).await;

    // an option that asks the Clause to be revised, where the change request asks it to be kept
    let mut clash = ask("Keeps C1", CruxKind::Ambiguity, vec![option("reword")]);
    clash.options[0].targets = vec![clause("C1")];
    let err = app.raise_crux(&clash).await.unwrap_err();
    assert!(
        matches!(&err, AppError::Input(m) if m.contains("target and kept")),
        "{err:?}"
    );
    // a target given twice by one option
    let mut twice = ask("Keeps C1", CruxKind::Ambiguity, vec![option("reword")]);
    twice.options[0].targets = vec![clause("C2")];
    twice.options[0].keep = vec![clause("C2")];
    assert!(matches!(
        app.raise_crux(&twice).await,
        Err(AppError::Input(_))
    ));
    assert_eq!(revision(&app).await, at);

    // the same kind is no clash, and another Clause is none
    let mut fine = ask(
        "Keeps C1",
        CruxKind::Ambiguity,
        vec![option("keep it"), option("reword C2")],
    );
    fine.options[0].keep = vec![clause("C1")];
    fine.options[1].targets = vec![clause("C2")];
    let raised = raise(&mut app, &fine).await;
    let stored = app.store().crux(raised.id).await.unwrap().unwrap();
    assert_eq!(stored.options()[0].expects().len(), 1);
    assert_eq!(
        stored.options()[1].expects().values().next(),
        Some(&ExpectKind::Target)
    );
}

#[tokio::test]
async fn each_kind_of_decision_is_taken_once_and_the_option_chosen_asks_its_targets_of_the_change_request()
 {
    let mut app = project().await;
    let mut input = ask(
        "Round halves",
        CruxKind::Ambiguity,
        vec![option("up"), option("to even")],
    );
    input.options[1].targets = vec![clause("C1")];
    input.options[1].keep = vec![clause("C2")];
    let by_option = raise(&mut app, &input).await;
    let worded = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("x")]),
    )
    .await;
    let free = raise(
        &mut app,
        &ask("Round halves", CruxKind::Conflict, vec![option("x")]),
    )
    .await;
    let withdrawn = raise(
        &mut app,
        &ask("Round halves", CruxKind::Disagreement, vec![option("x")]),
    )
    .await;

    // nothing is asked of the targets until the option is chosen
    async fn cr(app: &Test) -> chidev_core::domain::entity::change_request::ChangeRequest {
        app.store().change_requests().await.unwrap().remove(0)
    }
    assert!(cr(&app).await.decided_expects().is_empty());

    decide(&mut app, &by_option.id.to_string(), &chosen(&by_option, 1))
        .await
        .unwrap();
    let asked = cr(&app).await;
    assert_eq!(asked.decided_expects().len(), 2);
    let kinds: Vec<ExpectKind> = asked.decided_expects().iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&ExpectKind::Target) && kinds.contains(&ExpectKind::Keep));
    let answer = app.store().crux(by_option.id).await.unwrap().unwrap();
    assert_eq!(
        answer.decision(),
        Some(&Decision::Chosen {
            option: by_option.options[1].id,
            reason: Some("fairer".try_into().unwrap()),
        })
    );
    let given = answer.answer_given().unwrap();
    assert_eq!(given.by().name().as_str(), "alice");
    assert_eq!(given.at(), Timestamp::from_millis(1_000));

    decide(
        &mut app,
        &worded.id.to_string(),
        &DecisionInput::Worded {
            wording: "round half to even".into(),
            reason: None,
        },
    )
    .await
    .unwrap();
    decide(
        &mut app,
        &free.id.to_string(),
        &DecisionInput::Unspecified {
            keep: vec!["a total is whole cents".into()],
        },
    )
    .await
    .unwrap();
    decide(
        &mut app,
        &withdrawn.id.to_string(),
        &DecisionInput::Withdrawn,
    )
    .await
    .unwrap();
    for (raised, expected) in [
        (&worded, "worded"),
        (&free, "unspecified"),
        (&withdrawn, "withdrawn"),
    ] {
        let stored = app.store().crux(raised.id).await.unwrap().unwrap();
        assert!(!stored.is_open());
        let word = match stored.decision().unwrap() {
            Decision::Chosen { .. } => "chosen",
            Decision::Worded { .. } => "worded",
            Decision::Unspecified { .. } => "unspecified",
            Decision::Withdrawn => "withdrawn",
        };
        assert_eq!(word, expected);
    }
    // the words, the freedom and the withdrawal ask nothing more of the targets
    assert_eq!(cr(&app).await.decided_expects(), asked.decided_expects());
}

#[tokio::test]
async fn an_option_chosen_that_clashes_with_what_an_earlier_decision_asked_is_refused_like_a_clash_at_raising()
 {
    let mut app = project().await;
    let mut revise = ask("Round halves", CruxKind::Ambiguity, vec![option("reword")]);
    revise.options[0].targets = vec![clause("C1")];
    let mut keep = ask("Round halves", CruxKind::Ambiguity, vec![option("leave")]);
    keep.options[0].keep = vec![clause("C1")];
    let first = raise(&mut app, &revise).await;
    let second = raise(&mut app, &keep).await;
    decide(&mut app, &first.id.to_string(), &chosen(&first, 0))
        .await
        .unwrap();
    let at = revision(&app).await;
    let err = decide(&mut app, &second.id.to_string(), &chosen(&second, 0))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::Input(m) if m.contains("target and kept")),
        "{err:?}"
    );
    assert_eq!(codes(&app, &err).await, ["input.invalid"]);
    assert_eq!(revision(&app).await, at);
    assert!(
        app.store()
            .crux(second.id)
            .await
            .unwrap()
            .unwrap()
            .is_open(),
        "the Crux stays open"
    );
    // the same kind is no clash: what the first recorded stays as it was recorded, and the third records its own
    let mut again = ask(
        "Round halves",
        CruxKind::Ambiguity,
        vec![option("reword too")],
    );
    again.options[0].targets = vec![clause("C1")];
    let third = raise(&mut app, &again).await;
    let recorded = app.store().change_requests().await.unwrap().remove(0);
    decide(&mut app, &third.id.to_string(), &chosen(&third, 0))
        .await
        .unwrap();
    let after = app.store().change_requests().await.unwrap().remove(0);
    assert_eq!(after.decided_expects().len(), 2);
    assert_eq!(after.decided_expects()[0], recorded.decided_expects()[0]);
    assert_eq!(
        after.decided_expects()[1].target,
        recorded.decided_expects()[0].target
    );
}

#[tokio::test]
async fn a_question_about_the_words_of_the_model_cannot_be_left_unspecified() {
    let mut app = project().await;
    let raised = raise(
        &mut app,
        &ask("Round halves", CruxKind::Model, vec![option("a Cart")]),
    )
    .await;
    let at = revision(&app).await;
    let err = decide(
        &mut app,
        &raised.id.to_string(),
        &DecisionInput::Unspecified { keep: vec![] },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            AppError::ChangeRequestOf {
                error: ChangeRequestError::UnspecifiedNotAllowed("model"),
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(revision(&app).await, at);
    // it can be worded or withdrawn
    decide(&mut app, &raised.id.to_string(), &DecisionInput::Withdrawn)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_decision_names_an_option_the_crux_has_and_a_crux_that_exists() {
    let mut app = project().await;
    let raised = raise(
        &mut app,
        &ask(
            "Round halves",
            CruxKind::Ambiguity,
            vec![option("up"), option("down")],
        ),
    )
    .await;
    let id = raised.id.to_string();
    let at = revision(&app).await;

    // an id that is an option of no Crux here
    let stray = DecisionInput::Chosen {
        option: Uuid::from_u128(9_999).to_string(),
        reason: None,
    };
    let err = decide(&mut app, &id, &stray).await.unwrap_err();
    assert!(
        matches!(
            err,
            AppError::ChangeRequestOf {
                error: ChangeRequestError::NotAnOption(_),
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(codes(&app, &err).await, ["cr.crux-unknown-option"]);
    // the text of a choice is not an id
    let text = DecisionInput::Chosen {
        option: "up".into(),
        reason: None,
    };
    assert!(matches!(
        decide(&mut app, &id, &text).await,
        Err(AppError::Input(_))
    ));
    // a Crux that does not exist, and an id that is not one
    let ghost = Uuid::from_u128(8_888).to_string();
    assert!(matches!(
        decide(&mut app, &ghost, &chosen(&raised, 0)).await,
        Err(AppError::Input(m)) if m.contains("no crux")
    ));
    assert!(matches!(
        decide(&mut app, "the first one", &chosen(&raised, 0)).await,
        Err(AppError::Input(_))
    ));
    // a line of the Policy the Project does not have
    let err = app
        .decide_crux(
            &id,
            &chosen(&raised, 0),
            &BasisInput::Policy("P1".into()),
            &alice(),
        )
        .await
        .unwrap_err();
    assert_eq!(codes(&app, &err).await, ["cr.policy-line-unknown"]);
    assert_eq!(revision(&app).await, at, "none of them kept anything");
    assert!(
        app.store()
            .crux(raised.id)
            .await
            .unwrap()
            .unwrap()
            .is_open()
    );
}

#[tokio::test]
async fn a_crux_is_decided_once_and_a_different_decision_is_a_new_crux_that_supersedes_it() {
    let mut app = project().await;
    let raised = raise(
        &mut app,
        &ask(
            "Round halves",
            CruxKind::Ambiguity,
            vec![option("up"), option("down")],
        ),
    )
    .await;
    let id = raised.id.to_string();
    decide(&mut app, &id, &chosen(&raised, 0)).await.unwrap();
    let at = revision(&app).await;

    let err = decide(&mut app, &id, &chosen(&raised, 1))
        .await
        .unwrap_err();
    assert_eq!(codes(&app, &err).await, ["cr.crux-answered"]);
    let suggestion = &app.diagnose(&err).await[0].suggestions[0];
    assert_eq!(suggestion.op["op"], "cr_crux_raise");
    assert_eq!(suggestion.op["supersedes"], id.as_str());
    assert_eq!(suggestion.op["in"], "Round halves");
    // even a withdrawal does not rewrite it
    let err = decide(&mut app, &id, &DecisionInput::Withdrawn)
        .await
        .unwrap_err();
    assert_eq!(codes(&app, &err).await, ["cr.crux-answered"]);
    assert_eq!(revision(&app).await, at);
    let stored = app.store().crux(raised.id).await.unwrap().unwrap();
    assert!(matches!(
        stored.decision(),
        Some(Decision::Chosen { option, .. }) if *option == raised.options[0].id
    ));

    // the new Crux replaces the decision, by id
    let mut again = ask("Round halves", CruxKind::Ambiguity, vec![option("down")]);
    again.supersedes = Some(id.clone());
    let newer = raise(&mut app, &again).await;
    let stored = app.store().crux(newer.id).await.unwrap().unwrap();
    assert_eq!(stored.supersedes(), Some(raised.id));
}

#[tokio::test]
async fn a_crux_supersedes_only_a_crux_that_exists_and_has_a_decision_to_replace() {
    let mut app = project().await;
    let open = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("a")]),
    )
    .await;
    let gone = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("a")]),
    )
    .await;
    decide(&mut app, &gone.id.to_string(), &DecisionInput::Withdrawn)
        .await
        .unwrap();
    let at = revision(&app).await;
    let supersede = |id: String| {
        let mut input = ask("Round halves", CruxKind::Ambiguity, vec![option("b")]);
        input.supersedes = Some(id);
        input
    };
    for (target, code) in [
        (open.id.to_string(), "cr.supersedes-unanswered"),
        (gone.id.to_string(), "cr.supersedes-unanswered"),
        (Uuid::from_u128(7_777).to_string(), "cr.supersedes-unknown"),
    ] {
        let err = app.raise_crux(&supersede(target)).await.unwrap_err();
        assert_eq!(codes(&app, &err).await, [code]);
    }
    assert!(matches!(
        app.raise_crux(&supersede("the first".into())).await,
        Err(AppError::Input(_))
    ));
    assert_eq!(revision(&app).await, at);
}

#[tokio::test]
async fn a_change_request_is_not_resolved_while_a_crux_raised_in_it_is_open_whatever_the_resolution()
 {
    let mut app = project().await;
    app.open_change_request(&request("Other")).await.unwrap();
    let raised = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("up")]),
    )
    .await;
    let at = revision(&app).await;
    for how in [
        ResolutionInput::Done,
        ResolutionInput::WontDo,
        ResolutionInput::Invalid,
        ResolutionInput::Duplicate("Other".into()),
        ResolutionInput::Superseded("Other".into()),
    ] {
        let err = app
            .resolve_change_request("Round halves", &how, "why", &alice())
            .await
            .unwrap_err();
        assert_eq!(codes(&app, &err).await, ["cr.crux-unanswered"], "{how:?}");
        // the way out is to decide the Crux, and the suggestion names it
        let suggestion = &app.diagnose(&err).await[0].suggestions[0];
        assert_eq!(suggestion.op["op"], "cr_crux_decide", "{how:?}");
        assert_eq!(suggestion.op["crux"], raised.id.to_string().as_str());
    }
    assert_eq!(revision(&app).await, at);
    // a request that has no open Crux is not held by another's
    app.resolve_change_request("Other", &ResolutionInput::WontDo, "not needed", &alice())
        .await
        .unwrap();
    // a withdrawal is a decision
    decide(&mut app, &raised.id.to_string(), &DecisionInput::Withdrawn)
        .await
        .unwrap();
    app.resolve_change_request(
        "Round halves",
        &ResolutionInput::WontDo,
        "not needed",
        &alice(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn status_lists_the_cruxes_by_id_and_the_open_ones_as_awaiting_a_decision() {
    let mut app = project().await;
    app.open_change_request(&request("Quiet")).await.unwrap();
    let mut input = ask(
        "Round halves",
        CruxKind::Ambiguity,
        vec![option("up"), option("to even")],
    );
    input.options[1].recommended = Some(RecommendationInput {
        reason: Some("the literal reading".into()),
        confidence: Some(85),
    });
    input.options[1].targets = vec![clause("C1")];
    let first = raise(&mut app, &input).await;
    let second = raise(
        &mut app,
        &ask("Round halves", CruxKind::Conflict, vec![option("x")]),
    )
    .await;

    let status = app.status().await.unwrap();
    assert_eq!(status.cruxes.len(), 2);
    let ids: Vec<&str> = status.cruxes.iter().map(|q| q.id.as_str()).collect();
    assert!(ids.contains(&first.id.to_string().as_str()));
    let round = status
        .change_requests
        .iter()
        .find(|c| c.title == "Round halves")
        .unwrap();
    assert_eq!(round.state, "open");
    let mut listed = round.cruxes.clone();
    listed.sort();
    let mut expected = vec![first.id.to_string(), second.id.to_string()];
    expected.sort();
    assert_eq!(listed, expected);
    let quiet = status
        .change_requests
        .iter()
        .find(|c| c.title == "Quiet")
        .unwrap();
    assert!(quiet.cruxes.is_empty());
    assert_eq!(quiet.state, "open");

    let told = status
        .cruxes
        .iter()
        .find(|q| q.id == first.id.to_string())
        .unwrap();
    assert_eq!(told.raised_in, "Round halves");
    assert_eq!(told.state, "open");
    assert!(told.answer.is_none());
    assert_eq!(told.options[1].id, first.options[1].id.to_string());
    assert_eq!(told.options[1].expects[0].kind, "target");
    assert_eq!(
        told.recommended.as_ref().map(|r| r.option.as_str()),
        Some(first.options[1].id.to_string().as_str())
    );

    assert_eq!(status.awaiting.len(), 2);
    for a in &status.awaiting {
        assert_eq!(a.raised_in, "Round halves");
        assert_eq!(a.question, "How is a half rounded?");
    }
    // the Crux the findings of `check` speak of
    let found = status.findings.iter().any(|f| {
        f.rule.as_str() == "cr.awaiting-decision" && f.message.contains(&first.id.to_string())
    });
    assert!(found, "{:?}", status.findings);

    // deciding one leaves the other awaited, and the request still open; both decided makes it decided
    decide(&mut app, &first.id.to_string(), &chosen(&first, 0))
        .await
        .unwrap();
    let status = app.status().await.unwrap();
    assert_eq!(status.awaiting.len(), 1);
    assert_eq!(status.awaiting[0].crux, second.id.to_string());
    assert_eq!(
        status
            .change_requests
            .iter()
            .find(|c| c.title == "Round halves")
            .unwrap()
            .state,
        "open"
    );
    let decided = status
        .cruxes
        .iter()
        .find(|q| q.id == first.id.to_string())
        .unwrap();
    assert_eq!(decided.state, "answered");
    let answer = decided.answer.as_ref().unwrap();
    assert!(matches!(
        &answer.decision,
        chidev_core::app::DecisionStatus::Chosen { option, reason }
            if *option == first.options[0].id.to_string() && reason.as_deref() == Some("fairer")
    ));
    assert_eq!(
        (answer.by_kind, answer.by.as_str(), answer.at),
        ("person", "alice", 1_000)
    );

    decide(&mut app, &second.id.to_string(), &DecisionInput::Withdrawn)
        .await
        .unwrap();
    let status = app.status().await.unwrap();
    assert!(status.awaiting.is_empty());
    assert_eq!(
        status
            .change_requests
            .iter()
            .find(|c| c.title == "Round halves")
            .unwrap()
            .state,
        "decided"
    );
}

/// The target of `title` as `RaiseCruxInput` would name it: one option, asking for the Clause `label` to be revised
/// (`kind` target) or kept (`kind` keep).
fn asking_of(cr: &str, option_name: &str, label: &str, kind: ExpectKind) -> RaiseCruxInput {
    let mut input = ask(cr, CruxKind::Ambiguity, vec![option(option_name)]);
    match kind {
        ExpectKind::Target => input.options[0].targets = vec![clause(label)],
        ExpectKind::Keep => input.options[0].keep = vec![clause(label)],
    }
    input
}

async fn request_titled(
    app: &Test,
    title: &str,
) -> chidev_core::domain::entity::change_request::ChangeRequest {
    app.store()
        .change_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.title().as_str() == title)
        .unwrap()
}

#[tokio::test]
async fn a_crux_that_supersedes_a_decision_in_the_same_request_takes_back_what_that_decision_asked()
{
    let mut app = project().await;
    // A decision asks for C1 to be revised.
    let first = raise(
        &mut app,
        &asking_of("Round halves", "reword C1", "C1", ExpectKind::Target),
    )
    .await;
    decide(&mut app, &first.id.to_string(), &chosen(&first, 0))
        .await
        .unwrap();
    let before = request_titled(&app, "Round halves").await;
    assert_eq!(before.decided_expects().len(), 1);

    // A Crux that does not supersede it cannot ask for C1 to be kept: refused when raised, as `cr_open` refuses it.
    let at = revision(&app).await;
    let clash = asking_of("Round halves", "leave C1", "C1", ExpectKind::Keep);
    let err = app.raise_crux(&clash).await.unwrap_err();
    assert!(
        matches!(&err, AppError::Input(m) if m.contains("target and kept")),
        "{err:?}"
    );
    assert_eq!(codes(&app, &err).await, ["input.invalid"]);
    // Nor can one that supersedes another decision than the one that asked.
    let other = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("up")]),
    )
    .await;
    decide(&mut app, &other.id.to_string(), &chosen(&other, 0))
        .await
        .unwrap();
    let mut wrong = clash.clone();
    wrong.supersedes = Some(other.id.to_string());
    let err = app.raise_crux(&wrong).await.unwrap_err();
    assert_eq!(codes(&app, &err).await, ["input.invalid"]);
    assert!(
        revision(&app).await > at,
        "the Crux `up` was raised and decided"
    );

    // One that supersedes the decision that asked for C1 may ask the opposite of it: that decision no longer stands.
    let mut keeps = clash.clone();
    keeps.supersedes = Some(first.id.to_string());
    let second = raise(&mut app, &keeps).await;
    // Raising changed nothing in the request; the old decision stands until the new one is taken.
    assert_eq!(
        request_titled(&app, "Round halves").await.decided_expects(),
        before.decided_expects()
    );
    decide(&mut app, &second.id.to_string(), &chosen(&second, 0))
        .await
        .unwrap();
    let after = request_titled(&app, "Round halves").await;
    let kept = app.store().crux(second.id).await.unwrap().unwrap();
    let answer = kept.answer_given().unwrap().id();
    assert_eq!(after.decided_expects().len(), 1);
    let held = &after.decided_expects()[0];
    assert_eq!(held.kind, ExpectKind::Keep);
    assert_eq!(held.answer, answer);
    // The store reads back whole: every decided expectation names an answer that asked for it.
    app.status().await.unwrap();
}

#[tokio::test]
async fn a_crux_that_supersedes_one_of_two_decisions_that_keep_a_target_leaves_the_keep_of_the_other()
 {
    let mut app = project().await;
    // A decision keeps C1.
    let first = raise(
        &mut app,
        &asking_of("Round halves", "leave C1", "C1", ExpectKind::Keep),
    )
    .await;
    decide(&mut app, &first.id.to_string(), &chosen(&first, 0))
        .await
        .unwrap();
    let kept_first = request_titled(&app, "Round halves").await;
    assert_eq!(kept_first.decided_expects().len(), 1);
    let first_revision = kept_first.decided_expects()[0].recorded;
    // C1 moves on, and an independent Crux keeps it as it is now.
    app.put_contract(
        "core",
        "rounding",
        "Rounding",
        &[
            ClauseInput {
                label: "C1".into(),
                text: "a half rounds to even".into(),
            },
            ClauseInput {
                label: "C2".into(),
                text: "a total is whole cents".into(),
            },
        ],
    )
    .await
    .unwrap();
    let second = raise(
        &mut app,
        &asking_of("Round halves", "leave C1 again", "C1", ExpectKind::Keep),
    )
    .await;
    decide(&mut app, &second.id.to_string(), &chosen(&second, 0))
        .await
        .unwrap();
    let both = request_titled(&app, "Round halves").await;
    assert_eq!(both.decided_expects().len(), 2);
    let second_answer = app
        .store()
        .crux(second.id)
        .await
        .unwrap()
        .unwrap()
        .answer_given()
        .unwrap()
        .id();
    let second_revision = both
        .decided_expects()
        .iter()
        .find(|d| d.answer == second_answer)
        .unwrap()
        .recorded;
    assert!(second_revision > first_revision);
    // The keep is judged against the earliest revision: C1 moved since the first decision.
    let status = app.status().await.unwrap();
    let request = status
        .change_requests
        .iter()
        .find(|c| c.title == "Round halves")
        .unwrap();
    assert_eq!(request.keep.len(), 1);
    assert!(request.keep[0].moved);

    // A third Crux supersedes the first decision and asks for C2 to be revised.
    let mut third = asking_of("Round halves", "reword C2", "C2", ExpectKind::Target);
    third.supersedes = Some(first.id.to_string());
    let third = raise(&mut app, &third).await;
    decide(&mut app, &third.id.to_string(), &chosen(&third, 0))
        .await
        .unwrap();
    let after = request_titled(&app, "Round halves").await;
    let keeps: Vec<_> = after
        .decided_expects()
        .iter()
        .filter(|d| d.kind == ExpectKind::Keep)
        .collect();
    // The second keep stands, with its own revision.
    assert_eq!(keeps.len(), 1);
    assert_eq!(keeps[0].answer, second_answer);
    assert_eq!(keeps[0].recorded, second_revision);
    assert_eq!(after.decided_expects().len(), 2);
    let status = app.status().await.unwrap();
    let request = status
        .change_requests
        .iter()
        .find(|c| c.title == "Round halves")
        .unwrap();
    assert_eq!(request.keep.len(), 1);
    assert!(!request.keep[0].moved);
    // The store reads back whole.
    app.status().await.unwrap();
}

#[tokio::test]
async fn a_crux_that_supersedes_a_decision_of_another_request_leaves_that_request_as_it_is() {
    let mut app = project().await;
    app.open_change_request(&request("Other")).await.unwrap();
    let first = raise(
        &mut app,
        &asking_of("Other", "reword C2", "C2", ExpectKind::Target),
    )
    .await;
    decide(&mut app, &first.id.to_string(), &chosen(&first, 0))
        .await
        .unwrap();
    let other_before = request_titled(&app, "Other").await;
    // Raised in `Round halves`, it replaces the decision taken in `Other`; what that decision asked stays there.
    let mut keeps = asking_of("Round halves", "leave C2", "C2", ExpectKind::Keep);
    keeps.supersedes = Some(first.id.to_string());
    let second = raise(&mut app, &keeps).await;
    decide(&mut app, &second.id.to_string(), &chosen(&second, 0))
        .await
        .unwrap();
    assert_eq!(
        request_titled(&app, "Other").await.decided_expects(),
        other_before.decided_expects()
    );
    let here = request_titled(&app, "Round halves").await;
    assert_eq!(here.decided_expects().len(), 1);
    let answer = app
        .store()
        .crux(second.id)
        .await
        .unwrap()
        .unwrap()
        .answer_given()
        .unwrap()
        .id();
    let held = &here.decided_expects()[0];
    assert_eq!((held.kind, held.answer), (ExpectKind::Keep, answer));
    app.status().await.unwrap();
}

#[tokio::test]
async fn a_decision_that_supersedes_one_and_is_refused_takes_nothing_back() {
    let mut app = project().await;
    // The request itself asks C2 to be revised.
    let mut asked = request("Asks C2");
    asked.targets = vec![clause("C2")];
    app.open_change_request(&asked).await.unwrap();
    let first = raise(
        &mut app,
        &asking_of("Asks C2", "reword C1", "C1", ExpectKind::Target),
    )
    .await;
    decide(&mut app, &first.id.to_string(), &chosen(&first, 0))
        .await
        .unwrap();
    // The replacement is refused at raising when it clashes with the request's own expectations.
    let mut clash = asking_of("Asks C2", "leave C2", "C2", ExpectKind::Keep);
    clash.supersedes = Some(first.id.to_string());
    let err = app.raise_crux(&clash).await.unwrap_err();
    assert_eq!(codes(&app, &err).await, ["input.invalid"]);
    assert_eq!(
        request_titled(&app, "Asks C2")
            .await
            .decided_expects()
            .len(),
        1
    );
}

#[tokio::test]
async fn the_suggestion_for_an_open_or_an_answered_crux_names_that_crux_only() {
    let mut app = project().await;
    let a = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("up")]),
    )
    .await;
    let b = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("down")]),
    )
    .await;
    let c = raise(
        &mut app,
        &ask("Round halves", CruxKind::Ambiguity, vec![option("even")]),
    )
    .await;
    // The request has three Cruxes open: resolving it names one, and the suggestion decides that one.
    let err = app
        .resolve_change_request("Round halves", &ResolutionInput::Done, "why", &alice())
        .await
        .unwrap_err();
    let told = &app.diagnose(&err).await[0];
    assert_eq!(told.code, "cr.crux-unanswered");
    assert_eq!(told.suggestions.len(), 1, "{:?}", told.suggestions);
    let named = told.suggestions[0].op["crux"].as_str().unwrap().to_string();
    assert!([&a, &b, &c].iter().any(|q| q.id.to_string() == named));

    // Two Cruxes answered: deciding the second again is refused, and the suggestion replaces that one, not the first.
    decide(&mut app, &a.id.to_string(), &chosen(&a, 0))
        .await
        .unwrap();
    decide(&mut app, &b.id.to_string(), &chosen(&b, 0))
        .await
        .unwrap();
    let err = decide(&mut app, &b.id.to_string(), &chosen(&b, 0))
        .await
        .unwrap_err();
    let told = &app.diagnose(&err).await[0];
    assert_eq!(told.code, "cr.crux-answered");
    assert_eq!(told.suggestions.len(), 1, "{:?}", told.suggestions);
    assert_eq!(told.suggestions[0].op["op"], "cr_crux_raise");
    assert_eq!(
        told.suggestions[0].op["supersedes"],
        b.id.to_string().as_str()
    );
    // A withdrawn Crux has no decision to replace: there is nothing to suggest.
    decide(&mut app, &c.id.to_string(), &DecisionInput::Withdrawn)
        .await
        .unwrap();
    let err = decide(&mut app, &c.id.to_string(), &chosen(&c, 0))
        .await
        .unwrap_err();
    let told = &app.diagnose(&err).await[0];
    assert_eq!(told.code, "cr.crux-answered");
    assert!(told.suggestions.is_empty(), "{:?}", told.suggestions);
}
