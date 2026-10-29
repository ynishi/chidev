//! Tests that store failures while gathering names for did_you_mean are reported as `names_unread` rather
//! than silently swallowed.

use std::sync::atomic::{AtomicBool, Ordering};

use chidev_core::app::commit::Gated;
use chidev_core::app::ports::{
    AssetKind, AssetNames, IdSource, JudgedReport, JudgedRepository, Store, StoreRevision,
};
use chidev_core::app::{
    App, AppError, CaseInput, ChangeRequestInput, ClauseInput, CruxKind, DiagValue, Intent,
    ItemInput, ItemRef, OptionInput, RaiseCruxInput,
};
use chidev_core::domain::entity::change_request::ChangeRequest;
use chidev_core::domain::entity::crux::Crux;
use chidev_core::domain::entity::measurement::Measurement;
use chidev_core::domain::entity::pin::Pin;
use chidev_core::domain::entity::project::Project;
use chidev_core::domain::repository::{
    ChangeRequestRepository, CruxRepository, PinRepository, ProjectRepository, StoreError,
};
use chidev_core::domain::vo::id::{ChangeRequestId, ContextId, CruxId, InterfaceId, PinId};
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

struct FlakyStore {
    inner: SqliteStore,
    fail_reads: AtomicBool,
}

impl FlakyStore {
    fn new(inner: SqliteStore) -> Self {
        Self {
            inner,
            fail_reads: AtomicBool::new(false),
        }
    }
}

impl ProjectRepository for FlakyStore {
    async fn project(&self) -> Result<Option<Project>, StoreError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(StoreError::Failed("cannot read project".to_string()));
        }
        self.inner.project().await
    }
}

impl PinRepository for FlakyStore {
    async fn pin_of(&self, interface: InterfaceId) -> Result<Option<Pin>, StoreError> {
        self.inner.pin_of(interface).await
    }

    async fn pins(&self) -> Result<Vec<Pin>, StoreError> {
        self.inner.pins().await
    }
}

impl ChangeRequestRepository for FlakyStore {
    async fn change_request(
        &self,
        id: ChangeRequestId,
    ) -> Result<Option<ChangeRequest>, StoreError> {
        self.inner.change_request(id).await
    }

    async fn change_requests(&self) -> Result<Vec<ChangeRequest>, StoreError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(StoreError::Failed("cannot read requests".to_string()));
        }
        self.inner.change_requests().await
    }
}

impl CruxRepository for FlakyStore {
    async fn crux(&self, id: CruxId) -> Result<Option<Crux>, StoreError> {
        self.inner.crux(id).await
    }

    async fn cruxes(&self) -> Result<Vec<Crux>, StoreError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(StoreError::Failed("cannot read cruxes".to_string()));
        }
        self.inner.cruxes().await
    }

    async fn cruxes_raised_in(&self, raised_in: ChangeRequestId) -> Result<Vec<Crux>, StoreError> {
        self.inner.cruxes_raised_in(raised_in).await
    }
}

impl AssetNames for FlakyStore {
    async fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        self.inner.asset_id(context, kind, name).await
    }

    async fn asset_name(&self, id: Uuid) -> Result<Option<String>, StoreError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(StoreError::Failed("cannot read asset name".to_string()));
        }
        self.inner.asset_name(id).await
    }
}

impl JudgedRepository for FlakyStore {
    async fn judged_reports(&self) -> Result<Vec<JudgedReport>, StoreError> {
        self.inner.judged_reports().await
    }
}

impl Store for FlakyStore {
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
        self.inner.revision().await
    }

    async fn model_revision(&self) -> Result<StoreRevision, StoreError> {
        self.inner.model_revision().await
    }

    async fn commit(&self, read_at: StoreRevision, gated: Gated) -> Result<(), StoreError> {
        self.inner.commit(read_at, gated).await
    }
}

#[tokio::test]
async fn near_names_unread_reported_when_store_fails() {
    let dir = std::env::temp_dir().join(format!("chidev-unread-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sqlite = SqliteStore::open(dir.join("chidev.db")).await.unwrap();
    let flaky = FlakyStore::new(sqlite);
    let mut app = App::new(
        flaky,
        Counter::default(),
        FixedClock(Timestamp::from_millis(1_000)),
    );

    app.init().await.unwrap();
    app.add_context("core", &[]).await.unwrap();
    app.put_contract(
        "core",
        "billing",
        "Billing",
        &[ClauseInput {
            label: "C1".to_string(),
            text: "text".to_string(),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    app.put_interface(
        "core",
        "api",
        "code",
        &[ItemInput {
            key: "item1".to_string(),
            lines: (1, 1),
            replaces: None,
        }],
    )
    .await
    .unwrap();
    app.put_harness(
        "core",
        "harness1",
        "code",
        &[CaseInput {
            name: "case1".to_string(),
            lines: (1, 1),
            items: vec![ItemRef {
                context: None,
                interface: "api".to_string(),
                key: "item1".to_string(),
            }],
            clauses: vec![],
        }],
    )
    .await
    .unwrap();
    app.open_change_request(&ChangeRequestInput {
        title: "Round halves".to_string(),
        situation: None,
        intent: Intent::ChangeBehavior,
        parent: None,
        issuer: None,
        targets: vec![],
        keep: vec![],
        assumptions: vec![],
        addresses: vec![],
    })
    .await
    .unwrap();
    let raised = app
        .raise_crux(&RaiseCruxInput {
            cr: "Round halves".to_string(),
            question: "How to round?".to_string(),
            kind: CruxKind::Ambiguity,
            options: vec![OptionInput {
                choice: "up".to_string(),
                targets: vec![],
                change: None,
                keep: vec![],
                consequence: None,
                recommended: None,
            }],
            supersedes: None,
            importance: None,
            findings: vec![],
        })
        .await
        .unwrap();

    let errors = [
        AppError::UnknownContext("cor".to_string()),
        AppError::UnknownAsset {
            context: "core".to_string(),
            kind: "contract",
            name: "billin".to_string(),
        },
        AppError::UnknownItem {
            context: "core".to_string(),
            interface: "api".to_string(),
            key: "item".to_string(),
        },
        AppError::UnknownClause {
            context: "core".to_string(),
            contract: "billing".to_string(),
            label: "C".to_string(),
        },
        AppError::UnknownCase {
            context: "core".to_string(),
            harness: "harness1".to_string(),
            name: "case".to_string(),
        },
        AppError::UnknownChangeRequest("Round halve".to_string()),
        AppError::UnknownCrux({
            let s = raised.value.id.to_string();
            let mut chars: Vec<char> = s.chars().collect();
            let last = chars.last_mut().unwrap();
            *last = if *last == '0' { '1' } else { '0' };
            chars.into_iter().collect()
        }),
    ];

    // 1. With reads succeeding: each has did_you_mean and no names_unread
    for err in &errors {
        let diags = app.diagnose(err).await;
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.code, "input.unknown-name");
        assert!(
            !d.values.contains_key("names_unread"),
            "unexpected names_unread for {err:?}"
        );
        assert!(
            !d.message.contains("could not be read"),
            "message has error suffix: {}",
            d.message
        );
        assert!(
            !d.did_you_mean.is_empty(),
            "expected did_you_mean for {err:?}"
        );
    }

    // 2. With reads failing: each has names_unread, empty did_you_mean, and the message suffix
    app.store().fail_reads.store(true, Ordering::SeqCst);
    for err in &errors {
        let diags = app.diagnose(err).await;
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.code, "input.unknown-name");
        assert!(
            d.values.contains_key("names_unread"),
            "missing names_unread for {err:?}"
        );
        let DiagValue::Text(ref unread) = d.values["names_unread"] else {
            panic!("names_unread not text");
        };
        assert!(
            unread.contains("cannot read"),
            "unexpected unread text: {unread}"
        );
        assert!(
            d.message
                .contains("the names that exist could not be read:"),
            "missing suffix in message: {}",
            d.message
        );
        assert!(
            d.did_you_mean.is_empty(),
            "expected empty did_you_mean when unread for {err:?}"
        );
    }
}
