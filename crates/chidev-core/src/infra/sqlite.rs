//! The store in a SQLite file, through `rusqlite-isle`: the connection lives on a thread of its own and is reached
//! by async calls, so no blocking call runs on the caller's runtime. An async call cannot hold a transaction across
//! `.await`, so [`Store::commit`] writes everything of one use case in a single `call_write` closure, after
//! checking that nothing was committed since the use case began to read.
//!
//! The file holds `meta` (the store's revision, which every commit bumps), `project`, `pins`, `change_requests` and
//! `cruxes` (one JSON document each, in the schema the record names; see `persist`), `asset_names`, and
//! `check_invocations` (the checks each use case ran, appended and never changed; the column `hook` holds the
//! moment by its word, and `"trigger"` the use case that ran the check, by the word of its trigger), `judged_reports` and
//! `judged_findings` (what agents reported for the judged rules: one report per rule, replaced as a whole, and its
//! findings), `external_reports` (the latest result per external rule), `pin_codes` (the code each Pin
//! accepted), `measure_runs` and `measure_items` (the measurements, appended, and their mutants), and
//! `model_revision` (the store revision at which the model last changed). `PRAGMA user_version` is the layout of the
//! tables; a file with a higher one is refused. A file of layout 1 held the Crux inside its change request, and is
//! moved to layout 2 once, when it is opened (see `persist::layout2`). New tables, execution metadata and the trigger
//! on `check_invocations`, and `rule_digest` on `judged_reports` are added without a new layout number: they are
//! created `IF NOT EXISTS` (or added when missing) each time a file is opened, so a database made before them gets
//! them and nothing else in it changes. Old rule digests remain NULL: a current definition cannot establish the
//! definition that an earlier judgment used.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, params};
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver, IsleError};
use uuid::Uuid;

use super::persist::layout2::{self, CREATE_CRUXES};
use super::persist::measure::{ItemRow, RunRow};
use super::persist::{self, CURRENT, CURRENT_CHANGE_REQUEST, CURRENT_CRUX};
use crate::app::commit::{Gated, Trigger};
use crate::app::ports::external::{ExecutionEvidence, ExternalReport};
use crate::app::ports::{
    AssetKind, AssetNames, Changes, Invocation, InvocationSeq, JudgedFinding, JudgedReport,
    JudgedRepository, Outcome, Store, StoreRevision,
};
use crate::domain::check::level::MomentWordError;
use crate::domain::check::{Executor, Level, Moment};
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::crux::Crux;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{
    ChangeRequestRepository, CruxRepository, PinRepository, ProjectRepository, StoreError,
};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{RuleId, Subject, SubjectKind};
use crate::domain::vo::id::{ChangeRequestId, ContextId, CruxId, InterfaceId, PinId};
use crate::domain::vo::measure::MeasureKind;
use crate::domain::vo::name::AssetName;

/// The layout of the tables this code creates and reads. Layout 2 is layout 1 with the Cruxes in a table of their
/// own.
const LAYOUT: i64 = 2;

/// How long a statement waits for another process's lock before it fails.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const CREATE: &str = "
BEGIN;
CREATE TABLE IF NOT EXISTS meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL
);
INSERT OR IGNORE INTO meta (id, revision) VALUES (1, 0);
CREATE TABLE IF NOT EXISTS project (
    id TEXT PRIMARY KEY,
    schema INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pins (
    interface TEXT PRIMARY KEY,
    schema INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS change_requests (
    id TEXT PRIMARY KEY,
    schema INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS asset_names (
    context TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    id TEXT NOT NULL,
    PRIMARY KEY (context, kind, name)
);
CREATE INDEX IF NOT EXISTS asset_names_by_id ON asset_names (id);
PRAGMA user_version = 2;
COMMIT;
";

/// The record of the checks the use cases ran. Append-only: a row is written with the commit of the use case that
/// ran the check, and `store_revision` is the revision that use case read (the model the check saw). The column
/// is called `hook` for the moment, which a database made before the moments were named still has; it is the name
/// of a column and no word a person reads. A row with `put` there is an edit.
const CREATE_INVOCATIONS: &str = "
CREATE TABLE IF NOT EXISTS check_invocations (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    store_revision INTEGER NOT NULL,
    check_id TEXT NOT NULL,
    hook TEXT NOT NULL,
    notes INTEGER NOT NULL,
    warnings INTEGER NOT NULL,
    errors INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    executor TEXT NOT NULL DEFAULT 'builtin',
    judge TEXT,
    rule_digest TEXT,
    corrections TEXT
);
";

/// What agents reported for the judged rules. A report is replaced as a whole with the commit that makes it: its
/// rows in both tables are deleted and written again, so a rule has the findings of its latest report and no
/// others. The judge, `read_at` and the revision reported are of the report, so they are on its row; each finding
/// is joined to it by rule.
const CREATE_JUDGED: &str = "
CREATE TABLE IF NOT EXISTS judged_reports (
    rule TEXT PRIMARY KEY,
    judge TEXT NOT NULL,
    read_at INTEGER NOT NULL,
    reported_at INTEGER NOT NULL,
    on_older_revision INTEGER NOT NULL,
    scope TEXT NOT NULL,
    rule_digest TEXT
);
CREATE TABLE IF NOT EXISTS judged_findings (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    rule TEXT NOT NULL,
    subject_kind TEXT NOT NULL,
    subject_id TEXT,
    message TEXT NOT NULL,
    level TEXT,
    stamp TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS judged_findings_by_rule ON judged_findings (rule);
CREATE TABLE IF NOT EXISTS model_revision (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL
);
INSERT OR IGNORE INTO model_revision (id, revision) SELECT 1, revision FROM meta WHERE id = 1;
";

/// Latest external result per rule. The report and its invocation are committed with the change they checked.
const CREATE_EXTERNAL: &str = "
CREATE TABLE IF NOT EXISTS external_reports (
    rule TEXT PRIMARY KEY,
    doc TEXT NOT NULL
);
";

/// The code each Pin accepted (the latest per Interface), and the measurements: `measure_runs` (appended: the newest
/// per kind and subject is current, the older ones are history) with its `measure_items` (the mutants of a run),
/// written in the commit of the use case that made the run. A run is changed only by a correction of its texts,
/// which replaces its failure, evidence, summary and items and appends to its `corrections` (a JSON array, NULL
/// for none), and keeps its `seq`, its stamp and what it stood on.
const CREATE_MEASURE: &str = "
CREATE TABLE IF NOT EXISTS pin_codes (
    interface TEXT PRIMARY KEY,
    pin TEXT NOT NULL,
    code TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS measure_runs (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    stamp TEXT NOT NULL,
    inputs TEXT NOT NULL,
    store_revision INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    failure TEXT,
    evidence TEXT NOT NULL,
    summary TEXT NOT NULL,
    corrections TEXT
);
CREATE INDEX IF NOT EXISTS measure_runs_by_subject ON measure_runs (kind, subject_id, seq);
CREATE TABLE IF NOT EXISTS measure_items (
    run TEXT NOT NULL,
    seq INTEGER NOT NULL,
    key TEXT NOT NULL,
    site TEXT NOT NULL,
    operator TEXT NOT NULL,
    outcome TEXT NOT NULL,
    detail TEXT NOT NULL,
    PRIMARY KEY (run, seq)
);
";

/// Adds `column` to `table` when the table (made before the column existed) lacks it.
fn add_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), rusqlite::Error> {
    let present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        params![table, column],
        |r| r.get(0),
    )?;
    if present == 0 {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn failed(e: IsleError) -> StoreError {
    StoreError::Failed(e.to_string())
}

/// Makes the tables on a database that has none, and brings the tables of one of layout 2 up to date; leaves any
/// other alone (a file of layout 1 is moved by [`SqliteStore::open`], which then brings its tables up to date, and
/// the layout of any other is checked after opening).
fn prepare(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    // A page or a cell that a commit frees is overwritten with zeros, so a text a row held is not left in the free
    // space of the file after the row is replaced. It is a setting of the connection, so it is made for each one.
    conn.pragma_update(None, "secure_delete", 1)?;
    let layout: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if layout == 0 {
        conn.execute_batch(CREATE)?;
    }
    if layout == 0 || layout == LAYOUT {
        ensure_tables(conn)?;
    }
    Ok(())
}

/// Creates what a file of the layout lacks without the layout changing: the tables that were added later, and the
/// columns of the ones that grew.
fn ensure_tables(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(CREATE_CRUXES)?;
    conn.execute_batch(CREATE_INVOCATIONS)?;
    add_column(
        conn,
        "check_invocations",
        "executor",
        "TEXT NOT NULL DEFAULT 'builtin'",
    )?;
    add_column(conn, "check_invocations", "judge", "TEXT")?;
    add_column(conn, "check_invocations", "external", "TEXT")?;
    add_column(conn, "check_invocations", "rule_digest", "TEXT")?;
    add_column(conn, "check_invocations", "trigger", "TEXT")?;
    add_column(conn, "check_invocations", "corrections", "TEXT")?;
    conn.execute_batch(CREATE_JUDGED)?;
    add_column(conn, "judged_reports", "rule_digest", "TEXT")?;
    conn.execute_batch(CREATE_EXTERNAL)?;
    conn.execute_batch(CREATE_MEASURE)?;
    add_column(conn, "measure_runs", "corrections", "TEXT")?;
    Ok(())
}

/// The revision a stored number is; none for a negative one.
fn revision_of(n: i64) -> Option<StoreRevision> {
    u64::try_from(n).ok().map(StoreRevision)
}

/// An invocation as stored: seq, revision, check, moment, counts, outcome, executor, judge, external evidence, rule
/// digest, trigger, corrections.
type InvocationRow = (
    i64,
    i64,
    String,
    String,
    i64,
    i64,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// A stored document and the schema its record names.
type Row = (u32, String);

/// The store in a SQLite file.
pub struct SqliteStore {
    isle: AsyncIsle,
    driver: AsyncIsleDriver,
}

impl SqliteStore {
    /// Opens the database at `path`, making it (and the directory it is in) if it is not there. A database of layout
    /// 1 is moved to layout 2 first, once (see `persist::layout2`): if something it holds cannot be moved, opening
    /// fails with what did not match and the file is as it was. A database of a later layout is refused.
    pub async fn open(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
        let path = path.as_ref();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            // Once, before the connection exists; there is no runtime facility to wait for yet.
            std::fs::create_dir_all(dir)
                .map_err(|e| StoreError::Failed(format!("{}: {e}", dir.display())))?;
        }
        Self::connect(path, OpenFlags::default()).await
    }

    /// Opens the database at `path` only if the file is there, as [`SqliteStore::open`] does then. A missing file
    /// is refused, and neither it nor its directory is made: a command that only reads or measures a project must
    /// not make an empty store of a mistyped path, which `init` is for. The error does not repeat `path`, which the
    /// caller has; a path whose metadata cannot be read (no permission, say) is refused with the system's message,
    /// not as a missing file.
    pub async fn open_existing(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
        let path = path.as_ref();
        // Once, before the connection exists, as in `open`; the connection is opened without `SQLITE_OPEN_CREATE`
        // besides, so a file removed in between is refused rather than made empty.
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(StoreError::Failed("it is not a file".to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::Failed("there is no store file".to_string()));
            }
            Err(e) => return Err(StoreError::Failed(e.to_string())),
        }
        Self::connect(path, OpenFlags::default() - OpenFlags::SQLITE_OPEN_CREATE).await
    }

    /// Connects to the database file `path` with `flags` (with `SQLITE_OPEN_CREATE`, SQLite makes the file if it is
    /// not there, in a directory that is), makes or brings up to date its tables, and checks its layout.
    async fn connect(path: &Path, flags: OpenFlags) -> Result<SqliteStore, StoreError> {
        let (isle, driver) = AsyncIsle::builder()
            .busy_timeout(BUSY_TIMEOUT)
            .open_flags(flags)
            .spawn(path, prepare)
            .await
            .map_err(failed)?;
        let store = SqliteStore { isle, driver };
        let layout: i64 = store
            .isle
            .call(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
            .await
            .map_err(failed)?;
        if layout == 1 {
            let moved = store
                .isle
                .call(|c| match layout2::migrate(c) {
                    Ok(()) => {
                        ensure_tables(c)?;
                        Ok(Ok(()))
                    }
                    Err(layout2::Failure::Data(why)) => Ok(Err(why)),
                    Err(layout2::Failure::Store(e)) => Err(e),
                })
                .await;
            match moved {
                Ok(Ok(())) => {}
                Ok(Err(why)) => {
                    store.close().await?;
                    return Err(StoreError::Corrupt(format!(
                        "the store is of layout 1 and cannot be moved to layout 2: {why}"
                    )));
                }
                Err(e) => {
                    store.close().await?;
                    return Err(failed(e));
                }
            }
        }
        let layout: i64 = store
            .isle
            .call(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
            .await
            .map_err(failed)?;
        if layout != LAYOUT {
            let message =
                format!("the database is of layout {layout}; this version reads {LAYOUT}");
            store.close().await?;
            return Err(StoreError::Corrupt(message));
        }
        Ok(store)
    }

    /// Finishes what was asked of the connection and stops its thread. Dropping the store does the same, but
    /// cannot wait for it.
    pub async fn close(self) -> Result<(), StoreError> {
        let SqliteStore { isle, driver } = self;
        drop(isle);
        driver.shutdown().await.map_err(failed)
    }

    /// The checks the use cases have run, in the order they were kept, each with the `seq` of its row.
    async fn read_invocations(&self) -> Result<Vec<(InvocationSeq, Invocation)>, StoreError> {
        let rows: Vec<InvocationRow> = self
            .isle
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT seq, store_revision, check_id, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, \"trigger\", corrections
                     FROM check_invocations ORDER BY seq",
                )?;
                stmt.query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                        r.get(11)?,
                        r.get(12)?,
                        r.get(13)?,
                    ))
                })?
                .collect()
            })
            .await
            .map_err(failed)?;
        let corrupt = |what: &str| StoreError::Corrupt(format!("an invocation: {what}"));
        rows.into_iter()
            .map(
                |(
                    seq,
                    at,
                    check,
                    hook,
                    notes,
                    warnings,
                    errors,
                    outcome,
                    executor,
                    judge,
                    external,
                    rule_digest,
                    trigger,
                    corrections,
                )| {
                    let seq = InvocationSeq(u64::try_from(seq).map_err(|_| corrupt("a seq"))?);
                    Ok((
                        seq,
                        Invocation {
                            check: RuleId::try_from(check.as_str())
                                .map_err(|e| corrupt(&e.to_string()))?,
                            moment: match Moment::parse(&hook) {
                                Ok(m) => m,
                                // A row made when the word was `put` is an edit still.
                                Err(MomentWordError::Renamed { to, .. }) => to,
                                Err(_) => return Err(corrupt(&hook)),
                            },
                            read_at: StoreRevision(
                                u64::try_from(at).map_err(|_| corrupt("a revision"))?,
                            ),
                            notes: u32::try_from(notes).map_err(|_| corrupt("a count"))?,
                            warnings: u32::try_from(warnings).map_err(|_| corrupt("a count"))?,
                            errors: u32::try_from(errors).map_err(|_| corrupt("a count"))?,
                            outcome: Outcome::from_word(&outcome)
                                .ok_or_else(|| corrupt(&outcome))?,
                            executor: Executor::from_word(&executor)
                                .ok_or_else(|| corrupt(&executor))?,
                            judge,
                            trigger: trigger
                                .map(|w| Trigger::from_word(&w).ok_or_else(|| corrupt(&w)))
                                .transpose()?,
                            rule_digest: rule_digest
                                .map(|d| {
                                    Digest::restore(&d).ok_or_else(|| corrupt("a rule digest"))
                                })
                                .transpose()?,
                            external: external
                                .map(|doc| {
                                    serde_json::from_str::<ExecutionEvidence>(&doc)
                                        .map_err(|e| corrupt(&format!("external evidence: {e}")))
                                })
                                .transpose()?,
                            corrections: persist::measure::decode_invocation_corrections(
                                corrections.as_deref(),
                            )?,
                        },
                    ))
                },
            )
            .collect()
    }

    /// The measurements of the runs `filter` selects (a `WHERE` clause, or nothing for every run), in the order the runs
    /// were made, each with its items.
    ///
    /// `kind` is the one parameter of the filter (`?1`), a kind's word; none when the filter has no parameter.
    async fn runs_where(
        &self,
        filter: &'static str,
        kind: Option<String>,
    ) -> Result<Vec<Measurement>, StoreError> {
        let sql = format!(
            "SELECT id, kind, subject_kind, subject_id, stamp, inputs, started_at, outcome, failure, evidence, summary, corrections
             FROM measure_runs {filter} ORDER BY seq"
        );
        let runs: Vec<(RunRow, Vec<ItemRow>)> = self
            .isle
            .call(move |c| {
                let mut runs = c.prepare(&sql)?;
                let rows: Vec<RunRow> = runs
                    .query_map(rusqlite::params_from_iter(kind.iter()), |r| {
                        Ok(RunRow {
                            id: r.get(0)?,
                            kind: r.get(1)?,
                            subject_kind: r.get(2)?,
                            subject: r.get(3)?,
                            stamp: r.get(4)?,
                            inputs: r.get(5)?,
                            started_at: r.get(6)?,
                            outcome: r.get(7)?,
                            failure: r.get(8)?,
                            evidence: r.get(9)?,
                            summary: r.get(10)?,
                            corrections: r.get(11)?,
                        })
                    })?
                    .collect::<Result<_, _>>()?;
                let mut items = c.prepare(
                    "SELECT key, site, operator, outcome, detail FROM measure_items WHERE run = ?1 ORDER BY seq",
                )?;
                let mut out = Vec::with_capacity(rows.len());
                for run in rows {
                    let found: Vec<ItemRow> = items
                        .query_map(params![run.id], |r| {
                            Ok(ItemRow {
                                key: r.get(0)?,
                                site: r.get(1)?,
                                operator: r.get(2)?,
                                outcome: r.get(3)?,
                                detail: r.get(4)?,
                            })
                        })?
                        .collect::<Result<_, _>>()?;
                    out.push((run, found));
                }
                Ok(out)
            })
            .await
            .map_err(failed)?;
        runs.iter()
            .map(|(run, items)| persist::measure::decode(run, items))
            .collect()
    }

    /// The documents `sql` selects (`schema`, `doc`), with at most one text parameter.
    async fn documents(
        &self,
        sql: &'static str,
        param: Option<String>,
    ) -> Result<Vec<Row>, StoreError> {
        self.isle
            .call(move |c| {
                let mut stmt = c.prepare(sql)?;
                let map = |r: &rusqlite::Row<'_>| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?));
                match &param {
                    Some(p) => stmt.query_map(params![p], map)?.collect(),
                    None => stmt.query_map([], map)?.collect(),
                }
            })
            .await
            .map_err(failed)
    }
}

impl ProjectRepository for SqliteStore {
    async fn project(&self) -> Result<Option<Project>, StoreError> {
        let rows = self
            .documents("SELECT schema, doc FROM project LIMIT 1", None)
            .await?;
        rows.first()
            .map(|(schema, doc)| persist::decode_project(*schema, doc))
            .transpose()
    }
}

impl PinRepository for SqliteStore {
    async fn pin_of(&self, interface: InterfaceId) -> Result<Option<Pin>, StoreError> {
        let rows = self
            .documents(
                "SELECT schema, doc FROM pins WHERE interface = ?1",
                Some(interface.as_uuid().to_string()),
            )
            .await?;
        rows.first()
            .map(|(schema, doc)| persist::decode_pin(*schema, doc))
            .transpose()
    }

    async fn pins(&self) -> Result<Vec<Pin>, StoreError> {
        self.documents("SELECT schema, doc FROM pins ORDER BY interface", None)
            .await?
            .iter()
            .map(|(schema, doc)| persist::decode_pin(*schema, doc))
            .collect()
    }
}

impl ChangeRequestRepository for SqliteStore {
    async fn change_request(
        &self,
        id: ChangeRequestId,
    ) -> Result<Option<ChangeRequest>, StoreError> {
        let rows = self
            .documents(
                "SELECT schema, doc FROM change_requests WHERE id = ?1",
                Some(id.as_uuid().to_string()),
            )
            .await?;
        rows.first()
            .map(|(schema, doc)| persist::decode_change_request(*schema, doc))
            .transpose()
    }

    async fn change_requests(&self) -> Result<Vec<ChangeRequest>, StoreError> {
        self.documents("SELECT schema, doc FROM change_requests ORDER BY id", None)
            .await?
            .iter()
            .map(|(schema, doc)| persist::decode_change_request(*schema, doc))
            .collect()
    }
}

impl CruxRepository for SqliteStore {
    async fn crux(&self, id: CruxId) -> Result<Option<Crux>, StoreError> {
        let rows = self
            .documents(
                "SELECT schema, doc FROM cruxes WHERE id = ?1",
                Some(id.as_uuid().to_string()),
            )
            .await?;
        rows.first()
            .map(|(schema, doc)| persist::decode_crux(*schema, doc))
            .transpose()
    }

    async fn cruxes(&self) -> Result<Vec<Crux>, StoreError> {
        self.documents("SELECT schema, doc FROM cruxes ORDER BY id", None)
            .await?
            .iter()
            .map(|(schema, doc)| persist::decode_crux(*schema, doc))
            .collect()
    }

    async fn cruxes_raised_in(&self, raised_in: ChangeRequestId) -> Result<Vec<Crux>, StoreError> {
        self.documents(
            "SELECT schema, doc FROM cruxes WHERE raised_in = ?1 ORDER BY id",
            Some(raised_in.as_uuid().to_string()),
        )
        .await?
        .iter()
        .map(|(schema, doc)| persist::decode_crux(*schema, doc))
        .collect()
    }
}

impl AssetNames for SqliteStore {
    async fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &AssetName,
    ) -> Result<Option<Uuid>, StoreError> {
        let (context, kind, name) = (context.as_uuid().to_string(), kind.word(), name.to_string());
        let found: Option<String> = self
            .isle
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT id FROM asset_names WHERE context = ?1 AND kind = ?2 AND name = ?3",
                )?;
                let mut rows = stmt.query(params![context, kind, name])?;
                rows.next()?.map(|r| r.get(0)).transpose()
            })
            .await
            .map_err(failed)?;
        found
            .map(|id| {
                Uuid::parse_str(&id)
                    .map_err(|e| StoreError::Corrupt(format!("an asset id `{id}`: {e}")))
            })
            .transpose()
    }

    async fn asset_name(&self, id: Uuid) -> Result<Option<AssetName>, StoreError> {
        let id = id.to_string();
        let found: Option<String> = self
            .isle
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT name FROM asset_names WHERE id = ?1 ORDER BY context, kind LIMIT 1",
                )?;
                let mut rows = stmt.query(params![id])?;
                rows.next()?.map(|r| r.get(0)).transpose()
            })
            .await
            .map_err(failed)?;
        found
            .map(|name| {
                AssetName::try_from(name.as_str())
                    .map_err(|e| StoreError::Corrupt(format!("an asset name: {e}")))
            })
            .transpose()
    }
}

impl JudgedRepository for SqliteStore {
    async fn judged_reports(&self) -> Result<Vec<JudgedReport>, StoreError> {
        /// A report's row, with one of its findings (none for a report that found nothing).
        type Row = (
            (String, String, i64, i64, i64, String, Option<String>),
            Option<(String, Option<String>, String, Option<String>, String)>,
        );
        // One statement, so the reports and their findings are read from the same state of the file.
        let rows: Vec<Row> = self
            .isle
            .call(|c| {
                c.prepare(
                    "SELECT r.rule, r.judge, r.read_at, r.reported_at, r.on_older_revision, r.scope,
                            f.subject_kind, f.subject_id, f.message, f.level, f.stamp, r.rule_digest
                     FROM judged_reports r LEFT JOIN judged_findings f ON f.rule = r.rule
                     ORDER BY r.rule, f.seq",
                )?
                .query_map([], |r| {
                    let finding = match r.get::<_, Option<String>>(6)? {
                        Some(kind) => Some((kind, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?)),
                        None => None,
                    };
                    Ok((
                        (
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(11)?,
                        ),
                        finding,
                    ))
                })?
                .collect()
            })
            .await
            .map_err(failed)?;
        let corrupt = |what: &str| StoreError::Corrupt(format!("a judged report: {what}"));
        let revision = |n: i64| revision_of(n).ok_or_else(|| corrupt("a revision"));
        let mut out: Vec<JudgedReport> = Vec::new();
        for ((rule, judge, read_at, reported_at, older, scope, rule_digest), finding) in rows {
            let rule = RuleId::try_from(rule.as_str()).map_err(|e| corrupt(&e.to_string()))?;
            if out.last().is_none_or(|r| r.rule != rule) {
                out.push(JudgedReport {
                    rule,
                    judge,
                    read_at: revision(read_at)?,
                    reported_at: revision(reported_at)?,
                    on_older_revision: older != 0,
                    scope: Digest::restore(&scope).ok_or_else(|| corrupt("a scope"))?,
                    rule_digest: rule_digest
                        .map(|d| Digest::restore(&d).ok_or_else(|| corrupt("a rule digest")))
                        .transpose()?,
                    findings: Vec::new(),
                });
            }
            let (Some((kind, id, message, level, stamp)), Some(report)) = (finding, out.last_mut())
            else {
                continue;
            };
            let kind = SubjectKind::from_word(&kind).ok_or_else(|| corrupt(&kind))?;
            let subject = match id {
                None => None,
                Some(id) => Subject::from_parts(
                    kind,
                    Uuid::parse_str(&id).map_err(|_| corrupt("a subject id"))?,
                ),
            };
            report.findings.push(JudgedFinding {
                subject,
                message,
                level: level
                    .map(|w| Level::from_word(&w).ok_or_else(|| corrupt(&w)))
                    .transpose()?,
                stamp: Digest::restore(&stamp).ok_or_else(|| corrupt("a stamp"))?,
            });
        }
        Ok(out)
    }
}

impl Store for SqliteStore {
    async fn external_reports(&self) -> Result<Vec<ExternalReport>, StoreError> {
        let rows: Vec<(String, String)> = self
            .isle
            .call(|c| {
                let mut stmt = c.prepare("SELECT rule, doc FROM external_reports ORDER BY rule")?;
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .await
            .map_err(failed)?;
        rows.into_iter()
            .map(|(rule, doc)| {
                let report: ExternalReport = serde_json::from_str(&doc)
                    .map_err(|e| StoreError::Corrupt(format!("external report `{rule}`: {e}")))?;
                if report.rule != rule {
                    return Err(StoreError::Corrupt(format!(
                        "external report `{rule}` names `{}`",
                        report.rule
                    )));
                }
                Ok(report)
            })
            .collect()
    }

    async fn latest_measurements(&self) -> Result<Vec<Measurement>, StoreError> {
        self.runs_where(
            "WHERE seq IN (SELECT MAX(seq) FROM measure_runs GROUP BY kind, subject_id)",
            None,
        )
        .await
    }

    async fn latest_measurements_of(
        &self,
        kind: MeasureKind,
    ) -> Result<Vec<Measurement>, StoreError> {
        // only the rows of the kind are selected, and so only they are decoded
        self.runs_where(
            "WHERE kind = ?1 AND seq IN (SELECT MAX(seq) FROM measure_runs WHERE kind = ?1 GROUP BY subject_id)",
            Some(kind.word().to_string()),
        )
        .await
    }

    async fn measurements(&self) -> Result<Vec<Measurement>, StoreError> {
        self.runs_where("", None).await
    }

    async fn invocations(&self) -> Result<Vec<(InvocationSeq, Invocation)>, StoreError> {
        self.read_invocations().await
    }

    async fn clear_freed(&self) -> Result<(), StoreError> {
        // Outside any transaction: `VACUUM` cannot run inside one, and `call` opens none.
        self.isle
            .call(|c| c.execute_batch("VACUUM"))
            .await
            .map_err(|e| StoreError::Failed(format!("vacuum: {e}")))?;
        let wal = self
            .isle
            .call(|c| {
                let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
                if !mode.eq_ignore_ascii_case("wal") {
                    return Ok(None);
                }
                // The checkpoint answers a row (busy, log, checkpointed) and does not fail when a reader blocks
                // it: `busy` is 1 then, and the log still holds what was freed.
                c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                    Ok(Some((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    )))
                })
            })
            .await
            .map_err(|e| StoreError::Failed(format!("wal checkpoint: {e}")))?;
        match wal {
            Some((busy, log, checkpointed)) if busy != 0 => Err(StoreError::Failed(format!(
                "wal checkpoint: busy, a reader holds the log ({checkpointed} of {log} frames written back)"
            ))),
            _ => Ok(()),
        }
    }

    async fn pin_code(&self, pin: PinId) -> Result<Option<String>, StoreError> {
        let pin = pin.as_uuid().to_string();
        self.isle
            .call(move |c| {
                let mut stmt = c.prepare("SELECT code FROM pin_codes WHERE pin = ?1")?;
                let mut rows = stmt.query(params![pin])?;
                rows.next()?.map(|r| r.get(0)).transpose()
            })
            .await
            .map_err(failed)
    }

    async fn revision(&self) -> Result<StoreRevision, StoreError> {
        let number: i64 = self
            .isle
            .call(|c| c.query_row("SELECT revision FROM meta WHERE id = 1", [], |r| r.get(0)))
            .await
            .map_err(failed)?;
        u64::try_from(number)
            .map(StoreRevision)
            .map_err(|_| StoreError::Corrupt(format!("the store's revision is {number}")))
    }

    async fn model_revision(&self) -> Result<StoreRevision, StoreError> {
        let number: i64 = self
            .isle
            .call(|c| {
                c.query_row(
                    "SELECT revision FROM model_revision WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .map_err(failed)?;
        revision_of(number)
            .ok_or_else(|| StoreError::Corrupt(format!("the model's revision is {number}")))
    }

    async fn commit(&self, read_at: StoreRevision, gated: Gated) -> Result<(), StoreError> {
        self.write(read_at, gated.into_changes()).await
    }
}

impl SqliteStore {
    /// Keeps `changes` whole, if the store still stands at `read_at`. It is [`Store::commit`] without the proof that
    /// the checks were run, for the tests of what the store itself promises (a conflict keeps nothing, a record is
    /// appended, a report replaces): an inherent method of this type, which a use case, generic over [`Store`],
    /// cannot call.
    pub(crate) async fn write(
        &self,
        read_at: StoreRevision,
        changes: Changes,
    ) -> Result<(), StoreError> {
        let read_at = i64::try_from(read_at.0)
            .map_err(|_| StoreError::Failed("a revision too large to keep".to_string()))?;
        // Everything is written to text before the write lock is taken, so the lock is held only for the SQL.
        let project = changes
            .project
            .as_ref()
            .map(|p| {
                Ok::<_, StoreError>((p.id().as_uuid().to_string(), persist::encode_project(p)?))
            })
            .transpose()?;
        let pins = changes
            .pins
            .iter()
            .map(|p| Ok((p.interface().as_uuid().to_string(), persist::encode_pin(p)?)))
            .collect::<Result<Vec<_>, StoreError>>()?;
        let crs = changes
            .change_requests
            .iter()
            .map(|c| {
                Ok((
                    c.id().as_uuid().to_string(),
                    persist::encode_change_request(c)?,
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let cruxes = changes
            .cruxes
            .iter()
            .map(|q| {
                Ok((
                    q.id().as_uuid().to_string(),
                    q.raised_in().as_uuid().to_string(),
                    persist::encode_crux(q)?,
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let names = changes.names;
        let pin_codes = changes
            .pin_codes
            .iter()
            .map(|c| {
                (
                    c.interface.as_uuid().to_string(),
                    c.pin.as_uuid().to_string(),
                    c.code.clone(),
                )
            })
            .collect::<Vec<_>>();
        let measures = changes
            .measurements
            .iter()
            .map(persist::measure::encode)
            .collect::<Result<Vec<_>, StoreError>>()?;
        let corrected_runs = changes
            .corrected_measurements
            .iter()
            .map(persist::measure::encode)
            .collect::<Result<Vec<_>, StoreError>>()?;
        let external_text = |i: &Invocation| {
            i.external
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| StoreError::Failed(format!("external evidence: {e}")))
        };
        let corrected_rows = changes
            .corrected_invocations
            .iter()
            .map(|(seq, i)| {
                Ok((
                    i64::try_from(seq.0)
                        .map_err(|_| StoreError::Failed("a seq too large to keep".to_string()))?,
                    i.judge.clone(),
                    external_text(i)?,
                    persist::measure::encode_invocation_corrections(&i.corrections)?,
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let invocations = changes
            .invocations
            .iter()
            .map(|i| {
                Ok((
                    i64::try_from(i.read_at.0).map_err(|_| {
                        StoreError::Failed("a revision too large to keep".to_string())
                    })?,
                    i.check.to_string(),
                    // The stored word, not the word of the configuration: a row is read by every chidev that
                    // opens the file, 0.1.0 included, which knows `put` and not `edit`.
                    i.moment.digest_key(),
                    i64::from(i.notes),
                    i64::from(i.warnings),
                    i64::from(i.errors),
                    i.outcome.word(),
                    i.executor.word(),
                    i.judge.clone(),
                    external_text(i)?,
                    i.rule_digest.map(|d| d.to_string()),
                    i.trigger.map(|t| t.word()),
                    persist::measure::encode_invocation_corrections(&i.corrections)?,
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let external = changes
            .external
            .iter()
            .map(|report| {
                serde_json::to_string(report)
                    .map(|doc| (report.rule.clone(), doc))
                    .map_err(|e| StoreError::Failed(format!("external report: {e}")))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let judged = changes
            .judged
            .iter()
            .map(|r| {
                let number = |n: StoreRevision| {
                    i64::try_from(n.0)
                        .map_err(|_| StoreError::Failed("a revision too large to keep".to_string()))
                };
                let findings = r
                    .findings
                    .iter()
                    .map(|f| {
                        (
                            SubjectKind::of(f.subject).word(),
                            f.subject.map(|s| s.as_uuid().to_string()),
                            f.message.clone(),
                            f.level.map(|l| l.word()),
                            f.stamp.to_string(),
                        )
                    })
                    .collect::<Vec<_>>();
                Ok((
                    r.rule.to_string(),
                    r.judge.clone(),
                    number(r.read_at)?,
                    number(r.reported_at)?,
                    i64::from(r.on_older_revision),
                    r.scope.to_string(),
                    r.rule_digest.map(|d| d.to_string()),
                    findings,
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        // What an agent is shown is the Project, the change requests, the Cruxes and the names; a Pin is not in the
        // view.
        let model_changed =
            project.is_some() || !crs.is_empty() || !cruxes.is_empty() || !names.is_empty();
        let current = CURRENT;
        let current_change_request = CURRENT_CHANGE_REQUEST;
        let current_crux = CURRENT_CRUX;
        let kept = self
            .isle
            .call_write(move |tx| {
                let at: i64 = tx.query_row("SELECT revision FROM meta WHERE id = 1", [], |r| r.get(0))?;
                if at != read_at {
                    return Ok(false);
                }
                if let Some((id, doc)) = &project {
                    // The store has one Project.
                    tx.execute("DELETE FROM project WHERE id <> ?1", params![id])?;
                    tx.execute(
                        "INSERT INTO project (id, schema, doc) VALUES (?1, ?2, ?3)
                         ON CONFLICT (id) DO UPDATE SET schema = excluded.schema, doc = excluded.doc",
                        params![id, current, doc],
                    )?;
                }
                for (id, doc) in &pins {
                    tx.execute(
                        "INSERT INTO pins (interface, schema, doc) VALUES (?1, ?2, ?3)
                         ON CONFLICT (interface) DO UPDATE SET schema = excluded.schema, doc = excluded.doc",
                        params![id, current, doc],
                    )?;
                }
                for (interface, pin, code) in &pin_codes {
                    tx.execute(
                        "INSERT INTO pin_codes (interface, pin, code) VALUES (?1, ?2, ?3)
                         ON CONFLICT (interface) DO UPDATE SET pin = excluded.pin, code = excluded.code",
                        params![interface, pin, code],
                    )?;
                }
                let insert_items = |run: &RunRow, items: &[ItemRow]| -> Result<(), rusqlite::Error> {
                    for (n, item) in items.iter().enumerate() {
                        tx.execute(
                            "INSERT INTO measure_items (run, seq, key, site, operator, outcome, detail)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![
                                run.id, n as i64, item.key, item.site, item.operator, item.outcome,
                                item.detail
                            ],
                        )?;
                    }
                    Ok(())
                };
                for (run, items) in &measures {
                    tx.execute(
                        "INSERT INTO measure_runs
                         (id, kind, subject_kind, subject_id, stamp, inputs, store_revision, started_at, outcome, failure, evidence, summary, corrections)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                        params![
                            run.id, run.kind, run.subject_kind, run.subject, run.stamp, run.inputs,
                            read_at, run.started_at, run.outcome, run.failure, run.evidence,
                            run.summary, run.corrections
                        ],
                    )?;
                    insert_items(run, items)?;
                }
                // A correction replaces what a run says in text, and keeps the place of the run among the runs
                // (`seq`) and everything its stamp is read from.
                for (run, items) in &corrected_runs {
                    let changed = tx.execute(
                        "UPDATE measure_runs SET failure = ?2, evidence = ?3, summary = ?4, corrections = ?5
                         WHERE id = ?1",
                        params![run.id, run.failure, run.evidence, run.summary, run.corrections],
                    )?;
                    if changed == 0 {
                        return Err(rusqlite::Error::StatementChangedRows(0));
                    }
                    tx.execute("DELETE FROM measure_items WHERE run = ?1", params![run.id])?;
                    insert_items(run, items)?;
                }
                for (seq, judge, external, corrections) in &corrected_rows {
                    let changed = tx.execute(
                        "UPDATE check_invocations SET judge = ?2, external = ?3, corrections = ?4 WHERE seq = ?1",
                        params![seq, judge, external, corrections],
                    )?;
                    if changed == 0 {
                        return Err(rusqlite::Error::StatementChangedRows(0));
                    }
                }
                for (id, doc) in &crs {
                    tx.execute(
                        "INSERT INTO change_requests (id, schema, doc) VALUES (?1, ?2, ?3)
                         ON CONFLICT (id) DO UPDATE SET schema = excluded.schema, doc = excluded.doc",
                        params![id, current_change_request, doc],
                    )?;
                }
                for (id, raised_in, doc) in &cruxes {
                    tx.execute(
                        "INSERT INTO cruxes (id, raised_in, schema, doc) VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT (id) DO UPDATE SET raised_in = excluded.raised_in,
                           schema = excluded.schema, doc = excluded.doc",
                        params![id, raised_in, current_crux, doc],
                    )?;
                }
                for n in &names {
                    tx.execute(
                        "INSERT INTO asset_names (context, kind, name, id) VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT (context, kind, name) DO UPDATE SET id = excluded.id",
                        params![
                            n.context.as_uuid().to_string(),
                            n.kind.word(),
                            n.name.as_str(),
                            n.id.to_string()
                        ],
                    )?;
                }
                for (at, check, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, trigger, corrections) in
                    &invocations
                {
                    tx.execute(
                        "INSERT INTO check_invocations
                         (store_revision, check_id, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, \"trigger\", corrections)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                        params![at, check, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, trigger, corrections],
                    )?;
                }
                for (rule, doc) in &external {
                    tx.execute(
                        "INSERT INTO external_reports (rule, doc) VALUES (?1, ?2)
                         ON CONFLICT (rule) DO UPDATE SET doc = excluded.doc",
                        params![rule, doc],
                    )?;
                }
                for (rule, judge, read_at, reported_at, older, scope, rule_digest, findings) in &judged {
                    // A report replaces the rule's: its findings and its row go, and the new ones are written.
                    tx.execute("DELETE FROM judged_findings WHERE rule = ?1", params![rule])?;
                    tx.execute(
                        "INSERT INTO judged_reports
                         (rule, judge, read_at, reported_at, on_older_revision, scope, rule_digest)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT (rule) DO UPDATE SET judge = excluded.judge,
                           read_at = excluded.read_at, reported_at = excluded.reported_at,
                           on_older_revision = excluded.on_older_revision, scope = excluded.scope,
                           rule_digest = excluded.rule_digest",
                        params![rule, judge, read_at, reported_at, older, scope, rule_digest],
                    )?;
                    for (kind, id, message, level, stamp) in findings {
                        tx.execute(
                            "INSERT INTO judged_findings
                             (rule, subject_kind, subject_id, message, level, stamp)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![rule, kind, id, message, level, stamp],
                        )?;
                    }
                }
                tx.execute("UPDATE meta SET revision = revision + 1 WHERE id = 1", [])?;
                if model_changed {
                    tx.execute(
                        "UPDATE model_revision SET revision = (SELECT revision FROM meta WHERE id = 1)
                         WHERE id = 1",
                        [],
                    )?;
                }
                Ok(true)
            })
            .await
            .map_err(|e| match e {
                // A correction names a run or a row that the store does not have: nothing was kept.
                IsleError::Sqlite(rusqlite::Error::StatementChangedRows(0)) => StoreError::Corrupt(
                    "a correction names a record the store does not have".to_string(),
                ),
                other => failed(other),
            })?;
        if kept {
            Ok(())
        } else {
            Err(StoreError::Conflict)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ports::external::{ExternalStoredFinding, ExternalSubject};
    use crate::domain::entity::change_request::ChangeRequest;
    use crate::domain::vo::id::ClauseId;
    use crate::infra::persist::fixture;

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("chidev-sqlite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("chidev.db")
    }

    /// The runs kept, without the identity of their rows.
    async fn ran(store: &SqliteStore) -> Vec<Invocation> {
        store
            .invocations()
            .await
            .unwrap()
            .into_iter()
            .map(|(_, run)| run)
            .collect()
    }

    fn changes() -> Changes {
        let p = fixture::project();
        Changes {
            pins: vec![fixture::pin(&p)],
            change_requests: fixture::change_requests(&p),
            cruxes: fixture::cruxes(&p),
            // Every asset of the Project is named: a model whose asset has no name is not one a use case can word.
            names: [
                (AssetKind::Interface, "stack", fixture::IFACE),
                (AssetKind::Contract, "stack", 20),
                (AssetKind::Harness, "stack-tests", 40),
            ]
            .into_iter()
            .map(|(kind, name, id)| crate::app::ports::NamedAsset {
                context: ContextId::from_uuid(fixture::id(11)),
                kind,
                name: AssetName::try_from(name).unwrap(),
                id: fixture::id(id),
            })
            .collect(),
            project: Some(p),
            invocations: vec![Invocation {
                check: RuleId::from_static("cite.uncited-clause"),
                moment: Moment::Edit,
                trigger: None,
                read_at: StoreRevision(0),
                notes: 0,
                warnings: 2,
                errors: 0,
                outcome: Outcome::Found,
                executor: Executor::Builtin,
                judge: None,
                external: None,
                rule_digest: None,
                corrections: Vec::new(),
            }],
            judged: Vec::new(),
            external: Vec::new(),
            pin_codes: Vec::new(),
            measurements: Vec::new(),
            corrected_measurements: Vec::new(),
            corrected_invocations: Vec::new(),
        }
    }

    #[tokio::test]
    async fn what_was_committed_is_there_after_reopening() {
        let path = temp_db("reopen");
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.project().await.unwrap(), None);
        let at = store.revision().await.unwrap();
        let c = changes();
        store.write(at, c.clone()).await.unwrap();
        store.close().await.unwrap();

        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.revision().await.unwrap(), StoreRevision(at.0 + 1));
        assert_eq!(store.project().await.unwrap(), c.project);
        assert_eq!(store.pins().await.unwrap(), c.pins);
        assert_eq!(store.change_requests().await.unwrap(), c.change_requests);
        assert_eq!(store.cruxes().await.unwrap(), c.cruxes);
        for q in &c.cruxes {
            assert_eq!(store.crux(q.id()).await.unwrap().as_ref(), Some(q));
            assert!(
                store
                    .cruxes_raised_in(q.raised_in())
                    .await
                    .unwrap()
                    .contains(q)
            );
        }
        assert_eq!(
            store.pin_of(c.pins[0].interface()).await.unwrap().as_ref(),
            c.pins.first()
        );
        let n = &c.names[0];
        assert_eq!(
            store.asset_id(n.context, n.kind, &n.name).await.unwrap(),
            Some(n.id)
        );
        assert_eq!(store.asset_name(n.id).await.unwrap(), Some(n.name.clone()));
        assert_eq!(ran(&store).await, c.invocations);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn an_asset_of_the_model_with_no_name_is_a_store_that_does_not_agree_with_its_model_not_its_id()
     {
        use crate::app::{App, AppError};
        use crate::domain::vo::timestamp::Timestamp;
        use crate::infra::{clock::FixedClock, ids::UuidV7};

        let path = temp_db("unnamed-asset");
        let store = SqliteStore::open(&path).await.unwrap();
        let mut held = changes();
        held.names.clear();
        store.write(StoreRevision(0), held).await.unwrap();
        let app = App::new(store, UuidV7, FixedClock(Timestamp::from_millis(5)));
        let error = app.status().await.unwrap_err();
        assert!(
            matches!(&error, AppError::Store(StoreError::Corrupt(why)) if why.contains("has no name")),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_name_kept_that_breaks_the_rule_of_a_name_is_corrupt() {
        let path = temp_db("bad-asset-name");
        let store = SqliteStore::open(&path).await.unwrap();
        let c = changes();
        store.write(StoreRevision(0), c.clone()).await.unwrap();
        store.close().await.unwrap();
        let n = &c.names[0];
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE asset_names SET name = 'Has Space'", [])
            .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        let err = store.asset_name(n.id).await.unwrap_err();
        let StoreError::Corrupt(why) = err else {
            panic!("not corrupt: {err:?}");
        };
        assert!(why.contains("an asset name"), "{why}");
        assert!(why.contains("`H`"), "{why}");
        // a name that is not there is none, not corrupt
        assert_eq!(store.asset_name(Uuid::from_u128(999)).await.unwrap(), None);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn invocations_are_appended_and_a_refused_commit_appends_none() {
        let path = temp_db("invocations");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        store.write(at, changes()).await.unwrap();
        let again = store.revision().await.unwrap();
        store.write(again, changes()).await.unwrap();
        assert_eq!(store.invocations().await.unwrap().len(), 2);
        // Committed at an old revision: refused, so its record is not kept either.
        assert_eq!(store.write(at, changes()).await, Err(StoreError::Conflict));
        assert_eq!(store.invocations().await.unwrap().len(), 2);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_database_made_before_the_invocations_table_gets_it_on_opening() {
        let path = temp_db("before-invocations");
        SqliteStore::open(&path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("DROP TABLE check_invocations", []).unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(store.invocations().await.unwrap().is_empty());
        let at = store.revision().await.unwrap();
        store.write(at, changes()).await.unwrap();
        assert_eq!(store.invocations().await.unwrap().len(), 1);
        store.close().await.unwrap();
    }

    fn report(rule: &str, findings: usize) -> JudgedReport {
        JudgedReport {
            rule: RuleId::try_from(rule).unwrap(),
            judge: "agent".to_string(),
            read_at: StoreRevision(0),
            reported_at: StoreRevision(1),
            on_older_revision: false,
            scope: Digest::of(b"scope"),
            rule_digest: None,
            findings: (0..findings)
                .map(|n| JudgedFinding {
                    subject: (n % 2 == 0)
                        .then(|| Subject::Clause(ClauseId::from_uuid(fixture::id(50 + n as u128)))),
                    message: format!("finding {n}"),
                    level: (n == 1).then_some(Level::Note),
                    stamp: Digest::of(&[n as u8]),
                })
                .collect(),
        }
    }

    async fn commit_judged(store: &SqliteStore, reports: Vec<JudgedReport>) {
        let at = store.revision().await.unwrap();
        let changes = Changes {
            judged: reports,
            ..Changes::default()
        };
        store.write(at, changes).await.unwrap();
    }

    #[tokio::test]
    async fn a_judged_report_replaces_the_rules_earlier_one_and_survives_reopening() {
        let path = temp_db("judged");
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(store.judged_reports().await.unwrap().is_empty());
        let (a, b) = (report("judged.a", 3), report("judged.b", 0));
        commit_judged(&store, vec![a.clone(), b.clone()]).await;
        assert_eq!(store.judged_reports().await.unwrap(), vec![a, b.clone()]);
        // judged again: only the new findings are there; a report with none is kept
        let a2 = report("judged.a", 1);
        commit_judged(&store, vec![a2.clone()]).await;
        store.close().await.unwrap();
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.judged_reports().await.unwrap(), vec![a2, b]);
        store.close().await.unwrap();
    }

    fn judged_invocation(report: &JudgedReport) -> Invocation {
        let mut invocation = changes().invocations.remove(0);
        invocation.check = report.rule.clone();
        invocation.executor = Executor::Judged;
        invocation.judge = Some(report.judge.clone());
        invocation.rule_digest = report.rule_digest;
        invocation
    }

    #[tokio::test]
    async fn rule_versions_roundtrip_in_reports_and_append_only_invocations() {
        let path = temp_db("rule-versions");
        let store = SqliteStore::open(&path).await.unwrap();
        store.write(StoreRevision(0), changes()).await.unwrap();
        let mut first = report("judged.a", 1);
        first.rule_digest = Some(Digest::of(b"rule v1"));
        let first_run = judged_invocation(&first);
        let mut unknown = report("judged.unknown", 0);
        unknown.rule_digest = None;
        store
            .write(
                StoreRevision(1),
                Changes {
                    judged: vec![first.clone(), unknown.clone()],
                    invocations: vec![first_run.clone()],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(1));
        let mut second = report("judged.a", 0);
        second.rule_digest = Some(Digest::of(b"rule v2"));
        let second_run = judged_invocation(&second);
        store
            .write(
                StoreRevision(2),
                Changes {
                    judged: vec![second.clone()],
                    invocations: vec![second_run.clone()],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(1));
        store.close().await.unwrap();
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.judged_reports().await.unwrap(), vec![second, unknown]);
        let invocations = ran(&store).await;
        assert_eq!(invocations[0].rule_digest, None);
        assert_eq!(invocations[1..], [first_run, second_run]);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn old_databases_keep_rule_versions_unknown_without_backfilling() {
        let path = temp_db("before-rule-versions");
        let store = SqliteStore::open(&path).await.unwrap();
        let mut old_report = report("judged.a", 1);
        old_report.rule_digest = Some(Digest::of(b"irrecoverable old definition"));
        let old_run = judged_invocation(&old_report);
        store
            .write(
                StoreRevision(0),
                Changes {
                    judged: vec![old_report.clone()],
                    invocations: vec![old_run.clone()],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "ALTER TABLE judged_reports DROP COLUMN rule_digest;
            ALTER TABLE check_invocations DROP COLUMN rule_digest;",
        )
        .unwrap();
        drop(conn);
        old_report.rule_digest = None;
        let mut old_run = old_run;
        old_run.rule_digest = None;
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.judged_reports().await.unwrap(), vec![old_report]);
        assert_eq!(ran(&store).await, vec![old_run]);
        assert_eq!(store.revision().await.unwrap(), StoreRevision(1));
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(0));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_stored_rule_digests_are_corruption_not_unknown_versions() {
        for (name, table) in [
            ("report", "judged_reports"),
            ("invocation", "check_invocations"),
        ] {
            let path = temp_db(&format!("bad-rule-version-{name}"));
            let store = SqliteStore::open(&path).await.unwrap();
            let report = report("judged.a", 0);
            store
                .write(
                    StoreRevision(0),
                    Changes {
                        invocations: vec![judged_invocation(&report)],
                        judged: vec![report],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();
            store.close().await.unwrap();
            let conn = Connection::open(&path).unwrap();
            conn.execute(
                &format!("UPDATE {table} SET rule_digest = 'not-a-digest'"),
                [],
            )
            .unwrap();
            drop(conn);
            let store = SqliteStore::open(&path).await.unwrap();
            let corrupt = if name == "report" {
                matches!(store.judged_reports().await, Err(StoreError::Corrupt(_)))
            } else {
                matches!(store.invocations().await, Err(StoreError::Corrupt(_)))
            };
            assert!(corrupt, "{table}");
            store.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_conflicting_report_keeps_neither_the_new_version_nor_its_invocation() {
        let path = temp_db("rule-version-conflict");
        let store = SqliteStore::open(&path).await.unwrap();
        let mut first = report("judged.a", 1);
        first.rule_digest = Some(Digest::of(b"v1"));
        let first_run = judged_invocation(&first);
        store
            .write(
                StoreRevision(0),
                Changes {
                    invocations: vec![first_run.clone()],
                    judged: vec![first.clone()],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        let mut second = report("judged.a", 0);
        second.rule_digest = Some(Digest::of(b"v2"));
        assert_eq!(
            store
                .write(
                    StoreRevision(0),
                    Changes {
                        invocations: vec![judged_invocation(&second)],
                        judged: vec![second],
                        ..Changes::default()
                    }
                )
                .await,
            Err(StoreError::Conflict)
        );
        assert_eq!(store.judged_reports().await.unwrap(), vec![first]);
        assert_eq!(ran(&store).await, vec![first_run]);
        assert_eq!(store.revision().await.unwrap(), StoreRevision(1));
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(0));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn the_models_revision_moves_with_the_model_and_not_with_a_run_or_a_report() {
        let path = temp_db("model-revision");
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(0));
        commit_judged(&store, vec![report("judged.a", 1)]).await;
        assert_eq!(store.revision().await.unwrap(), StoreRevision(1));
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(0));
        let at = store.revision().await.unwrap();
        let pin_only = Changes {
            pins: changes().pins,
            ..Changes::default()
        };
        store.write(at, pin_only).await.unwrap();
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(0));
        let at = store.revision().await.unwrap();
        store.write(at, changes()).await.unwrap();
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(3));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_database_made_before_the_judged_tables_gets_them_and_keeps_its_invocations() {
        let path = temp_db("before-judged");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        store.write(at, changes()).await.unwrap();
        store.close().await.unwrap();
        // the tables as the previous version made them
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE judged_reports; DROP TABLE judged_findings; DROP TABLE model_revision;
             ALTER TABLE check_invocations DROP COLUMN executor;
             ALTER TABLE check_invocations DROP COLUMN judge;",
        )
        .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(store.judged_reports().await.unwrap().is_empty());
        // the model is taken to have last changed at the store's revision, the safe side
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(1));
        let kept = ran(&store).await;
        assert_eq!(kept.len(), 1);
        assert_eq!(
            (kept[0].executor, kept[0].judge.clone()),
            (Executor::Builtin, None)
        );
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_database_before_external_checks_keeps_its_invocations_and_gets_the_new_storage() {
        let path = temp_db("before-external");
        let store = SqliteStore::open(&path).await.unwrap();
        store.write(StoreRevision(0), changes()).await.unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE external_reports;
             ALTER TABLE check_invocations DROP COLUMN external;",
        )
        .unwrap();
        drop(conn);

        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.revision().await.unwrap(), StoreRevision(1));
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(1));
        assert!(store.external_reports().await.unwrap().is_empty());
        let invocations = ran(&store).await;
        assert_eq!(invocations, changes().invocations);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn an_external_report_with_invalid_json_is_corrupt() {
        let path = temp_db("external-corrupt");
        SqliteStore::open(&path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO external_reports (rule, doc) VALUES ('ext.a', '{')",
            [],
        )
        .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(matches!(
            store.external_reports().await,
            Err(StoreError::Corrupt(_))
        ));
        store.close().await.unwrap();
    }

    fn external_report(rule: &str, failure: Option<&str>) -> ExternalReport {
        ExternalReport {
            rule: rule.into(),
            input_stamp: Digest::of(b"input").to_string(),
            config_stamp: Digest::of(b"config").to_string(),
            findings: vec![ExternalStoredFinding {
                subject: Some(ExternalSubject {
                    kind: "clause".into(),
                    id: fixture::id(50).to_string(),
                }),
                message: "the clause contradicts its example".into(),
                level: "warning".into(),
            }],
            evidence: ExecutionEvidence {
                command: Some("/tools/check-contract".into()),
                command_digest: Some(Digest::of(b"program").to_string()),
                config_digest: Digest::of(b"process").to_string(),
                exit_code: Some(0),
                stderr: "a diagnostic".into(),
            },
            failure: failure.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn external_reports_replace_their_rule_and_evidence_survives_reopening() {
        let path = temp_db("external-reopen");
        let store = SqliteStore::open(&path).await.unwrap();
        let first = external_report("ext.a", Some("timeout"));
        let second = external_report("ext.b", None);
        let mut first_changes = changes();
        first_changes.external = vec![first, second.clone()];
        first_changes.invocations[0].check = RuleId::from_static("ext.b");
        first_changes.invocations[0].executor = Executor::External;
        first_changes.invocations[0].external = Some(second.evidence.clone());
        store
            .write(StoreRevision(0), first_changes.clone())
            .await
            .unwrap();
        let mut replacement = external_report("ext.a", None);
        replacement.findings.clear();
        store
            .write(
                StoreRevision(1),
                Changes {
                    external: vec![replacement.clone()],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        // Keeping an execution result does not change the model it checked.
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(1));
        assert_eq!(store.revision().await.unwrap(), StoreRevision(2));
        store.close().await.unwrap();

        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(
            store.external_reports().await.unwrap(),
            vec![replacement, second]
        );
        assert_eq!(ran(&store).await, first_changes.invocations);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_conflicting_commit_keeps_neither_external_reports_nor_evidence() {
        let path = temp_db("external-conflict");
        let store = SqliteStore::open(&path).await.unwrap();
        let first = external_report("ext.a", None);
        store
            .write(
                StoreRevision(0),
                Changes {
                    external: vec![first.clone()],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        let mut stale = changes();
        stale.external = vec![external_report("ext.b", Some("exit"))];
        stale.invocations[0].external = Some(first.evidence.clone());
        assert_eq!(
            store.write(StoreRevision(0), stale).await,
            Err(StoreError::Conflict)
        );
        assert_eq!(store.external_reports().await.unwrap(), vec![first]);
        assert!(store.invocations().await.unwrap().is_empty());
        assert!(store.project().await.unwrap().is_none());
        assert_eq!(store.model_revision().await.unwrap(), StoreRevision(0));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn an_external_report_whose_rule_disagrees_with_its_record_is_corrupt() {
        let path = temp_db("external-rule-corrupt");
        let store = SqliteStore::open(&path).await.unwrap();
        store
            .write(
                StoreRevision(0),
                Changes {
                    external: vec![external_report("ext.a", None)],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE external_reports SET rule = 'ext.b'", [])
            .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(matches!(
            store.external_reports().await,
            Err(StoreError::Corrupt(_))
        ));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_commit_after_another_is_refused_and_writes_nothing() {
        let path = temp_db("stale");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        store.write(at, Changes::default()).await.unwrap();
        let stale = store.write(at, changes()).await;
        assert_eq!(stale, Err(StoreError::Conflict));
        assert_eq!(store.project().await.unwrap(), None);
        assert!(store.pins().await.unwrap().is_empty());
        assert!(store.change_requests().await.unwrap().is_empty());
        assert_eq!(store.revision().await.unwrap(), StoreRevision(at.0 + 1));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_database_of_a_later_layout_is_refused() {
        let path = temp_db("later");
        SqliteStore::open(&path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        drop(conn);
        assert!(matches!(
            SqliteStore::open(&path).await.map(|_| ()),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[tokio::test]
    async fn a_document_of_a_later_schema_is_corrupt() {
        let path = temp_db("schema");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        store.write(at, changes()).await.unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE project SET schema = 3", []).unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(matches!(store.project().await, Err(StoreError::Corrupt(_))));
        store.close().await.unwrap();
    }

    /// The correction of texts: what the store keeps of it, and what it leaves in the files.
    mod corrections {
        use std::collections::BTreeMap;
        use std::path::Path;

        use super::*;
        use crate::app::ports::InvocationField;
        use crate::domain::entity::change_request::{ChangeRequestDraft, ChangeRequestField};
        use crate::domain::entity::measurement::MeasurementField;
        use crate::domain::vo::answer::DecisionMaker;
        use crate::domain::vo::correction::{Correctable, Correction, SoughtText};
        use crate::domain::vo::intent::Intent;
        use crate::domain::vo::measure::StepEvidence;
        use crate::domain::vo::prose::Prose;
        use crate::domain::vo::timestamp::Timestamp;
        use crate::domain::vo::title::Title;

        const TOKEN: &str = "tok-SECRET-123";

        fn sought() -> SoughtText {
            SoughtText::try_from(TOKEN).unwrap()
        }

        fn correction<F: PartialEq + std::fmt::Display>(fields: Vec<F>) -> Correction<F> {
            Correction::new(
                fields,
                Prose::try_from("a token was pasted").unwrap(),
                DecisionMaker::person("carol").unwrap(),
                Timestamp::from_millis(50),
            )
            .unwrap()
        }

        /// A change request with the token in its situation.
        fn leaky_cr() -> ChangeRequest {
            ChangeRequest::open(
                ChangeRequestId::from_uuid(fixture::id(900)),
                ChangeRequestDraft {
                    title: Title::try_from("a request").unwrap(),
                    situation: Some(Prose::try_from(format!("it holds {TOKEN}").as_str()).unwrap()),
                    intent: Intent::KeepBehavior,
                    parent: None,
                    issuer: None,
                    expects: BTreeMap::new(),
                    assumptions: Vec::new(),
                    addresses: Vec::new(),
                },
            )
        }

        /// A run of a measure with the token on the standard error of its step.
        fn leaky_run(n: u128, subject: u128) -> Measurement {
            use crate::domain::entity::measurement::fixture::{draft, empty_basis};
            let mut d = draft(
                n,
                InterfaceId::from_uuid(fixture::id(subject)),
                Digest::of(b"code"),
                empty_basis(),
                Vec::new(),
            );
            d.evidence = vec![StepEvidence {
                step: "baseline-test".into(),
                command: "cargo test".into(),
                command_digest: None,
                exit_code: Some(1),
                duration_ms: 3,
                stderr: format!("error: bad {TOKEN}"),
            }];
            Measurement::new(d).unwrap()
        }

        /// A record of a run of an external check with the token in its command, its stderr and its judge.
        fn leaky_invocation() -> Invocation {
            let mut run = changes().invocations.remove(0);
            run.executor = Executor::External;
            run.judge = Some(format!("judge {TOKEN}"));
            run.external = Some(ExecutionEvidence {
                command: Some(format!("/bin/check {TOKEN}")),
                stderr: format!("error {TOKEN}"),
                ..ExecutionEvidence::default()
            });
            run
        }

        /// The same record with the token replaced in all three places, as a correction does.
        fn corrected_invocation(run: &Invocation) -> Invocation {
            let mut fixed = run.clone();
            fixed
                .correct(
                    &sought(),
                    correction(vec![
                        InvocationField::Command,
                        InvocationField::Stderr,
                        InvocationField::Judge,
                    ]),
                    &|_, _| false,
                )
                .unwrap();
            fixed
        }

        /// The files the store keeps: the database and, when they exist, its journal and its write-ahead log.
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

        /// What a correction of the change request, the run of a measure and the record of a check's run puts in
        /// `Changes`.
        fn correcting(
            cr: &ChangeRequest,
            run: &Measurement,
            seq: InvocationSeq,
            check: &Invocation,
        ) -> Changes {
            let mut fixed_cr = cr.clone();
            fixed_cr
                .correct(
                    &sought(),
                    correction(vec![ChangeRequestField::Situation]),
                    &|_, _| false,
                )
                .unwrap();
            let mut fixed_run = run.clone();
            fixed_run
                .correct(
                    &sought(),
                    correction(vec![MeasurementField::StepStderr { step: 0 }]),
                    &|_, _| false,
                )
                .unwrap();
            Changes {
                change_requests: vec![fixed_cr],
                corrected_measurements: vec![fixed_run],
                corrected_invocations: vec![(seq, corrected_invocation(check))],
                ..Changes::default()
            }
        }

        #[tokio::test]
        async fn secure_delete_is_on_for_the_connection() {
            let path = temp_db("secure-delete");
            let store = SqliteStore::open(&path).await.unwrap();
            let on: i64 = store
                .isle
                .call(|c| c.query_row("PRAGMA secure_delete", [], |r| r.get(0)))
                .await
                .unwrap();
            assert_eq!(on, 1);
            store.close().await.unwrap();
        }

        #[tokio::test]
        async fn a_correction_replaces_the_texts_of_a_run_and_of_a_check_and_keeps_their_place() {
            let path = temp_db("correct-rows");
            let store = SqliteStore::open(&path).await.unwrap();
            let first = leaky_run(1, 30);
            let second = leaky_run(2, 30);
            let other = leaky_run(3, 31);
            let check = leaky_invocation();
            let at = store.revision().await.unwrap();
            store
                .write(
                    at,
                    Changes {
                        measurements: vec![first.clone(), second.clone(), other.clone()],
                        invocations: vec![check.clone()],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();
            let rows = store.invocations().await.unwrap();
            assert_eq!(rows.len(), 1);
            let seq = rows[0].0;
            let model = store.model_revision().await.unwrap();

            let mut fixed_first = first.clone();
            fixed_first
                .correct(
                    &sought(),
                    correction(vec![MeasurementField::StepStderr { step: 0 }]),
                    &|_, _| false,
                )
                .unwrap();
            let fixed_check = corrected_invocation(&check);
            let at = store.revision().await.unwrap();
            store
                .write(
                    at,
                    Changes {
                        corrected_measurements: vec![fixed_first.clone()],
                        corrected_invocations: vec![(seq, fixed_check.clone())],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();

            // the corrected run keeps its place among the runs, and the newest of a subject is still the newest
            let kept = [fixed_first.clone(), second.clone(), other.clone()];
            assert_eq!(store.measurements().await.unwrap(), kept);
            assert_eq!(
                store.latest_measurements().await.unwrap(),
                [second.clone(), other.clone()]
            );
            assert_eq!(
                store.invocations().await.unwrap(),
                [(seq, fixed_check.clone())]
            );
            assert_eq!(fixed_check.corrections.len(), 1);
            // a correction of a run or a check is not a change of the model
            assert_eq!(store.model_revision().await.unwrap(), model);
            assert_eq!(store.revision().await.unwrap(), StoreRevision(at.0 + 1));
            store.close().await.unwrap();

            // and it is what a store opened again reads
            let store = SqliteStore::open(&path).await.unwrap();
            assert_eq!(store.measurements().await.unwrap(), kept);
            assert_eq!(store.invocations().await.unwrap(), [(seq, fixed_check)]);
            store.close().await.unwrap();
        }

        /// A simulation whose one case could not start the program, with the token in the reason it gave.
        fn leaky_exit_run(n: u128) -> Measurement {
            use crate::domain::entity::measurement::fixture::sim_draft;
            use crate::domain::vo::id::HarnessId;
            use crate::domain::vo::revision::Revision;
            use crate::domain::vo::sim::{CaseExit, CaseRun, SimBasis, SimCase, SimOutcome};
            let basis = SimBasis {
                harness: Revision::first(),
                contracts: Default::default(),
                interfaces: Default::default(),
            };
            let outcome = SimOutcome {
                cases: vec![CaseRun {
                    case: SimCase::Seed(0),
                    exit: CaseExit::CannotStart(format!("python3 {TOKEN}: No such file")),
                    findings: Vec::new(),
                    unreached: Vec::new(),
                    undecided: Vec::new(),
                    schedule: None,
                    trace: None,
                }],
                reruns: Vec::new(),
            };
            Measurement::new(sim_draft(
                n,
                HarnessId::from_uuid(fixture::id(400)),
                basis,
                outcome,
            ))
            .unwrap()
        }

        #[tokio::test]
        async fn a_corrected_exit_of_a_case_is_the_outcome_the_store_keeps_and_leaves_no_text_behind()
         {
            let path = temp_db("correct-exit");
            let store = SqliteStore::open(&path).await.unwrap();
            let run = leaky_exit_run(1);
            let at = store.revision().await.unwrap();
            store
                .write(
                    at,
                    Changes {
                        measurements: vec![run.clone()],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();
            assert!(files_of(&path).iter().any(|(_, bytes)| holds_token(bytes)));
            let mut fixed = run.clone();
            fixed
                .correct(
                    &sought(),
                    correction(vec![MeasurementField::CaseExit { case: 0 }]),
                    &|_, _| false,
                )
                .unwrap();
            let at = store.revision().await.unwrap();
            store
                .write(
                    at,
                    Changes {
                        corrected_measurements: vec![fixed.clone()],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();
            store.clear_freed().await.unwrap();
            assert_eq!(store.measurements().await.unwrap(), [fixed]);
            for (name, bytes) in files_of(&path) {
                assert!(!holds_token(&bytes), "{name} holds the text");
            }
            store.close().await.unwrap();
        }

        #[tokio::test]
        async fn a_correction_of_a_record_the_store_does_not_have_keeps_nothing() {
            let path = temp_db("correct-missing");
            let store = SqliteStore::open(&path).await.unwrap();
            let at = store.revision().await.unwrap();
            let run = Changes {
                corrected_measurements: vec![leaky_run(5, 30)],
                ..Changes::default()
            };
            assert!(matches!(
                store.write(at, run).await,
                Err(StoreError::Corrupt(_))
            ));
            let row = Changes {
                corrected_invocations: vec![(InvocationSeq(9), leaky_invocation())],
                ..Changes::default()
            };
            assert!(matches!(
                store.write(at, row).await,
                Err(StoreError::Corrupt(_))
            ));
            assert_eq!(store.revision().await.unwrap(), at);
            store.close().await.unwrap();
        }

        #[tokio::test]
        async fn clearing_leaves_nothing_of_a_replaced_text_in_the_files() {
            let path = temp_db("clear-freed");
            let store = SqliteStore::open(&path).await.unwrap();
            let (cr, run, check) = (leaky_cr(), leaky_run(1, 30), leaky_invocation());
            let at = store.revision().await.unwrap();
            store
                .write(
                    at,
                    Changes {
                        change_requests: vec![cr.clone()],
                        measurements: vec![run.clone()],
                        invocations: vec![check.clone()],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();
            // the database holds the text before it is replaced
            assert!(files_of(&path).iter().any(|(_, bytes)| holds_token(bytes)));
            let seq = store.invocations().await.unwrap()[0].0;
            let at = store.revision().await.unwrap();
            store
                .write(at, correcting(&cr, &run, seq, &check))
                .await
                .unwrap();
            store.clear_freed().await.unwrap();
            for (name, bytes) in files_of(&path) {
                assert!(!holds_token(&bytes), "{name} holds the text");
            }
            // what is read back is the corrected record
            let read = store.change_requests().await.unwrap();
            assert_eq!(read[0].situation().unwrap().as_str(), "it holds [redacted]");
            assert_eq!(read[0].history().len(), 1);
            store.close().await.unwrap();
        }

        #[tokio::test]
        async fn clearing_empties_the_write_ahead_log_of_a_file_in_that_mode() {
            let path = temp_db("clear-freed-wal");
            SqliteStore::open(&path)
                .await
                .unwrap()
                .close()
                .await
                .unwrap();
            let conn = Connection::open(&path).unwrap();
            let mode: String = conn
                .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            drop(conn);
            let store = SqliteStore::open(&path).await.unwrap();
            let (cr, run, check) = (leaky_cr(), leaky_run(1, 30), leaky_invocation());
            let at = store.revision().await.unwrap();
            store
                .write(
                    at,
                    Changes {
                        change_requests: vec![cr.clone()],
                        measurements: vec![run.clone()],
                        invocations: vec![check.clone()],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();
            // the log holds the text now, and keeps it when the rows are replaced
            let log = files_of(&path)
                .into_iter()
                .find(|(name, _)| name.ends_with("-wal"))
                .expect("a file in that mode has a write-ahead log while it is open");
            assert!(holds_token(&log.1));
            let seq = store.invocations().await.unwrap()[0].0;
            let at = store.revision().await.unwrap();
            store
                .write(at, correcting(&cr, &run, seq, &check))
                .await
                .unwrap();
            store.clear_freed().await.unwrap();
            for (name, bytes) in files_of(&path) {
                assert!(!holds_token(&bytes), "{name} holds the text");
            }
            let wal = files_of(&path)
                .into_iter()
                .find(|(name, _)| name.ends_with("-wal"));
            assert!(wal.is_none_or(|(_, bytes)| bytes.is_empty()));
            store.close().await.unwrap();
        }

        /// A reader of the log stops the checkpoint from writing it back; the checkpoint answers `busy` and does not
        /// fail, so clearing must say that it did not finish. (The checkpoint waits out the store's busy timeout
        /// first, once.)
        #[tokio::test]
        async fn clearing_says_so_when_a_reader_holds_the_write_ahead_log() {
            let path = temp_db("clear-freed-busy");
            SqliteStore::open(&path)
                .await
                .unwrap()
                .close()
                .await
                .unwrap();
            let conn = Connection::open(&path).unwrap();
            let mode: String = conn
                .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            drop(conn);
            let store = SqliteStore::open(&path).await.unwrap();
            let (cr, run, check) = (leaky_cr(), leaky_run(1, 30), leaky_invocation());
            let at = store.revision().await.unwrap();
            store
                .write(
                    at,
                    Changes {
                        change_requests: vec![cr.clone()],
                        measurements: vec![run.clone()],
                        invocations: vec![check.clone()],
                        ..Changes::default()
                    },
                )
                .await
                .unwrap();
            // a reader begins, and reads, before the correction is written
            let reader = Connection::open(&path).unwrap();
            reader.execute_batch("BEGIN").unwrap();
            let _: i64 = reader
                .query_row("SELECT count(*) FROM change_requests", [], |r| r.get(0))
                .unwrap();
            let seq = store.invocations().await.unwrap()[0].0;
            let at = store.revision().await.unwrap();
            store
                .write(at, correcting(&cr, &run, seq, &check))
                .await
                .unwrap();
            match store.clear_freed().await {
                Err(StoreError::Failed(why)) => {
                    assert!(why.starts_with("wal checkpoint: busy"), "{why}");
                    assert!(!why.contains(TOKEN));
                }
                other => panic!("expected the checkpoint to be busy, got {other:?}"),
            }
            // once the reader is gone, the same clearing finishes
            reader.execute_batch("COMMIT").unwrap();
            drop(reader);
            store.clear_freed().await.unwrap();
            for (name, bytes) in files_of(&path) {
                assert!(!holds_token(&bytes), "{name} holds the text");
            }
            store.close().await.unwrap();
        }
    }

    fn run_of(
        n: u128,
        subject: u128,
        mutants: Vec<crate::domain::vo::measure::MutantResult>,
    ) -> Measurement {
        use crate::domain::entity::measurement::fixture::{draft, empty_basis};
        let mut d = draft(
            n,
            InterfaceId::from_uuid(fixture::id(subject)),
            Digest::of(b"code"),
            empty_basis(),
            mutants,
        );
        // The settings must survive the round trip as they are, and differ from run to run.
        if let crate::domain::entity::measurement::MeasureBody::Mutation { stamp, .. } = &mut d.body
        {
            *stamp = crate::domain::vo::measure::MeasureStamp::new(
                Digest::of(b"code"),
                empty_basis(),
                crate::domain::entity::measurement::fixture::settings_of(&format!("settings-{n}")),
            );
        }
        Measurement::new(d).unwrap()
    }

    #[tokio::test]
    async fn the_newest_measurement_of_a_subject_is_current_and_all_survive_reopening() {
        use crate::domain::entity::measurement::fixture::result;
        use crate::domain::vo::measure::{FailureKind, MeasureFailure, MutantOutcome};
        let path = temp_db("measure");
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(store.latest_measurements().await.unwrap().is_empty());
        let first = run_of(
            1,
            30,
            vec![
                result("f/rel/1/aaaa", 7, MutantOutcome::Killed),
                result("f/rel/2/bbbb", 9, MutantOutcome::Survived),
            ],
        );
        let at = store.revision().await.unwrap();
        let kept = Changes {
            measurements: vec![first.clone()],
            ..Changes::default()
        };
        store.write(at, kept).await.unwrap();
        assert_eq!(
            store.latest_measurements().await.unwrap(),
            std::slice::from_ref(&first)
        );

        // a newer run of the same subject replaces it as the current one; another subject has its own
        let mut failed = crate::domain::entity::measurement::fixture::draft(
            2,
            InterfaceId::from_uuid(fixture::id(30)),
            Digest::of(b"code"),
            crate::domain::entity::measurement::fixture::empty_basis(),
            vec![result("f/rel/1/aaaa", 7, MutantOutcome::Killed)],
        );
        failed.failure =
            Some(MeasureFailure::new(FailureKind::Budget, "used the whole budget").unwrap());
        let second = Measurement::new(failed).unwrap();
        let other = run_of(3, 31, vec![]);
        let at = store.revision().await.unwrap();
        let kept = Changes {
            measurements: vec![second.clone(), other.clone()],
            ..Changes::default()
        };
        store.write(at, kept).await.unwrap();
        let latest = store.latest_measurements().await.unwrap();
        assert_eq!(latest, [second.clone(), other.clone()]);
        assert!(latest[0].failure().is_some());
        assert_eq!(latest[0].mutants().len(), 1);
        store.close().await.unwrap();

        // history is kept: the runs are rows, and the first one's items are still there
        let conn = Connection::open(&path).unwrap();
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM measure_runs", [], |r| r.get(0))
            .unwrap();
        let items: i64 = conn
            .query_row("SELECT COUNT(*) FROM measure_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!((runs, items), (3, 3));
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.latest_measurements().await.unwrap(), [second, other]);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn the_latest_measurements_of_a_kind_are_those_of_that_kind_alone_and_the_others_are_not_read()
     {
        use crate::domain::entity::measurement::fixture::{examples_draft, result, sim_draft};
        use crate::domain::vo::examples_run::{
            ExampleRecord, ExampleResult, ExamplesBasis, ExamplesOutcome,
        };
        use crate::domain::vo::id::{CaseId, HarnessId};
        use crate::domain::vo::measure::{MeasureKind, MutantOutcome};
        use crate::domain::vo::revision::Revision;
        use crate::domain::vo::sim::{SimBasis, SimOutcome};
        let path = temp_db("measure-of-kind");
        let store = SqliteStore::open(&path).await.unwrap();
        let harness = HarnessId::from_uuid(fixture::id(40));
        let examples = |n: u128| {
            Measurement::new(examples_draft(
                n,
                harness,
                ExamplesBasis {
                    harness: Revision::first(),
                    builds: Default::default(),
                },
                ExamplesOutcome::new(vec![ExampleRecord {
                    case: CaseId::from_uuid(fixture::id(41)),
                    result: ExampleResult::Passed,
                }])
                .unwrap(),
            ))
            .unwrap()
        };
        let sim = Measurement::new(sim_draft(
            9,
            harness,
            SimBasis {
                harness: Revision::first(),
                contracts: Default::default(),
                interfaces: Default::default(),
            },
            SimOutcome::default(),
        ))
        .unwrap();
        let (older, newer) = (examples(1), examples(2));
        let mutation = run_of(
            3,
            31,
            vec![result("f/rel/1/aaaa", 7, MutantOutcome::Killed)],
        );
        let at = store.revision().await.unwrap();
        store
            .write(
                at,
                Changes {
                    measurements: vec![older, mutation.clone(), sim.clone(), newer.clone()],
                    ..Changes::default()
                },
            )
            .await
            .unwrap();
        // the newest run of the kind, of each subject; the Harness has a simulation too, and it is not here
        assert_eq!(
            store
                .latest_measurements_of(MeasureKind::Examples)
                .await
                .unwrap(),
            std::slice::from_ref(&newer)
        );
        assert_eq!(
            store
                .latest_measurements_of(MeasureKind::Simulate)
                .await
                .unwrap(),
            [sim]
        );
        assert_eq!(
            store
                .latest_measurements_of(MeasureKind::Mutate)
                .await
                .unwrap(),
            [mutation]
        );
        store.close().await.unwrap();

        // the rows of the other kinds are not read at all: a mutation row that cannot be decoded does not stop it
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE measure_runs SET inputs = '{' WHERE kind = 'mutate'",
            [],
        )
        .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(store.latest_measurements().await.is_err());
        assert_eq!(
            store
                .latest_measurements_of(MeasureKind::Examples)
                .await
                .unwrap(),
            [newer]
        );
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_conflicting_commit_keeps_no_measurement_and_no_pin_code() {
        use crate::app::ports::PinCode;
        let path = temp_db("measure-conflict");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        store.write(at, changes()).await.unwrap();
        let stale = Changes {
            measurements: vec![run_of(1, 30, vec![])],
            pin_codes: vec![PinCode {
                pin: PinId::from_uuid(fixture::id(50)),
                interface: InterfaceId::from_uuid(fixture::id(30)),
                code: "fn a() {}".to_string(),
            }],
            ..Changes::default()
        };
        assert_eq!(store.write(at, stale).await, Err(StoreError::Conflict));
        assert!(store.latest_measurements().await.unwrap().is_empty());
        assert_eq!(
            store
                .pin_code(PinId::from_uuid(fixture::id(50)))
                .await
                .unwrap(),
            None
        );
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn the_code_of_a_pin_is_kept_whole_and_the_next_pin_replaces_it() {
        use crate::app::ports::PinCode;
        let path = temp_db("pin-code");
        let store = SqliteStore::open(&path).await.unwrap();
        let code = |pin: u128, text: &str| PinCode {
            pin: PinId::from_uuid(fixture::id(pin)),
            interface: InterfaceId::from_uuid(fixture::id(30)),
            code: text.to_string(),
        };
        let text = "pub fn fits(a: u32) -> bool {\n    a <= 5\n}\n";
        let at = store.revision().await.unwrap();
        let kept = Changes {
            pin_codes: vec![code(50, text)],
            ..Changes::default()
        };
        store.write(at, kept).await.unwrap();
        assert_eq!(
            store
                .pin_code(PinId::from_uuid(fixture::id(50)))
                .await
                .unwrap(),
            Some(text.to_string())
        );
        let at = store.revision().await.unwrap();
        let kept = Changes {
            pin_codes: vec![code(51, "other")],
            ..Changes::default()
        };
        store.write(at, kept).await.unwrap();
        assert_eq!(
            store
                .pin_code(PinId::from_uuid(fixture::id(50)))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .pin_code(PinId::from_uuid(fixture::id(51)))
                .await
                .unwrap(),
            Some("other".to_string())
        );
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_measurement_row_changed_to_what_cannot_be_is_corrupt() {
        use crate::domain::entity::measurement::fixture::result;
        use crate::domain::vo::measure::MutantOutcome;
        for (n, sql) in [
            "UPDATE measure_items SET outcome = 'lost'",
            "UPDATE measure_items SET site = '0:0'",
            "UPDATE measure_runs SET outcome = 'failed'",
            "UPDATE measure_runs SET inputs = '{'",
        ]
        .into_iter()
        .enumerate()
        {
            let path = temp_db(&format!("measure-corrupt-{n}"));
            let store = SqliteStore::open(&path).await.unwrap();
            let at = store.revision().await.unwrap();
            let kept = Changes {
                measurements: vec![run_of(
                    1,
                    30,
                    vec![result("f/rel/1/aaaa", 7, MutantOutcome::Killed)],
                )],
                ..Changes::default()
            };
            store.write(at, kept).await.unwrap();
            assert_eq!(store.latest_measurements().await.unwrap().len(), 1);
            store.close().await.unwrap();
            let conn = Connection::open(&path).unwrap();
            conn.execute(sql, []).unwrap();
            drop(conn);
            let store = SqliteStore::open(&path).await.unwrap();
            assert!(
                matches!(
                    store.latest_measurements().await,
                    Err(StoreError::Corrupt(_))
                ),
                "{sql}"
            );
            store.close().await.unwrap();
        }
    }

    fn invocation_with(trigger: Option<Trigger>) -> Changes {
        let mut c = changes();
        c.invocations[0].trigger = trigger;
        c
    }

    #[tokio::test]
    async fn an_invocation_keeps_the_trigger_of_the_use_case_that_ran_it() {
        use crate::app::commit::{AskTrigger, EditTrigger};

        let path = temp_db("trigger");
        let store = SqliteStore::open(&path).await.unwrap();
        let first = Trigger::Edit(EditTrigger::PutContract);
        let second = Trigger::Ask(AskTrigger::CheckRun);
        store
            .write(StoreRevision(0), invocation_with(Some(first)))
            .await
            .unwrap();
        store
            .write(StoreRevision(1), invocation_with(Some(second)))
            .await
            .unwrap();
        let read = ran(&store).await;
        assert_eq!(read[0].trigger, Some(first));
        assert_eq!(read[1].trigger, Some(second));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_row_made_before_the_trigger_was_kept_reads_without_one() {
        let path = temp_db("trigger-old-row");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A database as an earlier version left it: the table has no `trigger` column.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(CREATE).unwrap();
        conn.execute_batch(CREATE_INVOCATIONS).unwrap();
        conn.execute(
            "INSERT INTO check_invocations
             (store_revision, check_id, hook, notes, warnings, errors, outcome)
             VALUES (0, 'cite.uncited-clause', 'put', 0, 2, 0, 'found')",
            [],
        )
        .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        let read = ran(&store).await;
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].trigger, None);
        // The word `put` was the moment of an edit, and still reads as one.
        assert_eq!(read[0].moment, Moment::Edit);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_trigger_word_no_version_wrote_is_a_corrupt_row() {
        let path = temp_db("trigger-unknown");
        let store = SqliteStore::open(&path).await.unwrap();
        store
            .write(StoreRevision(0), invocation_with(None))
            .await
            .unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE check_invocations SET \"trigger\" = 'bogus'", [])
            .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(matches!(
            store.invocations().await,
            Err(StoreError::Corrupt(_))
        ));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn the_moment_is_written_by_its_kept_word_and_either_word_of_an_edit_is_read_as_one() {
        let path = temp_db("moment-word");
        let store = SqliteStore::open(&path).await.unwrap();
        store
            .write(StoreRevision(0), invocation_with(None))
            .await
            .unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        let written: String = conn
            .query_row("SELECT hook FROM check_invocations", [], |r| r.get(0))
            .unwrap();
        // The word 0.1.0 reads, so that the file stays readable to it.
        assert_eq!(written, "put");
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(ran(&store).await[0].moment, Moment::Edit);
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE check_invocations SET hook = 'edit'", [])
            .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(ran(&store).await[0].moment, Moment::Edit);
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE check_invocations SET hook = 'save'", [])
            .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(matches!(
            store.invocations().await,
            Err(StoreError::Corrupt(_))
        ));
        store.close().await.unwrap();
    }

    // ---- a store of layout 1, which held the Crux inside its change request ----

    /// The tables of a store of layout 1, as the code before the Cruxes were a record of their own made them, with
    /// the revision of the store at 7 and the revision of the model at 5.
    const LAYOUT_1: &str = "
    BEGIN;
    CREATE TABLE meta (id INTEGER PRIMARY KEY CHECK (id = 1), revision INTEGER NOT NULL);
    INSERT INTO meta (id, revision) VALUES (1, 7);
    CREATE TABLE project (id TEXT PRIMARY KEY, schema INTEGER NOT NULL, doc TEXT NOT NULL);
    CREATE TABLE pins (interface TEXT PRIMARY KEY, schema INTEGER NOT NULL, doc TEXT NOT NULL);
    CREATE TABLE change_requests (id TEXT PRIMARY KEY, schema INTEGER NOT NULL, doc TEXT NOT NULL);
    CREATE TABLE asset_names (
        context TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL, id TEXT NOT NULL,
        PRIMARY KEY (context, kind, name)
    );
    CREATE INDEX asset_names_by_id ON asset_names (id);
    PRAGMA user_version = 1;
    COMMIT;
    ";

    fn uid(n: u128) -> String {
        Uuid::from_u128(n).to_string()
    }

    /// A change request document as the code before the Cruxes were records wrote it: its Crux inside.
    fn old_request(
        n: u128,
        title: &str,
        crux: serde_json::Value,
        decided_expects: serde_json::Value,
        history: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "schema": 2, "id": uid(n), "title": title, "situation": null, "intent": "change_behavior",
            "parent": null, "issuer": null, "expects": [], "crux": crux,
            "decided_expects": decided_expects, "assumptions": [], "addresses": [], "work": [],
            "history": history,
        })
    }

    fn old_crux(
        recommended: &str,
        supersedes: Option<String>,
        answer: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "kind": "ambiguity",
            "options": [
                { "choice": "fail", "change": null, "expects": [], "consequence": "callers check" },
                { "choice": "zero", "change": "C2 says zero",
                  "expects": [{ "target": "clause", "id": uid(31), "kind": "target" }],
                  "consequence": null },
            ],
            "recommended": { "choice": recommended, "reason": "it is the literal reading", "confidence": 85 },
            "importance": "high",
            "supersedes": supersedes,
            "answer": answer,
        })
    }

    fn old_answer(decision: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "decision": decision,
            "basis": { "arbitrary": { "reason": "nothing derives it" } },
            "by": { "role": "orchestrator" },
            "at": 1_700_000_000_123u64,
        })
    }

    /// Writes a store of layout 1 holding these change request documents (id, schema, document).
    fn layout_1_store(
        name: &str,
        requests: &[(u128, u32, serde_json::Value)],
    ) -> std::path::PathBuf {
        let path = temp_db(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(LAYOUT_1).unwrap();
        conn.execute_batch(CREATE_INVOCATIONS).unwrap();
        conn.execute_batch(CREATE_JUDGED).unwrap();
        conn.execute("UPDATE model_revision SET revision = 5", [])
            .unwrap();
        for (n, schema, doc) in requests {
            conn.execute(
                "INSERT INTO change_requests (id, schema, doc) VALUES (?1, ?2, ?3)",
                params![uid(*n), schema, doc.to_string()],
            )
            .unwrap();
        }
        path
    }

    /// The four requests of a store as the code before wrote it: a question open, one answered with a chosen option
    /// that was recommended, one whose Crux supersedes the answered one, one resolved as `wont_do` with its question
    /// open; and one without a Crux, of schema 1.
    fn old_store(name: &str) -> std::path::PathBuf {
        let none = serde_json::json!([]);
        let wont_do = serde_json::json!([{ "resolved": {
            "kind": "wont_do", "reason": "not now", "by": { "person": "alice" }, "at": 1 } }]);
        layout_1_store(
            name,
            &[
                (
                    1,
                    2,
                    old_request(
                        1,
                        "open question",
                        old_crux("fail", None, serde_json::Value::Null),
                        none.clone(),
                        none.clone(),
                    ),
                ),
                (
                    2,
                    2,
                    old_request(
                        2,
                        "answered question",
                        old_crux(
                            "zero",
                            None,
                            old_answer(serde_json::json!({
                                "chosen": { "choice": "zero", "reason": "fairer" } })),
                        ),
                        serde_json::json!([{ "target": "clause", "id": uid(31), "kind": "target", "recorded": 2 }]),
                        none.clone(),
                    ),
                ),
                (
                    3,
                    2,
                    old_request(
                        3,
                        "redecided",
                        old_crux(
                            "fail",
                            Some(uid(2)),
                            old_answer(serde_json::json!({
                                "worded": { "wording": "in my words", "reason": null } })),
                        ),
                        none.clone(),
                        none.clone(),
                    ),
                ),
                (
                    4,
                    2,
                    old_request(
                        4,
                        "dropped question",
                        old_crux("zero", None, serde_json::Value::Null),
                        none.clone(),
                        wont_do,
                    ),
                ),
                (5, 1, {
                    let mut plain = old_request(
                        5,
                        "no question",
                        serde_json::Value::Null,
                        none.clone(),
                        none,
                    );
                    plain["schema"] = serde_json::json!(1);
                    plain
                }),
            ],
        )
    }

    /// What a store holds that a migration must leave as it was when it fails: the layout, the tables, the change
    /// request documents, and the revisions.
    #[allow(clippy::type_complexity)]
    fn snapshot(
        path: &std::path::Path,
    ) -> (i64, Vec<String>, Vec<(String, u32, String)>, i64, i64) {
        let conn = Connection::open(path).unwrap();
        let layout = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let tables = conn
            .prepare("SELECT name FROM sqlite_master ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let requests = conn
            .prepare("SELECT id, schema, doc FROM change_requests ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let revision = conn
            .query_row("SELECT revision FROM meta", [], |r| r.get(0))
            .unwrap();
        let model = conn
            .query_row("SELECT revision FROM model_revision", [], |r| r.get(0))
            .unwrap();
        (layout, tables, requests, revision, model)
    }

    fn crux_titled<'a>(cruxes: &'a [Crux], crs: &[ChangeRequest], title: &str) -> &'a Crux {
        let cr = crs.iter().find(|c| c.title().as_str() == title).unwrap();
        cruxes.iter().find(|q| q.raised_in() == cr.id()).unwrap()
    }

    #[tokio::test]
    async fn a_new_file_is_made_at_layout_2_directly_with_the_table_of_cruxes() {
        let path = temp_db("new-layout-2");
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(store.cruxes().await.unwrap().is_empty());
        store.close().await.unwrap();
        let (layout, tables, ..) = snapshot(&path);
        assert_eq!(layout, 2);
        assert!(tables.contains(&"cruxes".to_string()));
    }

    #[tokio::test]
    async fn a_store_of_layout_1_is_moved_once_when_it_is_opened() {
        use crate::domain::service::change_request::state_of;
        use crate::domain::vo::crux::Decision;
        let path = old_store("migrate");
        assert_eq!(snapshot(&path).0, 1);
        let store = SqliteStore::open(&path).await.unwrap();
        let crs = store.change_requests().await.unwrap();
        let cruxes = store.cruxes().await.unwrap();
        // five requests, one Crux for each that held one
        assert_eq!(crs.len(), 5);
        assert_eq!(cruxes.len(), 4);
        let by = |title: &str| crs.iter().find(|c| c.title().as_str() == title).unwrap();
        for c in &crs {
            assert!(
                cruxes.iter().filter(|q| q.raised_in() == c.id()).count()
                    == usize::from(c.title().as_str() != "no question")
            );
        }
        // the open question: raised in its request, with the title for its question, no findings, its options in
        // order with ids, and its recommendation pointing at an option
        let open = crux_titled(&cruxes, &crs, "open question");
        assert!(open.is_open());
        assert_eq!(open.raised_in(), by("open question").id());
        assert_eq!(open.question().as_str(), "open question");
        assert!(open.findings().is_empty());
        let choices: Vec<&str> = open.options().iter().map(|o| o.choice().as_str()).collect();
        assert_eq!(choices, ["fail", "zero"]);
        assert_ne!(open.options()[0].id(), open.options()[1].id());
        assert_eq!(
            open.options()[1].change().map(|p| p.as_str()),
            Some("C2 says zero")
        );
        assert_eq!(
            open.options()[0].consequence().map(|p| p.as_str()),
            Some("callers check")
        );
        assert_eq!(open.options()[1].expects().len(), 1);
        let recommended = open.recommended().unwrap();
        assert_eq!(recommended.option(), open.options()[0].id());
        assert_eq!(recommended.confidence().map(|c| c.percent()), Some(85));
        assert_eq!(
            open.importance(),
            Some(crate::domain::vo::crux::Importance::High)
        );
        assert_eq!(state_of(by("open question"), &cruxes).word(), "open");
        // the ids are the ids of a clock and a random source, made in `infra`
        assert_eq!(open.id().as_uuid().get_version_num(), 7);
        assert_eq!(open.options()[0].id().as_uuid().get_version_num(), 7);
        // the answered one: the decision points at the option the text named, and has an id
        let answered = crux_titled(&cruxes, &crs, "answered question");
        let answer = answered.answer_given().unwrap();
        assert!(matches!(
            answer.decision(),
            Decision::Chosen { option, reason }
                if *option == answered.options()[1].id()
                    && reason.as_ref().map(|r| r.as_str()) == Some("fairer")
        ));
        assert_eq!(answer.id().as_uuid().get_version_num(), 7);
        assert_eq!(answer.by().to_string(), "role orchestrator");
        assert_eq!(answer.at().millis(), 1_700_000_000_123);
        assert_eq!(
            answered.recommended().map(|r| r.option()),
            Some(answered.options()[1].id())
        );
        // what the decision asked stays in the request, which is decided
        assert_eq!(by("answered question").decided_expects().len(), 1);
        // and names the answer that asked for it: the one the Crux was moved with
        assert!(
            by("answered question")
                .decided_expects()
                .iter()
                .all(|e| e.answer == answer.id())
        );
        assert_eq!(state_of(by("answered question"), &cruxes).word(), "decided");
        // the one that supersedes points at the Crux of the request it named
        let redecided = crux_titled(&cruxes, &crs, "redecided");
        assert_eq!(redecided.supersedes(), Some(answered.id()));
        assert!(matches!(
            redecided.decision(),
            Some(Decision::Worded { .. })
        ));
        // the request resolved as `wont_do` with its question open is kept so, and the question is still asked
        let dropped = crux_titled(&cruxes, &crs, "dropped question");
        assert!(dropped.is_open());
        assert!(by("dropped question").is_resolved());
        assert_eq!(state_of(by("dropped question"), &cruxes).word(), "resolved");
        // the tables a layout-1 file lacked are there as after any open
        assert!(store.invocations().await.unwrap().is_empty());
        // every request is of schema 3 now
        store.close().await.unwrap();
        let (layout, tables, requests, revision, model) = snapshot(&path);
        assert_eq!(layout, 2);
        assert!(tables.contains(&"cruxes".to_string()));
        assert!(requests.iter().all(|(_, schema, doc)| {
            *schema == 3 && !doc.contains("\"crux\"") && doc.contains("\"schema\":3")
        }));
        // neither the revision of the store nor the revision of the model moved
        assert_eq!((revision, model), (7, 5));
    }

    #[tokio::test]
    async fn opening_a_moved_store_again_changes_nothing() {
        let path = old_store("migrate-twice");
        SqliteStore::open(&path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        let rows = |conn: &Connection| -> Vec<(String, String, u32, String)> {
            conn.prepare("SELECT id, raised_in, schema, doc FROM cruxes ORDER BY id")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        let first = rows(&conn);
        assert_eq!(first.len(), 4);
        let was = snapshot(&path);
        drop(conn);
        SqliteStore::open(&path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        assert_eq!(snapshot(&path), was);
        assert_eq!(rows(&Connection::open(&path).unwrap()), first);
    }

    #[tokio::test]
    async fn a_second_process_that_finds_the_store_moved_does_nothing() {
        // The layout is read again after the write lock is taken: a connection that saw layout 1 before another
        // finished the move leaves the store alone.
        let path = old_store("migrate-race");
        SqliteStore::open(&path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let was = snapshot(&path);
        let mut conn = Connection::open(&path).unwrap();
        crate::infra::persist::layout2::migrate(&mut conn).unwrap();
        drop(conn);
        assert_eq!(snapshot(&path), was);
    }

    #[tokio::test]
    async fn a_store_whose_decision_matches_no_option_is_not_moved_and_says_what_did_not_match() {
        let none = serde_json::json!([]);
        for (what, doc, said) in [
            (
                "the decision",
                old_request(
                    1,
                    "the question",
                    old_crux(
                        "fail",
                        None,
                        old_answer(serde_json::json!({
                            "chosen": { "choice": "sideways", "reason": null } })),
                    ),
                    none.clone(),
                    none.clone(),
                ),
                "sideways",
            ),
            (
                "the recommendation",
                old_request(
                    1,
                    "the question",
                    old_crux("diagonally", None, serde_json::Value::Null),
                    none.clone(),
                    none.clone(),
                ),
                "diagonally",
            ),
            (
                "what is superseded",
                old_request(
                    1,
                    "the question",
                    old_crux("fail", Some(uid(9)), serde_json::Value::Null),
                    none.clone(),
                    none.clone(),
                ),
                &uid(9)[..],
            ),
        ] {
            let path = layout_1_store(
                "migrate-fails",
                &[
                    (
                        2,
                        2,
                        old_request(
                            2,
                            "other",
                            serde_json::Value::Null,
                            none.clone(),
                            none.clone(),
                        ),
                    ),
                    (1, 2, doc),
                ],
            );
            let was = snapshot(&path);
            let opened = SqliteStore::open(&path).await.map(|_| ());
            match opened {
                Err(StoreError::Corrupt(why)) => {
                    assert!(why.contains(said), "{what}: {why}");
                    assert!(why.contains("the question"), "{what}: {why}");
                }
                other => panic!("{what}: {other:?}"),
            }
            // the file is as it was: layout 1, no table of Cruxes, the documents as they were
            assert_eq!(snapshot(&path), was, "{what}");
            assert_eq!(was.0, 1);
            assert!(!was.1.contains(&"cruxes".to_string()));
        }
    }

    #[tokio::test]
    async fn a_store_of_layout_1_whose_moved_records_do_not_fit_together_is_not_moved() {
        // Resolved as done, with its question open: the old code refused to read it, and so does the new.
        let none = serde_json::json!([]);
        let done = serde_json::json!([{ "resolved": {
            "kind": "done", "reason": "r", "by": { "person": "alice" }, "at": 1 } }]);
        let path = layout_1_store(
            "migrate-unfit",
            &[(
                1,
                2,
                old_request(
                    1,
                    "the question",
                    old_crux("fail", None, serde_json::Value::Null),
                    none,
                    done,
                ),
            )],
        );
        let was = snapshot(&path);
        assert!(matches!(
            SqliteStore::open(&path).await.map(|_| ()),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(snapshot(&path), was);
    }

    #[tokio::test]
    async fn a_store_of_layout_1_with_a_recorded_expectation_and_no_answer_to_name_is_not_moved() {
        // What a decision asked is recorded, but the request holds no answered question: nothing to name.
        let none = serde_json::json!([]);
        let path = layout_1_store(
            "migrate-unnamed",
            &[(
                1,
                2,
                old_request(
                    1,
                    "no question",
                    serde_json::Value::Null,
                    serde_json::json!([{ "target": "clause", "id": uid(31), "kind": "target", "recorded": 2 }]),
                    none,
                ),
            )],
        );
        let was = snapshot(&path);
        assert!(matches!(
            SqliteStore::open(&path).await.map(|_| ()),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(snapshot(&path), was);
    }

    /// The word an earlier version wrote for the decision of a change request is read back as the trigger it was,
    /// though no use case writes it now.
    #[tokio::test]
    async fn an_invocation_an_earlier_version_recorded_under_the_old_word_of_a_decision_is_read_back()
     {
        use crate::app::commit::{EditTrigger, Trigger};

        let path = temp_db("old-trigger-word");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        store.write(at, changes()).await.unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE check_invocations SET trigger = 'decide_change_request'",
            [],
        )
        .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        let read = ran(&store).await;
        assert_eq!(read.len(), 1);
        assert_eq!(
            read[0].trigger,
            Some(Trigger::Edit(EditTrigger::DecideChangeRequest))
        );
        store.close().await.unwrap();
    }

    /// A store can hold a request resolved before its question was answered (a record of what was, and what a store
    /// of an earlier version may hold). The question is still awaited; it takes a withdrawal and no other decision
    /// until the request is reopened, and the request is not rewritten by it.
    #[tokio::test]
    async fn a_question_open_in_a_request_resolved_before_takes_a_withdrawal_and_no_other_decision()
    {
        use crate::app::{
            App, AppError, BasisInput, DecisionInput, DecisionMakerInput, ports::Store as _,
        };
        use crate::domain::entity::change_request::ChangeRequestError;
        use crate::domain::vo::timestamp::Timestamp;
        use crate::infra::{clock::FixedClock, ids::UuidV7};

        let path = temp_db("resolved-open-crux");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        let held = changes();
        store.write(at, held.clone()).await.unwrap();
        let mut app = App::new(store, UuidV7, FixedClock(Timestamp::from_millis(5)));
        let open = held.cruxes.iter().find(|q| q.is_open()).unwrap();
        let id = open.id().to_string();

        let awaiting = app.status().await.unwrap().awaiting;
        assert_eq!(awaiting.len(), 1);
        assert_eq!(awaiting[0].crux, id);
        assert_eq!(awaiting[0].raised_in, "rename push");
        assert_eq!(awaiting[0].question, "is a stack a list?");

        let basis = BasisInput::Arbitrary("settled".into());
        let by = DecisionMakerInput::Person("alice".into());
        let chosen = DecisionInput::Chosen {
            option: open.options()[0].id().to_string(),
            reason: None,
        };
        let revision = app.store().revision().await.unwrap();
        let refused = app.decide_crux(&id, &chosen, None, &basis, &by).await;
        assert!(
            matches!(
                refused,
                Err(AppError::ChangeRequestOf {
                    error: ChangeRequestError::AlreadyResolved,
                    ..
                })
            ),
            "{refused:?}"
        );
        assert_eq!(app.store().revision().await.unwrap(), revision);

        app.decide_crux(&id, &DecisionInput::Withdrawn, None, &basis, &by)
            .await
            .unwrap();
        let store = app.into_store();
        let after = store.crux(open.id()).await.unwrap().unwrap();
        assert_eq!(
            after.decision(),
            Some(&crate::domain::vo::crux::Decision::Withdrawn)
        );
        assert_eq!(store.change_requests().await.unwrap(), held.change_requests);
        store.close().await.unwrap();
    }
}
