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
        Some(put.id.as_uuid())
    );
    let p = store.project().await.unwrap().unwrap();
    assert!(p.context(ctx).unwrap().contract(put.id).is_some());
}
