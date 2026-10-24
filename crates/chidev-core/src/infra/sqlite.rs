//! The store in a SQLite file, through `rusqlite-isle`: the connection lives on a thread of its own and is reached
//! by async calls, so no blocking call runs on the caller's runtime. An async call cannot hold a transaction across
//! `.await`, so [`Store::commit`] writes everything of one use case in a single `call_write` closure, after
//! checking that nothing was committed since the use case began to read.
//!
//! The file holds six tables: `meta` (the store's revision, which every commit bumps), `project`, `pins` and
//! `change_requests` (one JSON document each, in the schema the record names; see `persist`), `asset_names`, and
//! `check_invocations` (the checks each use case ran, appended and never changed). `PRAGMA user_version` is the
//! layout of the tables; a file with a higher one is refused. `check_invocations` was added without a new layout
//! number: it is created `IF NOT EXISTS` each time a file of layout 1 is opened, so a database made before it gets
//! the table and nothing else in it changes.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, params};
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver, IsleError};
use uuid::Uuid;

use super::persist::{self, CURRENT};
use crate::app::ports::{
    AssetKind, AssetNames, Changes, Invocation, Outcome, Store, StoreRevision,
};
use crate::domain::check::Hook;
use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::{
    ChangeRequestRepository, PinRepository, ProjectRepository, StoreError,
};
use crate::domain::vo::finding::RuleId;
use crate::domain::vo::id::{ChangeRequestId, ContextId, InterfaceId};

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
/// ran the check, and `store_revision` is the revision that use case read (the model the check saw).
const CREATE_INVOCATIONS: &str = "
CREATE TABLE IF NOT EXISTS check_invocations (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    store_revision INTEGER NOT NULL,
    check_id TEXT NOT NULL,
    hook TEXT NOT NULL,
    notes INTEGER NOT NULL,
    warnings INTEGER NOT NULL,
    errors INTEGER NOT NULL,
    outcome TEXT NOT NULL
);
";

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
    }
    Ok(())
}

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
        let rows: Vec<(i64, String, String, i64, i64, i64, String)> = self
            .isle
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT store_revision, check_id, hook, notes, warnings, errors, outcome
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
                    ))
                })?
                .collect()
            })
            .await
            .map_err(failed)?;
        let corrupt = |what: &str| StoreError::Corrupt(format!("an invocation: {what}"));
        rows.into_iter()
            .map(|(at, check, hook, notes, warnings, errors, outcome)| {
                Ok(Invocation {
                    check: RuleId::try_from(check.as_str()).map_err(|e| corrupt(&e.to_string()))?,
                    hook: Hook::from_word(&hook).ok_or_else(|| corrupt(&hook))?,
                    read_at: StoreRevision(u64::try_from(at).map_err(|_| corrupt("a revision"))?),
                    notes: u32::try_from(notes).map_err(|_| corrupt("a count"))?,
                    warnings: u32::try_from(warnings).map_err(|_| corrupt("a count"))?,
                    errors: u32::try_from(errors).map_err(|_| corrupt("a count"))?,
                    outcome: Outcome::from_word(&outcome).ok_or_else(|| corrupt(&outcome))?,
                })
            })
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

impl Store for SqliteStore {
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

    async fn commit(&self, read_at: StoreRevision, changes: Changes) -> Result<(), StoreError> {
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
        let invocations = changes
            .invocations
            .iter()
            .map(|i| {
                Ok((
                    i64::try_from(i.read_at.0).map_err(|_| {
                        StoreError::Failed("a revision too large to keep".to_string())
                    })?,
                    i.check.to_string(),
                    i.hook.word(),
                    i64::from(i.notes),
                    i64::from(i.warnings),
                    i64::from(i.errors),
                    i.outcome.word(),
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
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
                for (at, check, hook, notes, warnings, errors, outcome) in &invocations {
                    tx.execute(
                        "INSERT INTO check_invocations
                         (store_revision, check_id, hook, notes, warnings, errors, outcome)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![at, check, hook, notes, warnings, errors, outcome],
                    )?;
                }
                tx.execute("UPDATE meta SET revision = revision + 1 WHERE id = 1", [])?;
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
                hook: Hook::Put,
                read_at: StoreRevision(0),
                notes: 0,
                warnings: 2,
                errors: 0,
                outcome: Outcome::Found,
            }],
        }
    }

    #[tokio::test]
    async fn what_was_committed_is_there_after_reopening() {
        let path = temp_db("reopen");
        let store = SqliteStore::open(&path).await.unwrap();
        assert_eq!(store.project().await.unwrap(), None);
        let at = store.revision().await.unwrap();
        let c = changes();
        store.commit(at, c.clone()).await.unwrap();
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
        store.commit(at, changes()).await.unwrap();
        let again = store.revision().await.unwrap();
        store.commit(again, changes()).await.unwrap();
        assert_eq!(store.invocations().await.unwrap().len(), 2);
        // Committed at an old revision: refused, so its record is not kept either.
        assert_eq!(store.commit(at, changes()).await, Err(StoreError::Conflict));
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
        store.commit(at, changes()).await.unwrap();
        assert_eq!(store.invocations().await.unwrap().len(), 1);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_commit_after_another_is_refused_and_writes_nothing() {
        let path = temp_db("stale");
        let store = SqliteStore::open(&path).await.unwrap();
        let at = store.revision().await.unwrap();
        store.commit(at, Changes::default()).await.unwrap();
        let stale = store.commit(at, changes()).await;
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
        store.commit(at, changes()).await.unwrap();
        store.close().await.unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE project SET schema = 2", []).unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).await.unwrap();
        assert!(matches!(store.project().await, Err(StoreError::Corrupt(_))));
        store.close().await.unwrap();
    }
}
