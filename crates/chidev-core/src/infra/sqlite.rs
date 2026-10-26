//! The store in a SQLite file, through `rusqlite-isle`: the connection lives on a thread of its own and is reached
//! by async calls, so no blocking call runs on the caller's runtime. An async call cannot hold a transaction across
//! `.await`, so [`Store::commit`] writes everything of one use case in a single `call_write` closure, after
//! checking that nothing was committed since the use case began to read.
//!
//! The file holds `meta` (the store's revision, which every commit bumps), `project`, `pins` and
//! `change_requests` (one JSON document each, in the schema the record names; see `persist`), `asset_names`, and
//! `check_invocations` (the checks each use case ran, appended and never changed; the column `hook` holds the
//! moment by its word, and `"trigger"` the use case that ran the check, by the word of its trigger), `judged_reports` and
//! `judged_findings` (what agents reported for the judged rules: one report per rule, replaced as a whole, and its
//! findings), `external_reports` (the latest result per external rule), `pin_codes` (the code each Pin
//! accepted), `measure_runs` and `measure_items` (the measurements, appended, and their mutants), and
//! `model_revision` (the store revision at which the model last changed). `PRAGMA user_version` is the layout of the tables; a file with a higher one
//! is refused. New tables, execution metadata and the trigger on `check_invocations`, and `rule_digest` on `judged_reports` are added
//! without a new layout number: they are created `IF NOT EXISTS` (or
//! added when missing) each time a file of layout 1 is opened, so a database made before them gets them and
//! nothing else in it changes. Old rule digests remain NULL: a current definition cannot establish the
//! definition that an earlier judgment used.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, params};
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver, IsleError};
use uuid::Uuid;

use super::persist::measure::{ItemRow, RunRow};
use super::persist::{self, CURRENT};
use crate::app::commit::{Gated, Trigger};
use crate::app::ports::external::{ExecutionEvidence, ExternalReport};
use crate::app::ports::{
    AssetKind, AssetNames, Changes, Invocation, JudgedFinding, JudgedReport, JudgedRepository,
    Outcome, Store, StoreRevision,
};
use crate::domain::check::level::MomentWordError;
use crate::domain::check::{Executor, Level, Moment};
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{
    ChangeRequestRepository, PinRepository, ProjectRepository, StoreError,
};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{RuleId, Subject, SubjectKind};
use crate::domain::vo::id::{ChangeRequestId, ContextId, InterfaceId, PinId};

/// The layout of the tables this code creates and reads.
const LAYOUT: i64 = 1;

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
PRAGMA user_version = 1;
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
    rule_digest TEXT
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

/// The code each Pin accepted (the latest per Interface), and the measurements: `measure_runs` (appended, never
/// changed: the newest per kind and subject is current, the older ones are history) with its `measure_items` (the
/// mutants of a run), written in the commit of the use case that made the run.
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
    summary TEXT NOT NULL
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

/// Makes the tables on a database that has none; leaves any other alone (its layout is checked after opening).
fn prepare(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    let layout: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if layout == 0 {
        conn.execute_batch(CREATE)?;
    }
    if layout <= LAYOUT {
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
        conn.execute_batch(CREATE_JUDGED)?;
        add_column(conn, "judged_reports", "rule_digest", "TEXT")?;
        conn.execute_batch(CREATE_EXTERNAL)?;
        conn.execute_batch(CREATE_MEASURE)?;
    }
    Ok(())
}

/// The revision a stored number is; none for a negative one.
fn revision_of(n: i64) -> Option<StoreRevision> {
    u64::try_from(n).ok().map(StoreRevision)
}

/// An invocation as stored: revision, check, moment, counts, outcome, executor, judge, external evidence, rule
/// digest, trigger.
type InvocationRow = (
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
);

/// A stored document and the schema its record names.
type Row = (u32, String);

/// The store in a SQLite file.
pub struct SqliteStore {
    isle: AsyncIsle,
    driver: AsyncIsleDriver,
}

impl SqliteStore {
    /// Opens the database at `path`, making it (and the directory it is in) if it is not there. A database of a
    /// later layout is refused.
    pub async fn open(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
        let path = path.as_ref();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            // Once, before the connection exists; there is no runtime facility to wait for yet.
            std::fs::create_dir_all(dir)
                .map_err(|e| StoreError::Failed(format!("{}: {e}", dir.display())))?;
        }
        let (isle, driver) = AsyncIsle::builder()
            .busy_timeout(BUSY_TIMEOUT)
            .spawn(path, prepare)
            .await
            .map_err(failed)?;
        let store = SqliteStore { isle, driver };
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

    /// The checks the use cases have run, in the order they were kept. Not part of the [`Store`] port: nothing in a
    /// use case reads them back; they are for the person (or a test) who asks what ran.
    pub async fn invocations(&self) -> Result<Vec<Invocation>, StoreError> {
        let rows: Vec<InvocationRow> = self
            .isle
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT store_revision, check_id, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, \"trigger\"
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
                )| {
                    Ok(Invocation {
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
                        outcome: Outcome::from_word(&outcome).ok_or_else(|| corrupt(&outcome))?,
                        executor: Executor::from_word(&executor)
                            .ok_or_else(|| corrupt(&executor))?,
                        judge,
                        trigger: trigger
                            .map(|w| Trigger::from_word(&w).ok_or_else(|| corrupt(&w)))
                            .transpose()?,
                        rule_digest: rule_digest
                            .map(|d| Digest::restore(&d).ok_or_else(|| corrupt("a rule digest")))
                            .transpose()?,
                        external: external
                            .map(|doc| {
                                serde_json::from_str::<ExecutionEvidence>(&doc)
                                    .map_err(|e| corrupt(&format!("external evidence: {e}")))
                            })
                            .transpose()?,
                    })
                },
            )
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

impl AssetNames for SqliteStore {
    async fn asset_id(
        &self,
        context: ContextId,
        kind: AssetKind,
        name: &str,
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

    async fn asset_name(&self, id: Uuid) -> Result<Option<String>, StoreError> {
        let id = id.to_string();
        self.isle
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT name FROM asset_names WHERE id = ?1 ORDER BY context, kind LIMIT 1",
                )?;
                let mut rows = stmt.query(params![id])?;
                rows.next()?.map(|r| r.get(0)).transpose()
            })
            .await
            .map_err(failed)
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
        let runs: Vec<(RunRow, Vec<ItemRow>)> = self
            .isle
            .call(|c| {
                let mut runs = c.prepare(
                    "SELECT id, kind, subject_kind, subject_id, stamp, inputs, started_at, outcome, failure, evidence, summary
                     FROM measure_runs
                     WHERE seq IN (SELECT MAX(seq) FROM measure_runs GROUP BY kind, subject_id)
                     ORDER BY seq",
                )?;
                let rows: Vec<RunRow> = runs
                    .query_map([], |r| {
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
                    i.external
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|e| StoreError::Failed(format!("external evidence: {e}")))?,
                    i.rule_digest.map(|d| d.to_string()),
                    i.trigger.map(|t| t.word()),
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
        // What an agent is shown is the Project, the change requests and the names; a Pin is not in the view.
        let model_changed = project.is_some() || !crs.is_empty() || !names.is_empty();
        let current = CURRENT;
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
                for (run, items) in &measures {
                    tx.execute(
                        "INSERT INTO measure_runs
                         (id, kind, subject_kind, subject_id, stamp, inputs, store_revision, started_at, outcome, failure, evidence, summary)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        params![
                            run.id, run.kind, run.subject_kind, run.subject, run.stamp, run.inputs,
                            read_at, run.started_at, run.outcome, run.failure, run.evidence,
                            run.summary
                        ],
                    )?;
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
                }
                for (id, doc) in &crs {
                    tx.execute(
                        "INSERT INTO change_requests (id, schema, doc) VALUES (?1, ?2, ?3)
                         ON CONFLICT (id) DO UPDATE SET schema = excluded.schema, doc = excluded.doc",
                        params![id, current, doc],
                    )?;
                }
                for n in &names {
                    tx.execute(
                        "INSERT INTO asset_names (context, kind, name, id) VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT (context, kind, name) DO UPDATE SET id = excluded.id",
                        params![
                            n.context.as_uuid().to_string(),
                            n.kind.word(),
                            n.name,
                            n.id.to_string()
                        ],
                    )?;
                }
                for (at, check, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, trigger) in
                    &invocations
                {
                    tx.execute(
                        "INSERT INTO check_invocations
                         (store_revision, check_id, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, \"trigger\")
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        params![at, check, hook, notes, warnings, errors, outcome, executor, judge, external, rule_digest, trigger],
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
            .map_err(failed)?;
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
    use crate::domain::vo::id::ClauseId;
    use crate::infra::persist::fixture;

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("chidev-sqlite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("chidev.db")
    }

    fn changes() -> Changes {
        let p = fixture::project();
        Changes {
            pins: vec![fixture::pin(&p)],
            change_requests: fixture::change_requests(&p),
            names: vec![crate::app::ports::AssetName {
                context: ContextId::from_uuid(fixture::id(11)),
                kind: AssetKind::Interface,
                name: "stack".to_string(),
                id: fixture::id(fixture::IFACE),
            }],
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
            }],
            judged: Vec::new(),
            external: Vec::new(),
            pin_codes: Vec::new(),
            measurements: Vec::new(),
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
        assert_eq!(store.invocations().await.unwrap(), c.invocations);
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
        let invocations = store.invocations().await.unwrap();
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
        assert_eq!(store.invocations().await.unwrap(), vec![old_run]);
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
        assert_eq!(store.invocations().await.unwrap(), vec![first_run]);
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
        let ran = store.invocations().await.unwrap();
        assert_eq!(ran.len(), 1);
        assert_eq!(
            (ran[0].executor, ran[0].judge.clone()),
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
        let invocations = store.invocations().await.unwrap();
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
        assert_eq!(
            store.invocations().await.unwrap(),
            first_changes.invocations
        );
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
        conn.pragma_update(None, "user_version", 2).unwrap();
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
        d.stamp = crate::domain::vo::measure::MeasureStamp::new(
            Digest::of(b"code"),
            empty_basis(),
            format!("settings-{n}"),
        )
        .unwrap()
        .into();
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
        let read = store.invocations().await.unwrap();
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
        let read = store.invocations().await.unwrap();
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
        assert_eq!(store.invocations().await.unwrap()[0].moment, Moment::Edit);
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE check_invocations SET hook = 'edit'", [])
            .unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.invocations().await.unwrap()[0].moment, Moment::Edit);
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
}
