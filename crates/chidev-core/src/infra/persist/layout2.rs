//! The migration of a store from layout 1 to layout 2: the Crux, which a change request held, becomes a record of its
//! own.
//!
//! It is done once, when the file is opened, and not by reading the old shape each time a document is read: an option
//! and a decision of the old Crux were named by the text of the choice and had no id, so reading them again would give
//! them another id each time. The migration gives each Crux, each option and each answer an id, and it does so in
//! `infra`, which may make ids (the domain may not).
//!
//! Each change request that held a Crux gives a Crux document. Its `raised_in` is that change request; it has no
//! findings (it was not raised from one); its question is the title of the change request, which stood for it; its
//! options are numbered in order, and the choice a recommendation and a decision named by text is replaced by the id of
//! the option with that text. A text that matches no option, and a `supersedes` that names a change request without a
//! Crux, cannot be moved, and fail the migration. Every change request document is rewritten as schema 3 (it does not
//! hold a Crux). The store's revision and the model's are not touched: the model means what it did.
//!
//! All of it is one transaction that takes the write lock first. A failure of any kind rolls it back, and the file is
//! as it was.

use std::collections::BTreeMap;

use rusqlite::{Connection, TransactionBehavior, params};
use serde::Deserialize;
use uuid::Uuid;

use super::crux::{
    AnswerDoc, CruxDoc, DecisionDoc, OptionDoc, RecommendedDoc, SCHEMA as CRUX_SCHEMA, StateDoc,
};
use super::v1::{AnswerBasisDoc, CR_SCHEMA, CruxKindDoc, ImportanceDoc, MakerDoc, OptionExpectDoc};
use super::{decode_change_request, decode_crux};
use crate::domain::service::change_request::check_requests_and_cruxes;

/// The table of the Cruxes, one JSON document each; `raised_in` is the key the Cruxes of a change request are found
/// by.
pub(crate) const CREATE_CRUXES: &str = "
CREATE TABLE IF NOT EXISTS cruxes (
    id TEXT PRIMARY KEY,
    raised_in TEXT NOT NULL,
    schema INTEGER NOT NULL,
    doc TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS cruxes_by_raised_in ON cruxes (raised_in);
";

/// Why the migration did not happen.
#[derive(Debug)]
pub(crate) enum Failure {
    /// SQLite failed.
    Store(rusqlite::Error),
    /// What the store holds cannot be moved: what did not match.
    Data(String),
}

impl From<rusqlite::Error> for Failure {
    fn from(e: rusqlite::Error) -> Failure {
        Failure::Store(e)
    }
}

/// The Crux as a change request document of schema 1 or 2 held it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OldCrux {
    kind: CruxKindDoc,
    options: Vec<OldOption>,
    recommended: Option<OldRecommended>,
    importance: Option<ImportanceDoc>,
    /// The id of the change request whose answered Crux this one replaces.
    supersedes: Option<String>,
    answer: Option<OldAnswer>,
}

#[derive(Debug, Deserialize)]
struct OldOption {
    choice: String,
    change: Option<String>,
    expects: Vec<OptionExpectDoc>,
    consequence: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OldRecommended {
    choice: String,
    reason: Option<String>,
    confidence: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OldAnswer {
    decision: OldDecision,
    basis: AnswerBasisDoc,
    by: MakerDoc,
    at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OldDecision {
    Chosen(OldReasoned),
    Worded {
        wording: String,
        reason: Option<String>,
    },
    Unspecified {
        keep: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
struct OldReasoned {
    choice: String,
    reason: Option<String>,
}

/// A change request document read for the migration: the document without its Crux, set to schema 3, and the Crux.
struct Held {
    id: String,
    title: String,
    document: serde_json::Value,
    crux: Option<OldCrux>,
}

fn data(message: impl Into<String>) -> Failure {
    Failure::Data(message.into())
}

/// Reads the change request document `doc`, kept as schema `stored`, and takes its Crux out of it.
fn take_crux(id: &str, stored: u32, doc: &str) -> Result<Held, Failure> {
    if !(1..=2).contains(&stored) {
        return Err(data(format!(
            "the change request {id} is of schema {stored}; a store of layout 1 holds schema 1 or 2"
        )));
    }
    let mut document: serde_json::Value = serde_json::from_str(doc)
        .map_err(|e| data(format!("the change request {id} is not a document: {e}")))?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| data(format!("the change request {id} is not a document")))?;
    if object.get("schema").and_then(serde_json::Value::as_u64) != Some(u64::from(stored)) {
        return Err(data(format!(
            "the change request {id} says another schema than its record, {stored}"
        )));
    }
    if object.get("id").and_then(serde_json::Value::as_str) != Some(id) {
        return Err(data(format!(
            "the change request {id} says another id than its record"
        )));
    }
    let title = object
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let crux = match object.remove("crux") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(serde_json::from_value::<OldCrux>(value).map_err(|e| {
            data(format!(
                "the crux of the change request `{title}` ({id}) cannot be read: {e}"
            ))
        })?),
    };
    object.insert("schema".to_string(), serde_json::json!(CR_SCHEMA));
    Ok(Held {
        id: id.to_string(),
        title,
        document,
        crux,
    })
}

/// The Crux document that `old` becomes, held by the change request `held`. `crux_of` is the id of the Crux of each
/// change request that has one, for `supersedes`.
fn moved(
    held: &Held,
    old: &OldCrux,
    id: Uuid,
    crux_of: &BTreeMap<String, Uuid>,
) -> Result<CruxDoc, Failure> {
    let title = &held.title;
    let options: Vec<(Uuid, &OldOption)> = old.options.iter().map(|o| (new_id(), o)).collect();
    let option_named = |choice: &str, what: &str| -> Result<String, Failure> {
        options
            .iter()
            .find(|(_, o)| o.choice == choice)
            .map(|(id, _)| id.to_string())
            .ok_or_else(|| {
                data(format!(
                    "the change request `{title}` ({}): {what} `{choice}`, which is none of the options of its crux",
                    held.id
                ))
            })
    };
    let recommended = old
        .recommended
        .as_ref()
        .map(|r| {
            Ok::<_, Failure>(RecommendedDoc {
                option: option_named(&r.choice, "its crux recommends")?,
                reason: r.reason.clone(),
                confidence: r.confidence,
            })
        })
        .transpose()?;
    let state = match &old.answer {
        None => StateDoc::Open,
        Some(a) => StateDoc::Answered(AnswerDoc {
            id: new_id().to_string(),
            decision: match &a.decision {
                OldDecision::Chosen(r) => DecisionDoc::Chosen {
                    option: option_named(&r.choice, "its decision chose")?,
                    reason: r.reason.clone(),
                },
                OldDecision::Worded { wording, reason } => DecisionDoc::Worded {
                    wording: wording.clone(),
                    reason: reason.clone(),
                },
                OldDecision::Unspecified { keep } => {
                    DecisionDoc::Unspecified { keep: keep.clone() }
                }
            },
            basis: a.basis.clone(),
            by: a.by.clone(),
            at: a.at,
        }),
    };
    let supersedes = old
        .supersedes
        .as_ref()
        .map(|replaced| {
            crux_of.get(replaced).map(Uuid::to_string).ok_or_else(|| {
                data(format!(
                    "the crux of the change request `{title}` ({}) supersedes the change request {replaced}, which has no crux in this store",
                    held.id
                ))
            })
        })
        .transpose()?;
    Ok(CruxDoc {
        schema: CRUX_SCHEMA,
        id: id.to_string(),
        raised_in: held.id.clone(),
        findings: Vec::new(),
        kind: old.kind.clone(),
        question: held.title.clone(),
        options: options
            .iter()
            .map(|(id, o)| OptionDoc {
                id: id.to_string(),
                choice: o.choice.clone(),
                change: o.change.clone(),
                expects: o.expects.clone(),
                consequence: o.consequence.clone(),
            })
            .collect(),
        recommended,
        importance: old.importance,
        supersedes,
        state,
    })
}

/// A fresh id. Made here and not in the domain: this is `infra`, which may read the clock and the random source.
fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Moves the store `conn` from layout 1 to layout 2, if it is at layout 1 when the write lock is taken; at any other
/// layout (another process may have got there first) it does nothing.
pub(crate) fn migrate(conn: &mut Connection) -> Result<(), Failure> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let layout: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if layout != 1 {
        return Ok(());
    }
    tx.execute_batch(CREATE_CRUXES)?;
    let rows: Vec<(String, u32, String)> = {
        let mut stmt = tx.prepare("SELECT id, schema, doc FROM change_requests ORDER BY id")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    let held: Vec<Held> = rows
        .iter()
        .map(|(id, stored, doc)| take_crux(id, *stored, doc))
        .collect::<Result<_, _>>()?;
    let crux_of: BTreeMap<String, Uuid> = held
        .iter()
        .filter(|h| h.crux.is_some())
        .map(|h| (h.id.clone(), new_id()))
        .collect();
    let mut cruxes = Vec::new();
    for h in &held {
        if let (Some(old), Some(id)) = (&h.crux, crux_of.get(&h.id)) {
            cruxes.push(moved(h, old, *id, &crux_of)?);
        }
    }
    let requests: Vec<(&str, String)> = held
        .iter()
        .map(|h| {
            serde_json::to_string(&h.document)
                .map(|text| (h.id.as_str(), text))
                .map_err(|e| {
                    data(format!(
                        "the change request {} cannot be written: {e}",
                        h.id
                    ))
                })
        })
        .collect::<Result<_, _>>()?;
    let crux_texts: Vec<(&CruxDoc, String)> = cruxes
        .iter()
        .map(|c| {
            serde_json::to_string(c)
                .map(|text| (c, text))
                .map_err(|e| data(format!("the crux {} cannot be written: {e}", c.id)))
        })
        .collect::<Result<_, _>>()?;
    // What is to be written is read back as the domain, as the store will read it; a store that would not open
    // after the migration is not migrated.
    let read =
        |what: String| move |e: crate::domain::repository::StoreError| data(format!("{what}: {e}"));
    let crs = requests
        .iter()
        .map(|(id, text)| {
            decode_change_request(CR_SCHEMA, text)
                .map_err(read(format!("the change request {id} after the migration")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let moved_cruxes = crux_texts
        .iter()
        .map(|(c, text)| {
            decode_crux(CRUX_SCHEMA, text)
                .map_err(read(format!("the crux {} after the migration", c.id)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    check_requests_and_cruxes(&crs, &moved_cruxes).map_err(|e| {
        data(format!(
            "the change requests and their cruxes after the migration: {e}"
        ))
    })?;
    for (c, text) in &crux_texts {
        tx.execute(
            "INSERT INTO cruxes (id, raised_in, schema, doc) VALUES (?1, ?2, ?3, ?4)",
            params![c.id, c.raised_in, CRUX_SCHEMA, text],
        )?;
    }
    for (id, text) in &requests {
        tx.execute(
            "UPDATE change_requests SET schema = ?2, doc = ?3 WHERE id = ?1",
            params![id, CR_SCHEMA, text],
        )?;
    }
    tx.execute_batch("PRAGMA user_version = 2")?;
    tx.commit()?;
    Ok(())
}
