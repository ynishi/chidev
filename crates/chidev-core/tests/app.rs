//! The use cases over the real store: each test opens an SQLite database of its own in a fresh temporary
//! directory, so no test double of the store is needed.

use chidev_core::app::ports::{AssetKind, AssetNames, IdSource};
use chidev_core::app::{App, AppError, ClauseInput};
use chidev_core::domain::repository::ProjectRepository;
use chidev_core::infra::sqlite::SqliteStore;
use uuid::Uuid;

/// Ids 1, 2, 3, ...
#[derive(Default)]
struct Counter(u128);

impl IdSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_u128(self.0)
    }
}

/// An App over a new database at a path no other test uses.
async fn app(name: &str) -> App<SqliteStore, Counter> {
    let dir = std::env::temp_dir().join(format!("chidev-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
    App::new(store, Counter::default())
}

#[tokio::test]
async fn a_project_is_made_once() {
    let mut app = app("init").await;
    let id = app.init().await.unwrap();
    assert_eq!(app.store().project().await.unwrap().unwrap().id(), id);
    assert_eq!(app.init().await, Err(AppError::AlreadyInitialized));
}

#[tokio::test]
async fn a_use_case_before_init_finds_no_project() {
    let mut app = app("no-project").await;
    assert_eq!(app.add_context("core", &[]).await, Err(AppError::NoProject));
    assert_eq!(app.status().await, Err(AppError::NoProject));
}

#[tokio::test]
async fn an_asset_and_its_name_are_kept_together() {
    let mut app = app("asset-name").await;
    app.init().await.unwrap();
    let ctx = app.add_context("core", &[]).await.unwrap();
    let put = app
        .put_contract(
            "core",
            "stack",
            "Stack",
            &[ClauseInput {
                label: "C1".to_string(),
                text: "push then pop".to_string(),
            }],
        )
        .await
        .unwrap();
    let store = app.store();
    assert_eq!(
        store
            .asset_id(ctx, AssetKind::Contract, "stack")
            .await
            .unwrap(),
        Some(put.value.id.as_uuid())
    );
    let p = store.project().await.unwrap().unwrap();
    assert!(p.context(ctx).unwrap().contract(put.value.id).is_some());
}

mod checks {
    use chidev_core::app::ports::{Invocation, Outcome};
    use chidev_core::app::{
        App, AppError, ChangeRequestInput, ClauseInput, ClauseRef, FindingInput, Intent, Registry,
    };
    use chidev_core::domain::check::{
        Check, CheckView, Descriptor, Executor, Finding, FindingState, Hook, Level, Params,
    };
    use chidev_core::domain::vo::finding::RuleId;
    use chidev_core::infra::sqlite::SqliteStore;

    use super::{Counter, app};

    /// A mandatory check that finds the Project whenever it has a Context, to try the refusal with.
    struct Forbidden;

    static FORBIDDEN: Descriptor = Descriptor {
        id: RuleId::from_static("test.forbidden"),
        summary: "a context exists",
        help: "only for the test",
        hooks: &[Hook::Put],
        default_level: Level::Error,
        mandatory: true,
        params: &[],
        executor: Executor::Builtin,
    };

    impl Check for Forbidden {
        fn descriptor(&self) -> &'static Descriptor {
            &FORBIDDEN
        }

        fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
            view.after
                .contexts()
                .map(|c| Finding::new(&FORBIDDEN, None, format!("context {}", c.name())))
                .collect()
        }
    }

    fn clauses() -> Vec<ClauseInput> {
        vec![ClauseInput {
            label: "C1".to_string(),
            text: "push then pop".to_string(),
        }]
    }

    async fn invocations(app: &App<SqliteStore, Counter>) -> Vec<Invocation> {
        app.store().invocations().await.unwrap()
    }

    #[tokio::test]
    async fn a_put_reports_what_the_checks_find_and_records_each_run() {
        let mut app = app("checks-put").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        let put = app
            .put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        let rules: Vec<&str> = put.findings.iter().map(|f| f.rule.as_str()).collect();
        assert!(rules.contains(&"cite.uncited-clause"), "{rules:?}");
        assert!(rules.contains(&"context.no-interface"));
        assert!(put.findings.iter().all(|f| f.state == FindingState::Open));
        let ran = invocations(&app).await;
        let uncited = ran
            .iter()
            .find(|i| i.check.as_str() == "cite.uncited-clause")
            .unwrap();
        assert_eq!(uncited.hook, Hook::Put);
        assert_eq!((uncited.warnings, uncited.outcome), (1, Outcome::Found));
        // A check with nothing to find is recorded too, as ok.
        let stale = ran
            .iter()
            .find(|i| i.check.as_str() == "cite.stale")
            .unwrap();
        assert_eq!((stale.warnings, stale.outcome), (0, Outcome::Ok));
    }

    #[tokio::test]
    async fn check_and_status_find_without_recording_or_keeping() {
        let mut app = app("checks-read").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        let before = invocations(&app).await.len();
        let revision = chidev_core::app::ports::Store::revision(app.store())
            .await
            .unwrap();
        let found = app.check().await.unwrap();
        assert!(
            found
                .iter()
                .any(|f| f.rule.as_str() == "cite.uncited-clause")
        );
        assert_eq!(app.status().await.unwrap().findings, found);
        assert_eq!(invocations(&app).await.len(), before);
        assert_eq!(
            chidev_core::app::ports::Store::revision(app.store())
                .await
                .unwrap(),
            revision
        );
    }

    #[tokio::test]
    async fn a_change_request_that_addresses_a_finding_turns_it_addressed_and_then_decided() {
        let mut app = app("checks-addressed").await;
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        app.put_contract("core", "stack", "Stack", &clauses())
            .await
            .unwrap();
        let open = |title: &str| ChangeRequestInput {
            title: title.to_string(),
            situation: None,
            intent: Intent::KeepBehavior,
            parent: None,
            issuer: None,
            items: Vec::new(),
            clauses: Vec::new(),
            crux: None,
            addresses: vec![FindingInput {
                rule: "cite.uncited-clause".to_string(),
                item: None,
                clause: Some(ClauseRef {
                    context: Some("core".to_string()),
                    contract: "stack".to_string(),
                    label: "C1".to_string(),
                }),
            }],
        };
        let state = |found: &[chidev_core::app::Found]| {
            found
                .iter()
                .find(|f| f.rule.as_str() == "cite.uncited-clause")
                .map(|f| f.state)
        };
        assert_eq!(state(&app.check().await.unwrap()), Some(FindingState::Open));
        app.open_change_request(&open("leave C1 unchecked"))
            .await
            .unwrap();
        assert_eq!(
            state(&app.check().await.unwrap()),
            Some(FindingState::Addressed)
        );
        app.resolve_change_request("leave C1 unchecked")
            .await
            .unwrap();
        assert_eq!(
            state(&app.check().await.unwrap()),
            Some(FindingState::Decided)
        );
        // Naming a rule that is no rule id is refused.
        let mut bad = open("bad");
        bad.addresses[0].rule = "Not A Rule".to_string();
        assert!(matches!(
            app.open_change_request(&bad).await,
            Err(AppError::Input(_))
        ));
    }

    #[tokio::test]
    async fn a_mandatory_check_at_error_refuses_the_use_case_and_nothing_is_kept() {
        let dir = std::env::temp_dir().join(format!("chidev-app-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
        let registry = Registry::new(&[&Forbidden], &[]).unwrap();
        let mut app = App::with_registry(store, Counter::default(), registry);
        app.init().await.unwrap();
        app.add_context("core", &[]).await.unwrap();
        let refused = app.put_contract("core", "stack", "Stack", &clauses()).await;
        let Err(AppError::Refused(found)) = refused else {
            panic!("expected a refusal, got {refused:?}");
        };
        assert_eq!(found[0].rule.as_str(), "test.forbidden");
        assert_eq!(found[0].level, Level::Error);
        use chidev_core::app::ports::AssetKind;
        use chidev_core::app::ports::AssetNames;
        let ctx = chidev_core::domain::repository::ProjectRepository::project(app.store())
            .await
            .unwrap()
            .unwrap()
            .contexts()
            .next()
            .unwrap()
            .id();
        assert_eq!(
            app.store()
                .asset_id(ctx, AssetKind::Contract, "stack")
                .await
                .unwrap(),
            None
        );
        assert!(invocations(&app).await.is_empty());
    }
}
