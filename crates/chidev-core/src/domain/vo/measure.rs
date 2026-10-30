//! What a measurement is made of: the kind of measure, a mutant and what became of it, how a run failed, and the
//! stamp that ties a run to the thing it measured. A measurement is a fact about code that was run; the rules that
//! judge it read these values and never run anything.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use thiserror::Error;

use super::digest::Digest;
use super::examples_run::ExamplesStamp;
use super::id::{ContractId, HarnessId, InterfaceId};
use super::pin_basis::{PinBasis, StaleReason};
use super::sim::SimStamp;

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
    /// A run evaluated more mutants than the provider gave.
    #[error("{mutants} mutants evaluated of the {planned} the provider gave")]
    MoreMutantsThanPlanned {
        /// The mutants evaluated.
        mutants: usize,
        /// The mutants the provider gave.
        planned: u32,
    },
    /// The provider gave more mutants than it found sites.
    #[error("{planned} mutants from {sites} sites")]
    MorePlannedThanSites {
        /// The mutants the provider gave.
        planned: u32,
        /// The sites it found.
        sites: u32,
    },
    /// A run that did not fail evaluated fewer mutants than the provider gave: it would pass for a whole one.
    #[error("{mutants} of the {planned} mutants evaluated, and no failure says why")]
    UnfinishedWithoutFailure {
        /// The mutants evaluated.
        mutants: usize,
        /// The mutants the provider gave.
        planned: u32,
    },
    /// A failure has no message.
    #[error("a failure says what failed")]
    BlankFailure,
    /// Two records of one run are of one Case.
    #[error("the example of the case {0} is recorded twice")]
    DuplicateExample(String),
}

/// What a measure measures: mutation testing, simulation and the examples of a Harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeasureKind {
    /// Single-site mutants of the code of an Interface, run against the Harnesses that check it.
    Mutate,
    /// A sim Harness (a script written blind from the Contracts), run as a driver of the program built from the
    /// code of the Pins it cites.
    Simulate,
    /// A Harness of examples, each Case run as one example against the program built from the code of the Pins of
    /// the Interfaces it builds.
    Examples,
}

impl MeasureKind {
    /// Every kind.
    pub const ALL: [MeasureKind; 3] = [
        MeasureKind::Mutate,
        MeasureKind::Simulate,
        MeasureKind::Examples,
    ];

    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            MeasureKind::Mutate => "mutate",
            MeasureKind::Simulate => "simulate",
            MeasureKind::Examples => "examples",
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
    /// The program a simulation drives does not build, so the script was not run.
    NoBuild,
    /// The program a simulation drives was built, but the script could not start it in any case: nothing was
    /// checked, and the cases say nothing of the Contracts.
    NoStart,
    /// A program the run started from the copy of the project is not the one the settings were digested with: its
    /// digest in the copy differs from the one taken at the project's root, so the run would have been made under
    /// other settings than the stamp it is kept with names.
    SettingsMoved,
}

impl FailureKind {
    /// Every kind.
    pub const ALL: [FailureKind; 9] = [
        FailureKind::Baseline,
        FailureKind::Provider,
        FailureKind::Protocol,
        FailureKind::Timeout,
        FailureKind::Budget,
        FailureKind::Io,
        FailureKind::NoBuild,
        FailureKind::NoStart,
        FailureKind::SettingsMoved,
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
            FailureKind::NoBuild => "no_build",
            FailureKind::NoStart => "no_start",
            FailureKind::SettingsMoved => "settings_moved",
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

/// The digest of what a run read of the settings (the commands it started and the programs they start, the limits it
/// kept to, where the code it ran lives), as the runner made it. It is opaque: the runner makes one from a
/// [`Digest`], and all anyone else does with it is compare two and show one. That it is a digest of something, and
/// not a blank or a made-up word, is therefore the type's guarantee and not a check made where it is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsDigest(Digest);

impl SettingsDigest {
    /// The settings whose digest is `digest`.
    pub fn new(digest: Digest) -> SettingsDigest {
        SettingsDigest(digest)
    }
}

/// The digest as [`Digest`] writes it (64 lower-case hex digits), which is also how a store keeps it.
impl std::fmt::Display for SettingsDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// What the settings are now for the subject of a measurement, as far as the runner can say. There is no state in
/// which they are "not compared": a run whose settings cannot be compared with what it was made with cannot be
/// shown to describe the settings, so it is stale, and the two ways of not being comparable are told apart so that
/// a person is told which to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsNow {
    /// The project has no `[measure]` table: there is no runner to say anything.
    Unconfigured,
    /// The settings read as this digest.
    Known(SettingsDigest),
    /// The project has a runner, and it cannot read the settings for this subject (no layout for it, no command to
    /// find, a program that is gone). The text says why, in the words of the runner.
    Unreadable(String),
}

impl SettingsNow {
    /// The settings of `subject` among `settings`. A subject that is not in them has none that can be read: a
    /// measurement of it cannot be compared on its settings, so it is stale, and not current by default. This is
    /// the one place that says so.
    pub fn of<K: Ord>(settings: &BTreeMap<K, SettingsNow>, subject: &K) -> SettingsNow {
        settings
            .get(subject)
            .cloned()
            .unwrap_or_else(|| SettingsNow::Unreadable("its settings were not read".to_string()))
    }

    /// Why a run cannot be compared on its settings; none when they are known.
    pub fn why_not_compared(&self) -> Option<String> {
        match self {
            SettingsNow::Unconfigured => SettingsGap::Unconfigured.why(),
            SettingsNow::Known(_) => None,
            SettingsNow::Unreadable(why) => SettingsGap::Unreadable(why.clone()).why(),
        }
    }

    /// The part of a stamp the settings now move, given the digest `was` the run was made with; none when they
    /// are known and are the same. Settings that are not known are a part that moved, for nothing shows the run
    /// was made under them. The one place that says so, for a mutation run and for a simulation.
    pub(super) fn moved_from(&self, was: &SettingsDigest) -> Option<StampPart> {
        let gap = match self {
            SettingsNow::Known(now) if now == was => return None,
            SettingsNow::Known(_) => SettingsGap::Differ,
            SettingsNow::Unconfigured => SettingsGap::Unconfigured,
            SettingsNow::Unreadable(why) => SettingsGap::Unreadable(why.clone()),
        };
        Some(StampPart::Settings(gap))
    }
}

/// How the settings now stand against the ones a run was made with, when they are not the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsGap {
    /// They read as another digest.
    Differ,
    /// The project has no `[measure]` table.
    Unconfigured,
    /// The runner cannot read them for the subject, and says why.
    Unreadable(String),
}

impl SettingsGap {
    /// Why a run cannot be compared on its settings, in a sentence; none when they could be compared and read as
    /// another digest. The one place that words it.
    pub fn why(&self) -> Option<String> {
        match self {
            SettingsGap::Differ => None,
            SettingsGap::Unconfigured => Some("measure is not configured".to_string()),
            SettingsGap::Unreadable(why) => Some(format!("the settings cannot be read: {why}")),
        }
    }
}

/// How an Interface that a sim Harness builds moved since a simulation was run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltPart {
    /// The Interface changed revision.
    Revision,
    /// The code pinned for it changed, or it got a Pin or lost it.
    PinnedCode,
    /// The Harness began to build the Interface, or no longer builds it.
    Membership,
}

/// An asset that has a name a person gives it, which a moved part of a stamp is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetRef {
    /// A Contract.
    Contract(ContractId),
    /// An Interface.
    Interface(InterfaceId),
    /// A Harness.
    Harness(HarnessId),
}

impl AssetRef {
    /// The id of the asset as a uuid, whichever it is.
    pub fn as_uuid(self) -> uuid::Uuid {
        match self {
            AssetRef::Contract(i) => i.as_uuid(),
            AssetRef::Interface(i) => i.as_uuid(),
            AssetRef::Harness(i) => i.as_uuid(),
        }
    }
}

/// One part of what a measurement stands on that has moved since the run: the one kind of part for a mutation run
/// and for a simulation, so that a status and a finding say what moved in the same words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampPart {
    /// A mutation run: a part of the basis of the Pin it stood on moved.
    Pin(StaleReason),
    /// A mutation run: the pinned code of the measured Interface.
    Code,
    /// A simulation: the revision of the sim Harness itself (its script and its Cases). The basis of a simulation
    /// has no id for it, for the Harness is the subject.
    Script,
    /// A run of examples: the revision of the Harness itself (its examples). Like [`StampPart::Script`] it has no
    /// id, for the Harness is the subject.
    Examples,
    /// A simulation: a Contract that its Cases cite a Clause of changed, is gone, or is newly cited.
    Contract(ContractId),
    /// A simulation or a run of examples: an Interface that it builds moved in the way the part says.
    Built(InterfaceId, BuiltPart),
    /// The settings the run was made under, as they are now.
    Settings(SettingsGap),
    /// The subject is no longer in the model, so there is no basis to compare with. An Interface that is in the
    /// model with no Pin is not this: nothing is compared for it, and it is not stale for want of the subject.
    Gone,
}

impl StampPart {
    /// The word for it, one of `code`, `interface`, `harness`, `contract`, `used-interface`, `script`, `examples`,
    /// `built-revision`, `built-code`, `builds`, `settings`, `settings-unconfigured`, `settings-unreadable` and
    /// `gone`: the one place that names the parts in words, for the status. A Contract a Pin stood on and a Contract
    /// a simulation cites both say `contract`: each is a cited Contract that moved. The settings say apart whether
    /// they read as another digest (`settings`), or could not be compared because there is no `[measure]` table
    /// (`settings-unconfigured`) or the runner cannot read them (`settings-unreadable`, whose
    /// [`reason`](StampPart::reason) is the runner's).
    pub fn word(&self) -> &'static str {
        match self {
            StampPart::Code => "code",
            StampPart::Pin(StaleReason::Interface) => "interface",
            StampPart::Pin(StaleReason::Harness(_)) => "harness",
            StampPart::Pin(StaleReason::Contract(_)) | StampPart::Contract(_) => "contract",
            StampPart::Pin(StaleReason::UsedInterface(_)) => "used-interface",
            StampPart::Script => "script",
            StampPart::Examples => "examples",
            StampPart::Built(_, BuiltPart::Revision) => "built-revision",
            StampPart::Built(_, BuiltPart::PinnedCode) => "built-code",
            StampPart::Built(_, BuiltPart::Membership) => "builds",
            StampPart::Settings(SettingsGap::Differ) => "settings",
            StampPart::Settings(SettingsGap::Unconfigured) => "settings-unconfigured",
            StampPart::Settings(SettingsGap::Unreadable(_)) => "settings-unreadable",
            StampPart::Gone => "gone",
        }
    }

    /// Why settings could not be compared, in a sentence ([`SettingsGap::why`]); none for any other part, and for
    /// settings that read as another digest.
    pub fn why(&self) -> Option<String> {
        match self {
            StampPart::Settings(gap) => gap.why(),
            _ => None,
        }
    }

    /// The reason the runner gave for settings it cannot read, in its own words; none for any other part.
    pub fn reason(&self) -> Option<&str> {
        match self {
            StampPart::Settings(SettingsGap::Unreadable(why)) => Some(why),
            StampPart::Pin(_)
            | StampPart::Code
            | StampPart::Script
            | StampPart::Examples
            | StampPart::Contract(_)
            | StampPart::Built(..)
            | StampPart::Settings(SettingsGap::Differ | SettingsGap::Unconfigured)
            | StampPart::Gone => None,
        }
    }

    /// The named asset the part is about; none for a part that is not about one. The one source of which parts
    /// name an asset.
    pub fn asset(&self) -> Option<AssetRef> {
        match self {
            StampPart::Pin(StaleReason::Harness(id)) => Some(AssetRef::Harness(*id)),
            StampPart::Pin(StaleReason::Contract(id)) | StampPart::Contract(id) => {
                Some(AssetRef::Contract(*id))
            }
            StampPart::Pin(StaleReason::UsedInterface(id)) | StampPart::Built(id, _) => {
                Some(AssetRef::Interface(*id))
            }
            StampPart::Pin(StaleReason::Interface)
            | StampPart::Code
            | StampPart::Script
            | StampPart::Examples
            | StampPart::Settings(_)
            | StampPart::Gone => None,
        }
    }
}

/// What a measurement stands on: the code that was measured, what the Pin of that code stood on, and the settings
/// (commands, layout, provider) it was made with. A measurement is *current* while the Project's Pin and the
/// settings still give this stamp; once any part moves, it measured something else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasureStamp {
    code: Digest,
    basis: PinBasis,
    settings: SettingsDigest,
}

impl MeasureStamp {
    /// The stamp of measuring the code `code`, standing on `basis`, with the settings `settings` (a digest the
    /// runner makes of what the run reads of its configuration).
    pub fn new(code: Digest, basis: PinBasis, settings: SettingsDigest) -> MeasureStamp {
        MeasureStamp {
            code,
            basis,
            settings,
        }
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
    pub fn settings(&self) -> &SettingsDigest {
        &self.settings
    }

    /// What of this stamp has moved, given the `code` pinned now, the `basis` the Pin stands on now and the
    /// `settings` that are now: the code, then each part of the basis as [`PinBasis::moved`] gives it, then the
    /// settings. Empty when this is the stamp of measuring `code` standing on `basis` under those settings. Settings
    /// that are not known (`Unconfigured`, `Unreadable`) are a part that moved, for nothing shows the run was made
    /// under them.
    pub fn moved(&self, code: Digest, basis: &PinBasis, settings: &SettingsNow) -> Vec<StampPart> {
        // every field is named, so a field added to the stamp is not left out of the comparison
        let MeasureStamp {
            code: was_code,
            basis: was_basis,
            settings: was_settings,
        } = self;
        let mut parts = Vec::new();
        if *was_code != code {
            parts.push(StampPart::Code);
        }
        parts.extend(was_basis.moved(basis).into_iter().map(StampPart::Pin));
        parts.extend(settings.moved_from(was_settings));
        parts
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

/// What a measurement is of: the Interface whose code was mutated, or the sim Harness that was run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeasureSubject {
    /// An Interface (a mutation run).
    Interface(InterfaceId),
    /// A Harness (a simulation).
    Harness(HarnessId),
}

impl MeasureSubject {
    /// The word for the kind of thing it is.
    pub fn word(self) -> &'static str {
        match self {
            MeasureSubject::Interface(_) => "interface",
            MeasureSubject::Harness(_) => "harness",
        }
    }

    /// The id as a uuid, whichever it is.
    pub fn as_uuid(self) -> uuid::Uuid {
        match self {
            MeasureSubject::Interface(i) => i.as_uuid(),
            MeasureSubject::Harness(h) => h.as_uuid(),
        }
    }
}

/// What a measurement stands on, seen without taking it from the measurement: the stamp its body holds, which is
/// the code of a Pin for a mutation run, the sim Harness with what it cites for a simulation, and the Harness of
/// examples with the code pinned for what it builds for a run of examples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampRef<'a> {
    /// Of a mutation run.
    Code(&'a MeasureStamp),
    /// Of a simulation.
    Sim(&'a SimStamp),
    /// Of a run of examples.
    Examples(&'a ExamplesStamp),
}

impl<'a> StampRef<'a> {
    /// One digest of the whole stamp.
    pub fn fingerprint(&self) -> Digest {
        match self {
            StampRef::Code(s) => s.fingerprint(),
            StampRef::Sim(s) => s.fingerprint(),
            StampRef::Examples(s) => s.fingerprint(),
        }
    }

    /// The settings digest.
    pub fn settings(&self) -> &'a SettingsDigest {
        match self {
            StampRef::Code(s) => s.settings(),
            StampRef::Sim(s) => s.settings(),
            StampRef::Examples(s) => s.settings(),
        }
    }

    /// The stamp of a mutation run, if it is one.
    pub fn code(&self) -> Option<&'a MeasureStamp> {
        match self {
            StampRef::Code(s) => Some(s),
            StampRef::Sim(_) | StampRef::Examples(_) => None,
        }
    }

    /// The stamp of a simulation, if it is one.
    pub fn sim(&self) -> Option<&'a SimStamp> {
        match self {
            StampRef::Sim(s) => Some(s),
            StampRef::Code(_) | StampRef::Examples(_) => None,
        }
    }

    /// The stamp of a run of examples, if it is one.
    pub fn examples(&self) -> Option<&'a ExamplesStamp> {
        match self {
            StampRef::Examples(s) => Some(s),
            StampRef::Code(_) | StampRef::Sim(_) => None,
        }
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
    use crate::domain::vo::id::{ContractId, HarnessId, InterfaceId, test_uuid};
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

    /// Settings that read as the digest of `word`.
    fn settings(word: &str) -> SettingsDigest {
        SettingsDigest::new(Digest::of(word.as_bytes()))
    }

    fn known(word: &str) -> SettingsNow {
        SettingsNow::Known(settings(word))
    }

    /// A basis of the Interface at `interface`, Harness 4 at `harness`, Contract 3 at `contract` and the other
    /// Interface 2 at `used`.
    fn whole(
        interface: Revision,
        harness: Revision,
        contract: Revision,
        used: Revision,
    ) -> PinBasis {
        PinBasis::new(
            interface,
            BTreeMap::from([(HarnessId::from_uuid(test_uuid(4)), harness)]),
            BTreeMap::from([(ContractId::from_uuid(test_uuid(3)), contract)]),
            BTreeMap::from([(InterfaceId::from_uuid(test_uuid(2)), used)]),
        )
    }

    #[test]
    fn a_stamp_says_which_part_moved_and_nothing_when_none_did() {
        let first = Revision::first();
        let next = first.next();
        let code = Digest::of(b"code");
        let stamp = MeasureStamp::new(code, whole(first, first, first, first), settings("s1"));
        let moved =
            |code: Digest, basis: PinBasis, now: &SettingsNow| stamp.moved(code, &basis, now);
        let same = whole(first, first, first, first);
        assert_eq!(moved(code, same.clone(), &known("s1")), []);
        // each part alone moves only that part
        assert_eq!(
            moved(Digest::of(b"other"), same.clone(), &known("s1")),
            [StampPart::Code]
        );
        assert_eq!(
            moved(code, whole(next, first, first, first), &known("s1")),
            [StampPart::Pin(StaleReason::Interface)]
        );
        assert_eq!(
            moved(code, whole(first, next, first, first), &known("s1")),
            [StampPart::Pin(StaleReason::Harness(HarnessId::from_uuid(
                test_uuid(4)
            )))]
        );
        assert_eq!(
            moved(code, whole(first, first, next, first), &known("s1")),
            [StampPart::Pin(StaleReason::Contract(
                ContractId::from_uuid(test_uuid(3))
            ))]
        );
        assert_eq!(
            moved(code, whole(first, first, first, next), &known("s1")),
            [StampPart::Pin(StaleReason::UsedInterface(
                InterfaceId::from_uuid(test_uuid(2))
            ))]
        );
        assert_eq!(
            moved(code, same.clone(), &known("s2")),
            [StampPart::Settings(SettingsGap::Differ)]
        );
        // several at once come in the order of the stamp: the code, the basis, the settings
        assert_eq!(
            moved(
                Digest::of(b"other"),
                whole(next, next, first, first),
                &known("s2")
            ),
            [
                StampPart::Code,
                StampPart::Pin(StaleReason::Interface),
                StampPart::Pin(StaleReason::Harness(HarnessId::from_uuid(test_uuid(4)))),
                StampPart::Settings(SettingsGap::Differ),
            ]
        );
    }

    #[test]
    fn settings_that_cannot_be_compared_are_a_part_that_moved_and_say_why() {
        let code = Digest::of(b"code");
        let stamp = MeasureStamp::new(code, basis(Revision::first()), settings("s"));
        // everything else is as measured, and still the stamp has a part that moved
        let unreadable = SettingsNow::Unreadable("no layout".to_string());
        assert_eq!(
            stamp.moved(code, &basis(Revision::first()), &SettingsNow::Unconfigured),
            [StampPart::Settings(SettingsGap::Unconfigured)]
        );
        assert_eq!(
            stamp.moved(code, &basis(Revision::first()), &unreadable),
            [StampPart::Settings(SettingsGap::Unreadable(
                "no layout".to_string()
            ))]
        );
        assert_eq!(
            SettingsNow::Unconfigured.why_not_compared().as_deref(),
            Some("measure is not configured")
        );
        assert_eq!(
            unreadable.why_not_compared().as_deref(),
            Some("the settings cannot be read: no layout")
        );
        assert_eq!(known("s").why_not_compared(), None);
    }

    #[test]
    fn a_subject_with_no_settings_among_them_has_settings_that_were_not_read() {
        let among = BTreeMap::from([(1u8, known("s"))]);
        assert_eq!(SettingsNow::of(&among, &1), known("s"));
        let missing = SettingsNow::of(&among, &2);
        assert_eq!(
            missing.why_not_compared().as_deref(),
            Some("the settings cannot be read: its settings were not read")
        );
    }

    #[test]
    fn a_settings_digest_shows_as_the_digest_it_was_made_from() {
        let digest = Digest::of(b"s");
        assert_eq!(SettingsDigest::new(digest).to_string(), digest.to_string());
        assert_eq!(settings("s"), settings("s"));
        assert_ne!(settings("s"), settings("t"));
    }

    #[test]
    fn the_fingerprint_follows_every_part_of_the_stamp() {
        let code = Digest::of(b"code");
        let print = |c: Digest, r: Revision, s: &str| {
            MeasureStamp::new(c, basis(r), settings(s)).fingerprint()
        };
        let base = print(code, Revision::first(), "s");
        assert_eq!(base, print(code, Revision::first(), "s"));
        assert_ne!(base, print(Digest::of(b"x"), Revision::first(), "s"));
        assert_ne!(base, print(code, Revision::first().next(), "s"));
        assert_ne!(base, print(code, Revision::first(), "t"));
    }

    #[test]
    fn every_part_has_its_word_and_only_the_parts_about_an_asset_name_one() {
        let h = HarnessId::from_uuid(test_uuid(4));
        let c = ContractId::from_uuid(test_uuid(3));
        let i = InterfaceId::from_uuid(test_uuid(2));
        let unreadable = || SettingsGap::Unreadable("why".to_string());
        for (part, word, asset) in [
            (StampPart::Code, "code", None),
            (StampPart::Pin(StaleReason::Interface), "interface", None),
            (
                StampPart::Pin(StaleReason::Harness(h)),
                "harness",
                Some(AssetRef::Harness(h)),
            ),
            (
                StampPart::Pin(StaleReason::Contract(c)),
                "contract",
                Some(AssetRef::Contract(c)),
            ),
            (
                StampPart::Pin(StaleReason::UsedInterface(i)),
                "used-interface",
                Some(AssetRef::Interface(i)),
            ),
            (StampPart::Script, "script", None),
            (StampPart::Examples, "examples", None),
            (
                StampPart::Contract(c),
                "contract",
                Some(AssetRef::Contract(c)),
            ),
            (
                StampPart::Built(i, BuiltPart::Revision),
                "built-revision",
                Some(AssetRef::Interface(i)),
            ),
            (
                StampPart::Built(i, BuiltPart::PinnedCode),
                "built-code",
                Some(AssetRef::Interface(i)),
            ),
            (
                StampPart::Built(i, BuiltPart::Membership),
                "builds",
                Some(AssetRef::Interface(i)),
            ),
            (StampPart::Settings(SettingsGap::Differ), "settings", None),
            (
                StampPart::Settings(SettingsGap::Unconfigured),
                "settings-unconfigured",
                None,
            ),
            (
                StampPart::Settings(unreadable()),
                "settings-unreadable",
                None,
            ),
            (StampPart::Gone, "gone", None),
        ] {
            assert_eq!(part.word(), word, "{part:?}");
            assert_eq!(part.asset(), asset, "{part:?}");
            // only settings the runner cannot read have a reason
            let reason = (word == "settings-unreadable").then_some("why");
            assert_eq!(part.reason(), reason, "{part:?}");
        }
        assert_eq!(SettingsGap::Differ.why(), None);
        assert_eq!(
            SettingsGap::Unconfigured.why().as_deref(),
            Some("measure is not configured")
        );
        assert_eq!(
            unreadable().why().as_deref(),
            Some("the settings cannot be read: why")
        );
        assert_eq!(AssetRef::Harness(h).as_uuid(), h.as_uuid());
        assert_eq!(AssetRef::Contract(c).as_uuid(), c.as_uuid());
        assert_eq!(AssetRef::Interface(i).as_uuid(), i.as_uuid());
    }
}
