//! What a measurement is made of: the kind of measure, a mutant and what became of it, how a run failed, and the
//! stamp that ties a run to the thing it measured. A measurement is a fact about code that was run; the rules that
//! judge it read these values and never run anything.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use thiserror::Error;

use super::digest::Digest;
use super::pin_basis::PinBasis;

/// Why a value of a measurement is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MeasureError {
    /// A mutant key is blank, too long, or holds white space or control characters.
    #[error("a mutant key is 1 to 200 characters without white space: `{0}`")]
    BadKey(String),
    /// An operator class is not lower-case words.
    #[error("an operator class is 1 to 32 characters of a-z, 0-9 and `-`: `{0}`")]
    BadOperator(String),
    /// A site is before the first line or column.
    #[error("a site starts at line 1, column 1 or later")]
    BadSite,
    /// A mutant changes nothing.
    #[error("a mutant changes the text it replaces into another one")]
    NoChange,
    /// Two mutants of one run have one key.
    #[error("the mutant key `{0}` is given twice")]
    DuplicateKey(String),
    /// A run has more mutants than sites.
    #[error("{mutants} mutants from {sites} sites")]
    MoreMutantsThanSites {
        /// The mutants.
        mutants: usize,
        /// The sites.
        sites: u32,
    },
    /// A failure has no message.
    #[error("a failure says what failed")]
    BlankFailure,
    /// A stamp has no settings.
    #[error("a stamp names the settings the run was made with")]
    BlankSettings,
}

/// What a measure measures. The first kind is mutation testing; the others (a simulation) will join it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeasureKind {
    /// Single-site mutants of the code, run against the Harness.
    Mutate,
}

impl MeasureKind {
    /// Every kind.
    pub const ALL: [MeasureKind; 1] = [MeasureKind::Mutate];

    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            MeasureKind::Mutate => "mutate",
        }
    }

    /// The kind a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<MeasureKind> {
        Self::ALL.into_iter().find(|k| k.word() == word)
    }
}

/// The stable id of a mutant, made by the provider: it names the mutant across runs, so it is kept as it came.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MutantKey(String);

impl MutantKey {
    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for MutantKey {
    type Error = MeasureError;

    fn try_from(text: &str) -> Result<MutantKey, MeasureError> {
        let ok = !text.is_empty()
            && text.chars().count() <= 200
            && !text.chars().any(|c| c.is_whitespace() || c.is_control());
        if ok {
            Ok(MutantKey(text.to_string()))
        } else {
            Err(MeasureError::BadKey(text.to_string()))
        }
    }
}

impl std::fmt::Display for MutantKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The class of change a mutant makes (`rel`, `eq`, `arith`, ...), as the provider names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Operator(String);

impl Operator {
    /// The class as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Operator {
    type Error = MeasureError;

    fn try_from(text: &str) -> Result<Operator, MeasureError> {
        let ok = !text.is_empty()
            && text.len() <= 32
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok {
            Ok(Operator(text.to_string()))
        } else {
            Err(MeasureError::BadOperator(text.to_string()))
        }
    }
}

impl std::fmt::Display for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where in the code a mutant is: a line and a column, both from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Site {
    line: u32,
    column: u32,
}

impl Site {
    /// The site at `line` and `column`, counted from 1.
    pub fn new(line: u32, column: u32) -> Result<Site, MeasureError> {
        if line == 0 || column == 0 {
            return Err(MeasureError::BadSite);
        }
        Ok(Site { line, column })
    }

    /// The line.
    pub fn line(self) -> u32 {
        self.line
    }

    /// The column.
    pub fn column(self) -> u32 {
        self.column
    }
}

/// A single-site change of the code, as a provider proposed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutant {
    key: MutantKey,
    site: Site,
    operator: Operator,
    from: String,
    to: String,
}

impl Mutant {
    /// The change of `from` into `to` at `site`.
    pub fn new(
        key: MutantKey,
        site: Site,
        operator: Operator,
        from: String,
        to: String,
    ) -> Result<Mutant, MeasureError> {
        if from == to {
            return Err(MeasureError::NoChange);
        }
        Ok(Mutant {
            key,
            site,
            operator,
            from,
            to,
        })
    }

    /// Its key.
    pub fn key(&self) -> &MutantKey {
        &self.key
    }

    /// Where it is.
    pub fn site(&self) -> Site {
        self.site
    }

    /// The class of change.
    pub fn operator(&self) -> &Operator {
        &self.operator
    }

    /// The text it replaces.
    pub fn from(&self) -> &str {
        &self.from
    }

    /// The text it puts in.
    pub fn to(&self) -> &str {
        &self.to
    }

    /// The mutant in a line: its key, where it is and what it changes.
    pub fn describe(&self) -> String {
        format!(
            "{}: line {}, column {}, {} `{}` -> `{}`",
            self.key, self.site.line, self.site.column, self.operator, self.from, self.to
        )
    }
}

/// What became of a mutant when the Harness ran against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MutantOutcome {
    /// The code with the change did not build: no test could see it, and it says nothing of the tests.
    Stillborn,
    /// A test failed: the Harness noticed the change.
    Killed,
    /// Every test passed: the Harness did not notice the change.
    Survived,
    /// The build or the tests did not end in time.
    Timeout,
}

impl MutantOutcome {
    /// Every outcome.
    pub const ALL: [MutantOutcome; 4] = [
        MutantOutcome::Stillborn,
        MutantOutcome::Killed,
        MutantOutcome::Survived,
        MutantOutcome::Timeout,
    ];

    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            MutantOutcome::Stillborn => "stillborn",
            MutantOutcome::Killed => "killed",
            MutantOutcome::Survived => "survived",
            MutantOutcome::Timeout => "timeout",
        }
    }

    /// The outcome a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<MutantOutcome> {
        Self::ALL.into_iter().find(|o| o.word() == word)
    }
}

/// A mutant and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutantResult {
    /// The change.
    pub mutant: Mutant,
    /// What became of it.
    pub outcome: MutantOutcome,
}

/// How a run came to fail: no measurement of the code was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FailureKind {
    /// The code and the Harness, before any change, did not build or did not pass: nothing to measure against.
    Baseline,
    /// The provider could not be run or did not finish.
    Provider,
    /// The provider's reply broke the protocol (a span that does not hold its `from`, a key given twice).
    Protocol,
    /// The run was stopped by a deadline.
    Timeout,
    /// The run used up its time budget.
    Budget,
    /// The copy of the project, or a program, could not be made or run.
    Io,
}

impl FailureKind {
    /// Every kind.
    pub const ALL: [FailureKind; 6] = [
        FailureKind::Baseline,
        FailureKind::Provider,
        FailureKind::Protocol,
        FailureKind::Timeout,
        FailureKind::Budget,
        FailureKind::Io,
    ];

    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            FailureKind::Baseline => "baseline",
            FailureKind::Provider => "provider",
            FailureKind::Protocol => "protocol",
            FailureKind::Timeout => "timeout",
            FailureKind::Budget => "budget",
            FailureKind::Io => "io",
        }
    }

    /// The kind a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<FailureKind> {
        Self::ALL.into_iter().find(|k| k.word() == word)
    }
}

/// A failed run: its kind and what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureFailure {
    kind: FailureKind,
    message: String,
}

impl MeasureFailure {
    /// A failure of `kind` with a message that is not blank.
    pub fn new(
        kind: FailureKind,
        message: impl Into<String>,
    ) -> Result<MeasureFailure, MeasureError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(MeasureError::BlankFailure);
        }
        Ok(MeasureFailure { kind, message })
    }

    /// A failure of `kind`. A blank message is replaced by the kind's word, so a failure that has to be recorded
    /// is never lost for want of a message.
    pub fn of(kind: FailureKind, message: impl Into<String>) -> MeasureFailure {
        let message = message.into();
        let message = if message.trim().is_empty() {
            kind.word().to_string()
        } else {
            message
        };
        MeasureFailure { kind, message }
    }

    /// The kind.
    pub fn kind(&self) -> FailureKind {
        self.kind
    }

    /// What happened.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for MeasureFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind.word(), self.message)
    }
}

/// What one command of a run did: the evidence the run really ran it, taken by the runner and not told by a
/// provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEvidence {
    /// Which step of the run (`baseline-build`, `baseline-test`, `provider`).
    pub step: String,
    /// The command, resolved, with its arguments.
    pub command: String,
    /// SHA-256 of the executable, when it was found.
    pub command_digest: Option<String>,
    /// The exit code; none for a signal or a program that did not end.
    pub exit_code: Option<i32>,
    /// How long it ran.
    pub duration_ms: u64,
    /// The end of what it wrote on standard error.
    pub stderr: String,
}

/// What a measurement stands on: the code that was measured, what the Pin of that code stood on, and the settings
/// (commands, layout, provider) it was made with. A measurement is *current* while the Project's Pin and the
/// settings still give this stamp; once any part moves, it measured something else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureStamp {
    code: Digest,
    basis: PinBasis,
    settings: String,
}

impl MeasureStamp {
    /// The stamp of measuring the code `code`, standing on `basis`, with the settings `settings` (a digest the
    /// runner makes of its configuration and its provider).
    pub fn new(
        code: Digest,
        basis: PinBasis,
        settings: impl Into<String>,
    ) -> Result<MeasureStamp, MeasureError> {
        let settings = settings.into();
        if settings.trim().is_empty() {
            return Err(MeasureError::BlankSettings);
        }
        Ok(MeasureStamp {
            code,
            basis,
            settings,
        })
    }

    /// The digest of the code measured.
    pub fn code(&self) -> Digest {
        self.code
    }

    /// What the code stood on.
    pub fn basis(&self) -> &PinBasis {
        &self.basis
    }

    /// The settings digest.
    pub fn settings(&self) -> &str {
        &self.settings
    }

    /// Whether this is the stamp of measuring `code` standing on `basis`; with `settings` given, also with them.
    /// Without them the settings are not compared (nothing could say what they are now).
    pub fn holds(&self, code: Digest, basis: &PinBasis, settings: Option<&str>) -> bool {
        self.code == code && self.basis == *basis && settings.is_none_or(|s| s == self.settings)
    }

    /// One digest of the whole stamp, for a person to compare two stamps by eye.
    pub fn fingerprint(&self) -> Digest {
        let mut text = format!("code={};interface={}", self.code, self.basis.interface());
        let mut list = |name: &str, entries: Vec<(String, u64)>| {
            let _ = write!(text, ";{name}=");
            for (id, revision) in entries {
                let _ = write!(text, "{id}@{revision},");
            }
        };
        list("harnesses", rows(self.basis.harnesses()));
        list("contracts", rows(self.basis.contracts()));
        list("used", rows(self.basis.used_interfaces()));
        let _ = write!(text, ";settings={}", self.settings);
        Digest::of(text.as_bytes())
    }
}

/// `(id, revision number)` of each entry, in the order of the ids.
fn rows<K: std::fmt::Display>(map: &BTreeMap<K, super::revision::Revision>) -> Vec<(String, u64)> {
    map.iter()
        .map(|(k, r)| (k.to_string(), r.number()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::{HarnessId, test_uuid};
    use crate::domain::vo::revision::Revision;

    #[test]
    fn a_key_and_an_operator_are_checked_when_made() {
        assert!(MutantKey::try_from("S::fits/rel/1/9f3a").is_ok());
        for bad in ["", "a b", "a\nb", &"x".repeat(201)] {
            assert!(matches!(
                MutantKey::try_from(bad),
                Err(MeasureError::BadKey(_))
            ));
        }
        assert!(Operator::try_from("rel").is_ok());
        for bad in ["", "Rel", "a b", &"x".repeat(33)] {
            assert!(matches!(
                Operator::try_from(bad),
                Err(MeasureError::BadOperator(_))
            ));
        }
        assert_eq!(Site::new(0, 1), Err(MeasureError::BadSite));
        assert_eq!(Site::new(1, 0), Err(MeasureError::BadSite));
    }

    #[test]
    fn a_mutant_changes_something_and_says_where() {
        let key = MutantKey::try_from("f/rel/1/0000").unwrap();
        let op = Operator::try_from("rel").unwrap();
        let site = Site::new(4, 13).unwrap();
        assert_eq!(
            Mutant::new(key.clone(), site, op.clone(), ">".into(), ">".into()),
            Err(MeasureError::NoChange)
        );
        let m = Mutant::new(key, site, op, ">".into(), ">=".into()).unwrap();
        assert_eq!(
            m.describe(),
            "f/rel/1/0000: line 4, column 13, rel `>` -> `>=`"
        );
    }

    #[test]
    fn the_words_of_the_kinds_outcomes_and_failures_round_trip() {
        for k in MeasureKind::ALL {
            assert_eq!(MeasureKind::from_word(k.word()), Some(k));
        }
        for o in MutantOutcome::ALL {
            assert_eq!(MutantOutcome::from_word(o.word()), Some(o));
        }
        for k in FailureKind::ALL {
            assert_eq!(FailureKind::from_word(k.word()), Some(k));
        }
        assert_eq!(MutantOutcome::from_word("lost"), None);
        assert_eq!(
            MeasureFailure::new(FailureKind::Io, "  "),
            Err(MeasureError::BlankFailure)
        );
    }

    fn basis(harness_revision: Revision) -> PinBasis {
        PinBasis::new(
            Revision::first(),
            BTreeMap::from([(HarnessId::from_uuid(test_uuid(4)), harness_revision)]),
            BTreeMap::new(),
            BTreeMap::new(),
        )
    }

    #[test]
    fn a_stamp_holds_for_what_it_measured_and_not_for_a_moved_part() {
        let code = Digest::of(b"code");
        let stamp = MeasureStamp::new(code, basis(Revision::first()), "settings-1").unwrap();
        assert!(stamp.holds(code, &basis(Revision::first()), Some("settings-1")));
        // settings are compared only when known
        assert!(stamp.holds(code, &basis(Revision::first()), None));
        assert!(!stamp.holds(code, &basis(Revision::first()), Some("settings-2")));
        assert!(!stamp.holds(Digest::of(b"other"), &basis(Revision::first()), None));
        assert!(!stamp.holds(code, &basis(Revision::first().next()), None));
        assert_eq!(
            MeasureStamp::new(code, basis(Revision::first()), " "),
            Err(MeasureError::BlankSettings)
        );
    }

    #[test]
    fn the_fingerprint_follows_every_part_of_the_stamp() {
        let code = Digest::of(b"code");
        let print = |c: Digest, r: Revision, s: &str| {
            MeasureStamp::new(c, basis(r), s).unwrap().fingerprint()
        };
        let base = print(code, Revision::first(), "s");
        assert_eq!(base, print(code, Revision::first(), "s"));
        assert_ne!(base, print(Digest::of(b"x"), Revision::first(), "s"));
        assert_ne!(base, print(code, Revision::first().next(), "s"));
        assert_ne!(base, print(code, Revision::first(), "t"));
    }
}
