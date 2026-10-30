//! A forced correction of records: finding where a text that must not stay is kept, and replacing it by a mark.
//!
//! The text sought is never in an answer, a refusal or a record: a place is a record and a field by their words
//! ([`Place`]), a refusal names a record by its id (a title may hold the text) and a field by its word, and the step
//! a record keeps of a correction holds the fields, the reason, who and when, and no text. The use case that
//! corrects ([`App::correct_records`]) enters through its own kind of trigger ([`CorrectTrigger`]); finding is a
//! read and keeps nothing.

use std::fmt::{self, Display};

use super::commit::{CorrectTrigger, CorrectionOrder};
use super::ports::external::ExternalExecutor;
use super::ports::{IdSource, InvocationField, InvocationSeq, Store};
use super::{App, AppError, DecisionMakerInput, Names, maker_of};
use crate::domain::entity::change_request::ChangeRequestField;
use crate::domain::entity::crux::CruxField;
use crate::domain::entity::measurement::MeasurementField;
use crate::domain::repository::StoreError;
use crate::domain::rule::{CORRECT_TEXT_IN_REASON, CORRECT_UNKNOWN_RECORD};
use crate::domain::violation::{Value, Violation};
use crate::domain::vo::answer::DecisionMaker;
use crate::domain::vo::correction::{Correctable, CorrectionError, SoughtText};
use crate::domain::vo::id::{ChangeRequestId, CruxId, MeasurementId};
use crate::domain::vo::prose::Prose;
use crate::domain::vo::timestamp::Timestamp;

/// Where a text is kept: a record, named by its id, and a field of it. The text is not in it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Place {
    /// A field of a change request.
    ChangeRequest {
        /// The change request.
        id: ChangeRequestId,
        /// The field.
        field: ChangeRequestField,
    },
    /// A field of a Crux.
    Crux {
        /// The Crux.
        id: CruxId,
        /// The field.
        field: CruxField,
    },
    /// A field of a measurement.
    Measurement {
        /// The measurement.
        id: MeasurementId,
        /// The field.
        field: MeasurementField,
    },
    /// A field of a row of the record of check runs.
    Invocation {
        /// The row, by the order it was kept in.
        seq: InvocationSeq,
        /// The field.
        field: InvocationField,
    },
}

impl Place {
    /// The kind of record this place is in.
    pub fn kind(&self) -> RecordKind {
        match self {
            Place::ChangeRequest { .. } => RecordKind::ChangeRequest,
            Place::Crux { .. } => RecordKind::Crux,
            Place::Measurement { .. } => RecordKind::Measurement,
            Place::Invocation { .. } => RecordKind::Invocation,
        }
    }
}

/// A kind of record a correction can change: the one statement of their words, which every refusal, diagnostic and
/// wire form that names a kind takes from [`RecordKind::word`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecordKind {
    /// A change request.
    ChangeRequest,
    /// A Crux.
    Crux,
    /// A measurement.
    Measurement,
    /// A row of the record of check runs.
    Invocation,
}

impl RecordKind {
    /// The word of the kind: `change_request`, `crux`, `measurement` or `invocation`.
    pub fn word(self) -> &'static str {
        match self {
            RecordKind::ChangeRequest => "change_request",
            RecordKind::Crux => "crux",
            RecordKind::Measurement => "measurement",
            RecordKind::Invocation => "invocation",
        }
    }
}

/// A correction asked for: replace `text` by a mark in each of the `places`.
#[derive(Clone, PartialEq, Eq)]
pub struct CorrectInput {
    /// The text that must not stay, exactly as it is kept (no white space is trimmed).
    pub text: String,
    /// Why, in the person's words; kept in each record's step. It does not hold `text`.
    pub reason: String,
    /// Who says so: a claim that nothing checks.
    pub by: DecisionMakerInput,
    /// The places to replace it in, as [`App::find_text`] lists them; at least one, none twice.
    pub places: Vec<Place>,
}

/// The text is the one thing that does not show in a debug print.
impl fmt::Debug for CorrectInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CorrectInput")
            .field("text", &"..")
            .field("reason", &self.reason)
            .field("by", &self.by)
            .field("places", &self.places)
            .finish()
    }
}

/// Whether what the store freed was cleared from its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cleared {
    /// The file was written again without it ([`Store::clear_freed`]).
    Done,
    /// The correction is kept, but the file was not cleared; `reason` says why. What the file still holds of the
    /// text is not known to the correction.
    NotFinished {
        /// Why, in the store's words. Clearing writes the file again and holds no record, so the words are those of
        /// the database engine and hold no text of a record.
        reason: String,
    },
}

/// The places that still hold the text after a correction. The search is made once the correction is kept, and
/// the correction does not fail because it could not be made: the correction is kept either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remaining {
    /// The search was made. The places are those not named, or in a record the correction did not know to name;
    /// empty when the text is gone from the records.
    Found(Vec<Place>),
    /// The search could not be made (the store changed or could not be read); `reason` says why in a fixed sentence
    /// per kind of failure ([`why_unsearched`]), never the store's own words, which on a corrupt document may quote
    /// a text of a record. Run `text_find` to look again.
    Unknown {
        /// Why.
        reason: String,
    },
}

/// What a correction did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectAnswer {
    /// The places the text was replaced in: change requests first, then Cruxes, measurements and rows of the record
    /// of runs, each by id, the fields of a record in the order they were given.
    pub corrected: Vec<Place>,
    /// The places that still hold the text after it, if they could be searched for.
    pub remaining: Remaining,
    /// Whether the store's file was cleared of what it freed.
    pub cleared: Cleared,
}

/// The refusal of a violation: it names nothing by title, so the diagnostic needs no name from the store.
fn violated(violation: Violation) -> AppError {
    AppError::Violated {
        violations: vec![violation],
        names: Names::default(),
    }
}

/// The refusal of a correction that is not about a record yet (the text, the places given as a list).
fn refused_plain(error: &CorrectionError) -> AppError {
    violated(error.violation(None))
}

/// The refusal `error` of a correction of a record of this kind with this `id`. The record is named by its id
/// and kind only: no title or name is looked up, because a title may hold the text sought and a refusal never shows
/// a text of a record.
pub(super) fn refusal(record: RecordKind, id: impl Display, error: &CorrectionError) -> AppError {
    violated(
        error
            .violation(None)
            .with_value("record", Value::Word(record.word()))
            .with_value("id", Value::Text(id.to_string())),
    )
}

/// The refusal of a correction that names a record the store does not have.
pub(super) fn unknown_record(record: RecordKind, id: impl Display) -> AppError {
    violated(
        Violation::of(CORRECT_UNKNOWN_RECORD)
            .with_value("record", Value::Word(record.word()))
            .with_value("id", Value::Text(id.to_string())),
    )
}

/// Why a search for the text could not be made, as one fixed sentence per kind of failure. It is built from the kind
/// alone: the words of the error are not used, since they may quote a text of a record.
fn why_unsearched(error: &AppError) -> &'static str {
    match error {
        AppError::Store(StoreError::Conflict) => "the store changed while it was searched",
        AppError::Store(StoreError::Corrupt(_)) => "the store holds a record that cannot be read",
        AppError::Store(StoreError::Failed(_)) => "the store could not be read",
        _ => "the store could not be searched",
    }
}

/// The text sought, or the refusal for a text that cannot be sought.
fn sought_of(text: &str) -> Result<SoughtText, AppError> {
    SoughtText::try_from(text).map_err(|e| refused_plain(&e))
}

/// Groups the places by record. A field named twice for one record is refused, as is an empty list.
fn order_of(
    places: &[Place],
    sought: SoughtText,
    reason: Prose,
    by: DecisionMaker,
    at: Timestamp,
) -> Result<CorrectionOrder, AppError> {
    if places.is_empty() {
        return Err(refused_plain(&CorrectionError::NoField));
    }
    let mut order = CorrectionOrder {
        sought,
        reason,
        by,
        at,
        requests: Default::default(),
        cruxes: Default::default(),
        measurements: Default::default(),
        invocations: Default::default(),
    };
    for place in places {
        let twice = match place {
            Place::ChangeRequest { id, field } => {
                push_once(order.requests.entry(*id).or_default(), *field)
            }
            Place::Crux { id, field } => push_once(order.cruxes.entry(*id).or_default(), *field),
            Place::Measurement { id, field } => {
                push_once(order.measurements.entry(*id).or_default(), *field)
            }
            Place::Invocation { seq, field } => {
                push_once(order.invocations.entry(*seq).or_default(), *field)
            }
        };
        if let Some(word) = twice {
            return Err(refused_plain(&CorrectionError::FieldTwice(word)));
        }
    }
    Ok(order)
}

/// Adds `field` to `fields`; the word of the field when it was there already.
fn push_once<F: PartialEq + Display>(fields: &mut Vec<F>, field: F) -> Option<String> {
    if fields.contains(&field) {
        return Some(field.to_string());
    }
    fields.push(field);
    None
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Lists where `text` is kept in the records only a correction can change: for each place, the record (by id) and
    /// the field, never the text. The places are the change requests', the Cruxes', the measurements' and the rows of
    /// the record of runs', in that order, and every field that holds the text is listed, so what is listed is what
    /// [`App::correct_records`] can replace. Nothing is kept.
    ///
    /// A text that is empty, or a part of the mark that replaces a text, is refused (`correct.text-empty`,
    /// `correct.text-in-mark`).
    pub async fn find_text(&self, text: &str) -> Result<Vec<Place>, AppError> {
        let sought = sought_of(text)?;
        self.places_of(&sought).await
    }

    /// Replaces `input.text` by a mark in each of `input.places`, and keeps the correction (the fields, the reason,
    /// who and when, and no text) as a step of each record's history. It is forced: no check runs and none refuses
    /// it, and it changes nothing but the texts of the records named. A place that does not hold the text, a field
    /// the record has not, a record that is not there, a place named twice or none at all, are refused whole
    /// (`correct.*`), and nothing is changed.
    ///
    /// When it is kept, the store's file is cleared of what it freed ([`Store::clear_freed`]); that it was not is
    /// said in the answer ([`Cleared::NotFinished`]) and is not a failure of the correction. The answer lists the
    /// places corrected and the places that still hold the text.
    ///
    /// The reason and the name of who corrects are kept in the records, so they may not hold the text.
    pub async fn correct_records(
        &mut self,
        input: &CorrectInput,
    ) -> Result<CorrectAnswer, AppError> {
        let now = self.clock.now();
        let at = self.store.revision().await?;
        let sought = sought_of(&input.text)?;
        let reason = Prose::try_from(input.reason.as_str()).map_err(super::input)?;
        let by = maker_of(&input.by)?;
        // They are kept in the records, so they may not hold the text.
        for (field, held) in [("reason", reason.as_str()), ("by", by.name().as_str())] {
            if sought.occurs_in(held) {
                return Err(violated(
                    Violation::of(CORRECT_TEXT_IN_REASON).with_value("field", Value::Word(field)),
                ));
            }
        }
        let order = order_of(&input.places, sought.clone(), reason, by, now)?;
        // From here the correction is kept: nothing below fails the call.
        let corrected = self
            .commit_correct(CorrectTrigger::CorrectRecords, at, order)
            .await?;
        let cleared = match self.store.clear_freed().await {
            Ok(()) => Cleared::Done,
            Err(e) => Cleared::NotFinished {
                reason: e.to_string(),
            },
        };
        let remaining = match self.places_of(&sought).await {
            Ok(places) => Remaining::Found(places),
            Err(e) => Remaining::Unknown {
                reason: why_unsearched(&e).to_string(),
            },
        };
        Ok(CorrectAnswer {
            corrected,
            remaining,
            cleared,
        })
    }

    /// Every place that holds the text sought, in the order of the kinds of record, read at one revision.
    async fn places_of(&self, sought: &SoughtText) -> Result<Vec<Place>, AppError> {
        let at = self.store.revision().await?;
        let (change_requests, cruxes) = self.requests_and_cruxes().await?;
        let measurements = self.store.measurements().await?;
        let invocations = self.store.invocations().await?;
        if self.store.revision().await? != at {
            return Err(AppError::Store(StoreError::Conflict));
        }
        let mut places = Vec::new();
        for cr in &change_requests {
            for (field, text) in cr.texts() {
                if sought.occurs_in(text) {
                    places.push(Place::ChangeRequest { id: cr.id(), field });
                }
            }
        }
        for crux in &cruxes {
            for (field, text) in crux.texts() {
                if sought.occurs_in(text) {
                    places.push(Place::Crux {
                        id: crux.id(),
                        field,
                    });
                }
            }
        }
        for m in &measurements {
            for (field, text) in m.texts() {
                if sought.occurs_in(text) {
                    places.push(Place::Measurement { id: m.id(), field });
                }
            }
        }
        for (seq, run) in &invocations {
            for (field, text) in run.texts() {
                if sought.occurs_in(text) {
                    places.push(Place::Invocation { seq: *seq, field });
                }
            }
        }
        Ok(places)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::name::ContextName;

    #[test]
    fn the_reason_a_search_failed_is_fixed_and_holds_no_text_of_the_store() {
        let quoted = "tok-123".to_string();
        for (error, sentence) in [
            (
                AppError::Store(StoreError::Conflict),
                "the store changed while it was searched",
            ),
            (
                AppError::Store(StoreError::Corrupt(quoted.clone())),
                "the store holds a record that cannot be read",
            ),
            (
                AppError::Store(StoreError::Failed(quoted.clone())),
                "the store could not be read",
            ),
            (
                AppError::UnknownContext(ContextName::try_from(quoted.as_str()).unwrap()),
                "the store could not be searched",
            ),
        ] {
            assert_eq!(why_unsearched(&error), sentence);
            assert!(!why_unsearched(&error).contains(&quoted));
        }
    }

    #[test]
    fn a_place_is_in_the_kind_of_record_that_has_its_word() {
        let place = Place::Invocation {
            seq: InvocationSeq(1),
            field: InvocationField::Stderr,
        };
        assert_eq!(place.kind(), RecordKind::Invocation);
        assert_eq!(place.kind().word(), "invocation");
    }
}
