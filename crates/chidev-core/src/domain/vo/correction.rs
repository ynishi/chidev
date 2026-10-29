//! A forced correction: a text that must not stay in a record is replaced by a mark, and the record keeps a step that
//! says which fields were corrected, why, by whom and when. The text that was replaced is in none of it.
//!
//! The text sought ([`SoughtText`]) is exact bytes, is never printed, and is not a part of a [`Correction`] or of any
//! error: an error names a field by its word, so no refusal, answer or history step can carry the text. A record
//! says where it keeps text ([`Correctable::texts`]) and applies a correction ([`Correctable::correct`]); the two
//! read one list of fields, so a place found is a place that can be corrected.

use std::fmt;

use thiserror::Error;

use super::answer::DecisionMaker;
use super::prose::Prose;
use super::timestamp::Timestamp;

/// What replaces the text sought. Every mark starts with it; a field that must stay distinct from others takes a
/// numbered one when this one is taken ([`mark_for`]).
pub const MARK: &str = "[redacted]";

/// How many marks [`mark_for`] tries before it gives up on a value that no mark makes free of the text.
const MARK_TRIES: u32 = 10_000;

/// Why a correction is refused. Nothing is changed. A string is the word of a field, never a text of a record.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CorrectionError {
    /// The text sought is empty.
    #[error("the text sought is empty")]
    EmptyText,
    /// The text sought is a part of the mark, so every mark would be found again.
    #[error("the text sought is a part of the mark")]
    TextInMark,
    /// A correction names no field.
    #[error("a correction names no field")]
    NoField,
    /// A field is named twice.
    #[error("the field `{0}` is named twice")]
    FieldTwice(String),
    /// The record has no such field, or the field is not set.
    #[error("the record has no field `{0}`")]
    NoSuchField(String),
    /// The text sought is not in the field.
    #[error("the text sought is not in the field `{0}`")]
    TextAbsent(String),
    /// The record refuses the field once the text is replaced (or the text would remain in it).
    #[error("the field `{0}` is not valid once the text is replaced")]
    Invalid(String),
}

/// The text a correction looks for. It is exact (no trim, case-sensitive) and never printed: `Debug` hides it and
/// there is no `Display`, so it cannot reach an error, a log or an answer.
#[derive(Clone, PartialEq, Eq)]
pub struct SoughtText(String);

impl fmt::Debug for SoughtText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SoughtText(..)")
    }
}

impl TryFrom<&str> for SoughtText {
    type Error = CorrectionError;

    /// Refuses an empty text, and a text that is a part of the [`MARK`]: otherwise every mark would be found again and
    /// a correction would never be seen to end. A text that merely contains the mark is accepted.
    fn try_from(text: &str) -> Result<SoughtText, CorrectionError> {
        if text.is_empty() {
            return Err(CorrectionError::EmptyText);
        }
        if MARK.contains(text) {
            return Err(CorrectionError::TextInMark);
        }
        Ok(SoughtText(text.to_string()))
    }
}

impl SoughtText {
    /// Whether the text is in `s`.
    pub fn occurs_in(&self, s: &str) -> bool {
        s.contains(self.0.as_str())
    }

    /// `s` with every occurrence of the text replaced by `mark`; none when the text is not in `s`.
    pub fn replace_in(&self, s: &str, mark: &str) -> Option<String> {
        self.occurs_in(s).then(|| s.replace(self.0.as_str(), mark))
    }
}

/// `value` with the text sought replaced by the first mark (the [`MARK`], then `[redacted 2]`, `[redacted 3]`, ...)
/// whose result is not `taken` and does not hold the text. It is for a value that must stay different from others
/// (a title among the titles, a choice among the choices of a Crux), so that a correction is never refused for a
/// name already given. It is called only on a value that holds the text. When no mark makes the value free of the
/// text (the text is made by the words around the mark), the last result is given and the caller refuses it.
pub fn mark_for(sought: &SoughtText, value: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut result = String::new();
    for n in 1..=MARK_TRIES {
        let mark = if n == 1 {
            MARK.to_string()
        } else {
            format!("[redacted {n}]")
        };
        result = value.replace(sought.0.as_str(), &mark);
        if !taken(&result) && !sought.occurs_in(&result) {
            break;
        }
    }
    result
}

/// `text` with the text sought replaced by the plain [`MARK`]. The field is named by its `word` in the refusal:
/// `TextAbsent` when the text is not in it, `Invalid` when it would remain (the words around the mark make it).
pub fn masked(sought: &SoughtText, word: &str, text: &str) -> Result<String, CorrectionError> {
    let replaced = sought
        .replace_in(text, MARK)
        .ok_or_else(|| CorrectionError::TextAbsent(word.to_string()))?;
    free_of(sought, word, replaced)
}

/// `text` with the text sought replaced by a mark that [`mark_for`] picks, for a field that must stay distinct.
pub fn masked_distinct(
    sought: &SoughtText,
    word: &str,
    text: &str,
    taken: impl Fn(&str) -> bool,
) -> Result<String, CorrectionError> {
    if !sought.occurs_in(text) {
        return Err(CorrectionError::TextAbsent(word.to_string()));
    }
    free_of(sought, word, mark_for(sought, text, taken))
}

/// `replaced`, when the text sought is not in it; `Invalid` for the field `word` when it is.
fn free_of(sought: &SoughtText, word: &str, replaced: String) -> Result<String, CorrectionError> {
    if sought.occurs_in(&replaced) {
        return Err(CorrectionError::Invalid(word.to_string()));
    }
    Ok(replaced)
}

/// The step a record keeps of a correction: the fields corrected, why, who said so, and when. The text replaced is
/// not in it, by type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction<F> {
    fields: Vec<F>,
    reason: Prose,
    by: DecisionMaker,
    at: Timestamp,
}

impl<F: PartialEq + fmt::Display> Correction<F> {
    /// A correction of `fields`, which are at least one and none twice.
    pub fn new(
        fields: Vec<F>,
        reason: Prose,
        by: DecisionMaker,
        at: Timestamp,
    ) -> Result<Correction<F>, CorrectionError> {
        if fields.is_empty() {
            return Err(CorrectionError::NoField);
        }
        for (k, field) in fields.iter().enumerate() {
            if fields[..k].contains(field) {
                return Err(CorrectionError::FieldTwice(field.to_string()));
            }
        }
        Ok(Correction {
            fields,
            reason,
            by,
            at,
        })
    }
}

impl<F> Correction<F> {
    /// The fields corrected, in the order given.
    pub fn fields(&self) -> &[F] {
        &self.fields
    }

    /// Why.
    pub fn reason(&self) -> &Prose {
        &self.reason
    }

    /// Who said so.
    pub fn by(&self) -> &DecisionMaker {
        &self.by
    }

    /// When.
    pub fn at(&self) -> Timestamp {
        self.at
    }
}

/// A record whose texts can be corrected. `texts` is the one list of where it keeps text, read by the search for a
/// text and by the correction alike.
pub trait Correctable {
    /// What names a place of text in the record.
    type Field: Clone + Eq + Ord + fmt::Display;

    /// Every text field that is set, in a fixed order.
    fn texts(&self) -> Vec<(Self::Field, &str)>;

    /// Replaces the text sought, in each field the `correction` names, by a mark, and appends the correction to the
    /// history of the record. Each field must be one of [`Correctable::texts`] (`NoSuchField`) and hold the text
    /// (`TextAbsent`). The replacements are made on a copy, which the record's own constructors read again
    /// (`Invalid` for a field they refuse), and the record is replaced only when all of it succeeded: on an error it
    /// is as it was.
    ///
    /// `taken(field, candidate)` says whether a value for a field that must stay distinct is held already by a value
    /// outside the record (the title of another change request). A record answers for its own parts itself.
    fn correct(
        &mut self,
        sought: &SoughtText,
        correction: Correction<Self::Field>,
        taken: &dyn Fn(&Self::Field, &str) -> bool,
    ) -> Result<(), CorrectionError>;
}

/// Checks that each field `fields` names is one of the texts of `record` and holds the text sought. It is the first
/// step of every [`Correctable::correct`].
pub fn check_places<R: Correctable>(
    record: &R,
    sought: &SoughtText,
    fields: &[R::Field],
) -> Result<(), CorrectionError> {
    let texts = record.texts();
    for field in fields {
        match texts.iter().find(|(f, _)| f == field) {
            None => return Err(CorrectionError::NoSuchField(field.to_string())),
            Some((_, text)) if !sought.occurs_in(text) => {
                return Err(CorrectionError::TextAbsent(field.to_string()));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sought(text: &str) -> SoughtText {
        SoughtText::try_from(text).unwrap()
    }

    #[test]
    fn a_sought_text_is_exact_and_not_empty_nor_a_part_of_the_mark() {
        assert_eq!(SoughtText::try_from(""), Err(CorrectionError::EmptyText));
        for in_mark in ["red", "[redacted]", "[", "]", "d]"] {
            assert_eq!(
                SoughtText::try_from(in_mark),
                Err(CorrectionError::TextInMark)
            );
        }
        // no trim: a space is a text
        assert!(SoughtText::try_from(" ").is_ok());
        // a text that contains the mark is accepted: replacing it leaves only the mark
        assert!(SoughtText::try_from("a[redacted]").is_ok());
        assert!(SoughtText::try_from("redacted!").is_ok());
    }

    #[test]
    fn a_sought_text_is_never_printed() {
        let s = sought("tok-SECRET-123");
        assert_eq!(format!("{s:?}"), "SoughtText(..)");
        assert_eq!(format!("{s:#?}"), "SoughtText(..)");
        // a correction error names fields, and the text is in none of them
        for e in [
            CorrectionError::EmptyText,
            CorrectionError::TextInMark,
            CorrectionError::NoField,
            CorrectionError::FieldTwice("title".into()),
            CorrectionError::NoSuchField("situation".into()),
            CorrectionError::TextAbsent("title".into()),
            CorrectionError::Invalid("title".into()),
        ] {
            assert!(!format!("{e} {e:?}").contains("SECRET"));
        }
    }

    #[test]
    fn a_text_is_found_and_every_occurrence_is_replaced() {
        let s = sought("ab");
        assert!(s.occurs_in("xxabxx"));
        assert!(!s.occurs_in("AB"));
        assert_eq!(s.replace_in("ab-ab", "#").as_deref(), Some("#-#"));
        assert_eq!(s.replace_in("nothing", "#"), None);
    }

    #[test]
    fn a_mark_is_numbered_while_the_result_is_taken() {
        let s = sought("tok");
        assert_eq!(mark_for(&s, "tok", |_| false), "[redacted]");
        assert_eq!(mark_for(&s, "tok", |c| c == "[redacted]"), "[redacted 2]");
        assert_eq!(
            mark_for(&s, "a tok", |c| c == "a [redacted]"
                || c == "a [redacted 2]"),
            "a [redacted 3]"
        );
    }

    #[test]
    fn a_numbered_mark_that_holds_the_text_is_skipped() {
        // the text is a digit: `[redacted 2]` holds it
        let s = sought("2");
        assert_eq!(mark_for(&s, "2", |c| c == "[redacted]"), "[redacted 3]");
    }

    #[test]
    fn a_text_made_by_the_words_around_the_mark_is_refused_not_looped_on() {
        // `x[r` remains in `x[redacted]` whatever the number is
        let s = sought("x[r");
        assert_eq!(
            masked(&s, "title", "xx[r"),
            Err(CorrectionError::Invalid("title".into()))
        );
        assert_eq!(
            masked_distinct(&s, "title", "xx[r", |_| false),
            Err(CorrectionError::Invalid("title".into()))
        );
    }

    #[test]
    fn masking_names_the_field_when_the_text_is_absent() {
        let s = sought("tok");
        assert_eq!(
            masked(&s, "situation", "a tok b").unwrap(),
            "a [redacted] b"
        );
        assert_eq!(
            masked(&s, "situation", "nothing"),
            Err(CorrectionError::TextAbsent("situation".into()))
        );
        assert_eq!(
            masked_distinct(&s, "title", "nothing", |_| false),
            Err(CorrectionError::TextAbsent("title".into()))
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct Word(&'static str);

    impl fmt::Display for Word {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    fn correction(fields: Vec<Word>) -> Result<Correction<Word>, CorrectionError> {
        Correction::new(
            fields,
            Prose::try_from("a token was pasted").unwrap(),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(7),
        )
    }

    #[test]
    fn a_correction_names_at_least_one_field_and_none_twice() {
        assert_eq!(correction(Vec::new()), Err(CorrectionError::NoField));
        assert_eq!(
            correction(vec![Word("a"), Word("b"), Word("a")]),
            Err(CorrectionError::FieldTwice("a".into()))
        );
        let c = correction(vec![Word("b"), Word("a")]).unwrap();
        assert_eq!(c.fields(), [Word("b"), Word("a")]);
        assert_eq!(c.reason().as_str(), "a token was pasted");
        assert_eq!(c.by().to_string(), "person alice");
        assert_eq!(c.at().millis(), 7);
    }

    struct Record(Vec<(Word, String)>);

    impl Correctable for Record {
        type Field = Word;

        fn texts(&self) -> Vec<(Word, &str)> {
            self.0
                .iter()
                .map(|(f, t)| (f.clone(), t.as_str()))
                .collect()
        }

        fn correct(
            &mut self,
            _: &SoughtText,
            _: Correction<Word>,
            _: &dyn Fn(&Word, &str) -> bool,
        ) -> Result<(), CorrectionError> {
            Ok(())
        }
    }

    #[test]
    fn the_places_are_checked_against_the_texts_the_record_lists() {
        let r = Record(vec![
            (Word("a"), "has tok".to_string()),
            (Word("b"), "has none".to_string()),
        ]);
        let s = sought("tok");
        assert_eq!(check_places(&r, &s, &[Word("a")]), Ok(()));
        assert_eq!(
            check_places(&r, &s, &[Word("b")]),
            Err(CorrectionError::TextAbsent("b".into()))
        );
        assert_eq!(
            check_places(&r, &s, &[Word("a"), Word("c")]),
            Err(CorrectionError::NoSuchField("c".into()))
        );
    }
}
