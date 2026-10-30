//! The simulation as values: the cases a sim script is run for, what each run found, how a rerun compared with the
//! first run, and what a simulation stands on. A sim Harness is a script written blind from the Contracts; chidev
//! runs it case by case (`seed-<n>`, `regress-<name>`) as a black-box driver of the program and keeps what each run
//! said. Reading the script's report is the runner's work (it is text from a process); everything here is what
//! the rules read afterwards.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use thiserror::Error;

use super::clause::ClauseLabel;
use super::digest::Digest;
use super::id::{ClauseId, ContractId, InterfaceId};
use super::measure::{BuiltPart, SettingsDigest, SettingsNow, StampPart};
use super::name::AssetName;
use super::pin_basis::moved_keys;
use super::revision::Revision;

/// The name a sim Harness gives its one Case that stands for the seeded sweep.
pub const SWEEP: &str = "sweep";

/// The prefix of the name of a regress Case of a sim Harness.
pub const REGRESS_PREFIX: &str = "regress-";

/// Why a value of the simulation is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SimError {
    /// A regress name is not `[a-z0-9-]{1,64}` without a leading `-`.
    #[error("a regress name is 1 to 64 of a-z, 0-9 and `-`, not starting with `-`: `{0}`")]
    BadRegressName(String),
    /// A case is not `seed-<n>` or `regress-<name>`.
    #[error("a case is `seed-<n>` or `regress-<name>`: `{0}`")]
    BadCase(String),
    /// A text is not `<contract>:<label>`.
    #[error(
        "`{0}` is not a clause name (`<contract>:C<n>`, with the contract's name of a-z, 0-9, `-` and `_`)"
    )]
    BadClauseName(String),
    /// No Clause of any Contract the sweep cites has this contract's name.
    #[error("no contract `{contract}` among those the sweep cites ({})", .cited.join(", "))]
    UnknownContract {
        /// The name the script gave.
        contract: String,
        /// The names of the Contracts the sweep cites a Clause of, in order.
        cited: Vec<String>,
    },
    /// The Contract is one the sweep cites, but not this Clause of it.
    #[error("the sweep case does not cite clause {0}")]
    UncitedClause(ClauseName),
}

/// A Clause as a script names it: the Contract's name and the Clause's label, `<contract>:<label>` (`stock:C3`).
/// A label is unique only within its Contract, so the Contract is part of the name; the script's name is resolved
/// to the Clause's id when a run is recorded and the record keeps the id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClauseName {
    contract: AssetName,
    label: ClauseLabel,
}

impl ClauseName {
    /// The name of the Clause `label` of the Contract named `contract`. The Contract's name has no white space
    /// (a line of the report ends the name at the first white space) and no `:`, which is what the rule of an
    /// asset name gives; nothing else is checked here.
    pub fn new(contract: AssetName, label: ClauseLabel) -> ClauseName {
        ClauseName { contract, label }
    }

    /// The Contract's name.
    pub fn contract(&self) -> &AssetName {
        &self.contract
    }

    /// The Clause's label.
    pub fn label(&self) -> ClauseLabel {
        self.label
    }
}

impl TryFrom<&str> for ClauseName {
    type Error = SimError;

    /// Reads `<contract>:<label>`; the label is what follows the last `:`.
    fn try_from(text: &str) -> Result<ClauseName, SimError> {
        let bad = || SimError::BadClauseName(text.to_string());
        let (contract, label) = text.rsplit_once(':').ok_or_else(bad)?;
        let label = ClauseLabel::try_from(label).map_err(|_| bad())?;
        let contract = AssetName::try_from(contract).map_err(|_| bad())?;
        Ok(ClauseName::new(contract, label))
    }
}

impl fmt::Display for ClauseName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.contract, self.label)
    }
}

/// A Clause the sweep cites, by the name a script writes it as and the id the record keeps it as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitedClause {
    /// The name in the script's report.
    pub name: ClauseName,
    /// The Clause.
    pub id: ClauseId,
}

/// The Clause `name` names among `cited`, the Clauses the sweep cites. The script may name only those: a name
/// of a Contract the sweep cites nothing of, and a Clause of a Contract it cites that the sweep does not, are told
/// apart in the error.
pub fn resolve(cited: &[CitedClause], name: &ClauseName) -> Result<ClauseId, SimError> {
    if let Some(found) = cited.iter().find(|c| c.name == *name) {
        return Ok(found.id);
    }
    if cited.iter().any(|c| c.name.contract == name.contract) {
        return Err(SimError::UncitedClause(name.clone()));
    }
    let mut known: Vec<String> = cited.iter().map(|c| c.name.contract.to_string()).collect();
    known.sort();
    known.dedup();
    Err(SimError::UnknownContract {
        contract: name.contract.to_string(),
        cited: known,
    })
}

/// The name of a regress case: a fixed schedule the script keeps (a past finding, a situation hard to reach by
/// chance). It is part of the `regress-<name>` Case name of the Harness and of the case argument of the script,
/// so it is limited to what a command line and a file name take safely.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegressName(String);

impl RegressName {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for RegressName {
    type Error = SimError;

    fn try_from(text: &str) -> Result<RegressName, SimError> {
        let ok = !text.is_empty()
            && text.len() <= 64
            && !text.starts_with('-')
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok {
            Ok(RegressName(text.to_string()))
        } else {
            Err(SimError::BadRegressName(text.to_string()))
        }
    }
}

impl fmt::Display for RegressName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One case the script is run for: a seed, or a regress case. Its text is the argument the script takes and the
/// name the report and the record use: `seed-3`, `regress-lease-lost`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SimCase {
    /// The case the script makes from this seed.
    Seed(u32),
    /// The explicit case the script keeps under this name.
    Regress(RegressName),
}

impl fmt::Display for SimCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimCase::Seed(n) => write!(f, "seed-{n}"),
            SimCase::Regress(name) => write!(f, "{REGRESS_PREFIX}{name}"),
        }
    }
}

impl TryFrom<&str> for SimCase {
    type Error = SimError;

    fn try_from(text: &str) -> Result<SimCase, SimError> {
        let bad = || SimError::BadCase(text.to_string());
        if let Some(digits) = text.strip_prefix("seed-") {
            // The canonical text only (no sign, no leading zero), so that a case has one name.
            let canonical = !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (digits == "0" || !digits.starts_with('0'));
            return digits
                .parse::<u32>()
                .ok()
                .filter(|_| canonical)
                .map(SimCase::Seed)
                .ok_or_else(bad);
        }
        match text.strip_prefix(REGRESS_PREFIX) {
            Some(name) => RegressName::try_from(name)
                .map(SimCase::Regress)
                .map_err(|_| bad()),
            None => Err(bad()),
        }
    }
}

/// What a Case of a sim Harness stands for, read from its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseRole {
    /// The seeded sweep: the Case that cites every Clause the script checks.
    Sweep,
    /// A regress case of this name.
    Regress(RegressName),
}

/// The role the Case named `name` has in a sim Harness; none when the name is neither `sweep` nor a valid
/// `regress-<name>`.
pub fn role_of(name: &str) -> Option<CaseRole> {
    if name == SWEEP {
        return Some(CaseRole::Sweep);
    }
    name.strip_prefix(REGRESS_PREFIX)
        .and_then(|n| RegressName::try_from(n).ok())
        .map(CaseRole::Regress)
}

/// A promise the program broke in a run, as the script said it: after which step, of which Clause, and how. The
/// Clause is kept by its id, resolved from the name the script gave when the run was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseFinding {
    /// The step of the trace (from 1) after which the Clause did not hold.
    pub step: u32,
    /// The Clause that did not hold.
    pub clause: ClauseId,
    /// What did not hold, in the script's words.
    pub text: String,
}

/// A Clause the run could not check because the situation it needs did not come up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseUnreached {
    /// The Clause.
    pub clause: ClauseId,
    /// The situation that was needed, in the script's words.
    pub text: String,
}

/// How the run of the script for one case ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseExit {
    /// Exit 0 or 1 and a report that follows the protocol.
    Normal,
    /// Any other exit code (a signal is -1), or exit 1 that names no finding: the script itself failed.
    Script(i32),
    /// The script did not end within the time allowed for a case.
    Timeout,
    /// The script said (`CANNOT_START <text>`) it could not start the program, so it drove nothing and checked
    /// nothing: the case has no findings and no unreached Clause, and it is not a case in which the program kept
    /// its promises. The text is one line, as the script gave it.
    CannotStart(String),
}

impl CaseExit {
    /// The word the record keeps: `normal`, `timeout`, `script:<code>` or `cannot_start:<text>`.
    pub fn word(&self) -> String {
        match self {
            CaseExit::Normal => "normal".to_string(),
            CaseExit::Script(code) => format!("script:{code}"),
            CaseExit::Timeout => "timeout".to_string(),
            CaseExit::CannotStart(why) => format!("cannot_start:{why}"),
        }
    }

    /// The exit a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<CaseExit> {
        match word {
            "normal" => Some(CaseExit::Normal),
            "timeout" => Some(CaseExit::Timeout),
            _ => word
                .strip_prefix("script:")
                .and_then(|c| c.parse().ok())
                .map(CaseExit::Script)
                .or_else(|| {
                    word.strip_prefix("cannot_start:")
                        .filter(|why| !why.trim().is_empty() && !why.contains('\n'))
                        .map(|why| CaseExit::CannotStart(why.to_string()))
                }),
        }
    }

    /// Whether the script said it could not start the program.
    pub fn cannot_start(&self) -> bool {
        matches!(self, CaseExit::CannotStart(_))
    }
}

/// What one run of the script for one case gave: how it ended, what it reported, and the digests of the schedule
/// and the trace chidev took from the files it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseRun {
    /// The case.
    pub case: SimCase,
    /// How the run ended.
    pub exit: CaseExit,
    /// The promises broken.
    pub findings: Vec<CaseFinding>,
    /// The Clauses whose situation did not come up.
    pub unreached: Vec<CaseUnreached>,
    /// Why the script could not decide something, one line each: counted, never failed.
    pub undecided: Vec<String>,
    /// The digest of `schedule`, the choices the script made for this case; none when it wrote none.
    pub schedule: Option<Digest>,
    /// The digest of `trace`, what the program did under that schedule; none when it wrote none.
    pub trace: Option<Digest>,
}

/// How a case compared with itself when it was run a second time in a fresh copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reproduction {
    /// The same schedule and the same trace.
    Same,
    /// Two schedules for one case: the script draws something besides its seed (a fault of the script).
    ScheduleDiffers,
    /// One schedule, two traces (or two ways to end): the program draws its own order (a fault of the program).
    TraceDiffers,
}

impl Reproduction {
    /// Every outcome.
    pub const ALL: [Reproduction; 3] = [
        Reproduction::Same,
        Reproduction::ScheduleDiffers,
        Reproduction::TraceDiffers,
    ];

    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Reproduction::Same => "same",
            Reproduction::ScheduleDiffers => "schedule-differs",
            Reproduction::TraceDiffers => "trace-differs",
        }
    }

    /// The outcome a word names; `None` for any other.
    pub fn from_word(word: &str) -> Option<Reproduction> {
        Self::ALL.into_iter().find(|r| r.word() == word)
    }
}

/// A case that was run twice, and what the second run showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rerun {
    /// The case.
    pub case: SimCase,
    /// How its second run compared with its first.
    pub result: Reproduction,
}

/// Compares the second run of a case with the first. A different schedule is the script's fault whatever else
/// differs; with one schedule, a different trace or a different way to end is the program's.
pub fn reproduction(first: &CaseRun, second: &CaseRun) -> Reproduction {
    if first.schedule != second.schedule {
        Reproduction::ScheduleDiffers
    } else if first.trace != second.trace || first.exit != second.exit {
        Reproduction::TraceDiffers
    } else {
        Reproduction::Same
    }
}

/// The cases to run a second time: `seed-0` (the first case of the sweep) if it was run and started the program,
/// then up to `sample` of the cases that had findings, in the order they were run. A finding is what is worth
/// reproducing, so those are the cases a person will read.
pub fn rerun_cases(runs: &[CaseRun], sample: usize) -> Vec<SimCase> {
    let mut chosen: Vec<SimCase> = Vec::new();
    if runs
        .iter()
        .any(|r| r.case == SimCase::Seed(0) && !r.exit.cannot_start())
    {
        chosen.push(SimCase::Seed(0));
    }
    let mut with_findings = 0;
    for run in runs {
        if with_findings >= sample {
            break;
        }
        if !run.findings.is_empty() && !chosen.contains(&run.case) {
            chosen.push(run.case.clone());
            with_findings += 1;
        }
    }
    chosen
}

/// Every Clause with a finding: how many cases found it, and the first one (in the order the cases ran).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundClause {
    /// The Clause.
    pub clause: ClauseId,
    /// How many cases broke it.
    pub cases: usize,
    /// The first case that did.
    pub first_case: SimCase,
    /// The step of that case.
    pub first_step: u32,
    /// What the script said of it.
    pub first_text: String,
}

/// What a simulation run gave: each case's run, and the second run of the sampled ones.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimOutcome {
    /// Each case that was run, in the order it was run: the seeds, then the regress cases.
    pub cases: Vec<CaseRun>,
    /// The cases run a second time and how they compared.
    pub reruns: Vec<Rerun>,
}

impl SimOutcome {
    /// The Clauses with a finding, in the order of their ids.
    pub fn found(&self) -> Vec<FoundClause> {
        let mut by_clause: BTreeMap<ClauseId, FoundClause> = BTreeMap::new();
        for run in &self.cases {
            let mut seen = BTreeSet::new();
            for f in &run.findings {
                let new_in_case = seen.insert(f.clause);
                let entry = by_clause.entry(f.clause).or_insert_with(|| FoundClause {
                    clause: f.clause,
                    cases: 0,
                    first_case: run.case.clone(),
                    first_step: f.step,
                    first_text: f.text.clone(),
                });
                if new_in_case {
                    entry.cases += 1;
                }
            }
        }
        by_clause.into_values().collect()
    }

    /// The Clauses no seed reached: every seed that ended normally said so, and at least one did. A regress case
    /// aims at one situation and does not count. The text is the first seed's.
    pub fn unreached(&self) -> Vec<CaseUnreached> {
        let seeds: Vec<&CaseRun> = self
            .cases
            .iter()
            .filter(|r| matches!(r.case, SimCase::Seed(_)) && r.exit == CaseExit::Normal)
            .collect();
        let Some((first, rest)) = seeds.split_first() else {
            return Vec::new();
        };
        first
            .unreached
            .iter()
            .filter(|u| {
                rest.iter()
                    .all(|r| r.unreached.iter().any(|x| x.clause == u.clause))
            })
            .cloned()
            .collect()
    }

    /// The reruns that did not reproduce.
    pub fn differing(&self) -> impl Iterator<Item = &Rerun> {
        self.reruns
            .iter()
            .filter(|r| r.result != Reproduction::Same)
    }

    /// The runs that did not end normally.
    pub fn abnormal(&self) -> impl Iterator<Item = &CaseRun> {
        self.cases.iter().filter(|r| r.exit != CaseExit::Normal)
    }

    /// The runs in which the program could not be started.
    pub fn cannot_start(&self) -> impl Iterator<Item = &CaseRun> {
        self.cases.iter().filter(|r| r.exit.cannot_start())
    }

    /// Whether the program could not be started in any case that was run (and some case was run): nothing was
    /// checked, and the cases say nothing of the Contracts.
    pub fn none_started(&self) -> bool {
        !self.cases.is_empty() && self.cases.iter().all(|r| r.exit.cannot_start())
    }
}

/// What a simulation stands on, as the model gives it: the sim Harness, the Contracts its Cases cite a Clause of
/// (any number, of one Context or of those it uses), and the Interfaces it builds, each with the code of its Pin
/// (none when it has none). An Interface the Harness builds is in the basis though no Case cites it: the program
/// the script ran against was made of its pinned code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimBasis {
    /// The revision of the sim Harness.
    pub harness: Revision,
    /// The revision of each Contract with a Clause a Case cites.
    pub contracts: BTreeMap<ContractId, Revision>,
    /// The revision of each Interface the Harness builds, and the digest of the code pinned for it.
    pub interfaces: BTreeMap<InterfaceId, (Revision, Option<Digest>)>,
}

/// A simulation's tie to what it ran: the basis, and the settings (commands, limits, seeds, interpreter) it ran
/// under. A simulation is *current* while the model and the settings still give this stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimStamp {
    basis: SimBasis,
    settings: SettingsDigest,
}

impl SimStamp {
    /// The stamp of a simulation that stood on `basis` and ran under `settings`.
    pub fn new(basis: SimBasis, settings: SettingsDigest) -> SimStamp {
        SimStamp { basis, settings }
    }

    /// What it stood on.
    pub fn basis(&self) -> &SimBasis {
        &self.basis
    }

    /// The settings digest.
    pub fn settings(&self) -> &SettingsDigest {
        &self.settings
    }

    /// What of this stamp has moved, given the `basis` the model gives now and the `settings` that are now: the
    /// script of the Harness, then each Contract and then each Interface it builds, by id, then the settings. An
    /// Interface that moved in two ways says both. Empty when this is the stamp of a simulation that stood on
    /// `basis` under those settings. Settings that are not known (`Unconfigured`, `Unreadable`) are a part that
    /// moved, for nothing shows the run was made under them.
    pub fn moved(&self, basis: &SimBasis, settings: &SettingsNow) -> Vec<StampPart> {
        let was = &self.basis;
        let mut parts = Vec::new();
        if was.harness != basis.harness {
            parts.push(StampPart::Script);
        }
        parts.extend(moved_keys(&was.contracts, &basis.contracts).map(StampPart::Contract));
        for id in moved_keys(&was.interfaces, &basis.interfaces) {
            match (was.interfaces.get(&id), basis.interfaces.get(&id)) {
                (Some((was_revision, was_code)), Some((revision, code))) => {
                    if was_revision != revision {
                        parts.push(StampPart::Built(id, BuiltPart::Revision));
                    }
                    if was_code != code {
                        parts.push(StampPart::Built(id, BuiltPart::PinnedCode));
                    }
                }
                _ => parts.push(StampPart::Built(id, BuiltPart::Membership)),
            }
        }
        parts.extend(settings.moved_from(&self.settings));
        parts
    }

    /// One digest of the whole stamp, for a person to compare two stamps by eye.
    pub fn fingerprint(&self) -> Digest {
        let b = &self.basis;
        let mut text = format!("sim;harness={}", b.harness.number());
        let _ = write!(text, ";contracts=");
        for (id, revision) in &b.contracts {
            let _ = write!(text, "{id}@{},", revision.number());
        }
        let _ = write!(text, ";interfaces=");
        for (id, (revision, pin)) in &b.interfaces {
            let pin = pin.map_or_else(|| "-".to_string(), |d| d.to_string());
            let _ = write!(text, "{id}@{}:{pin},", revision.number());
        }
        let _ = write!(text, ";settings={}", self.settings);
        Digest::of(text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::test_uuid;
    use crate::domain::vo::measure::SettingsGap;

    fn label(n: u32) -> ClauseLabel {
        ClauseLabel::try_from(format!("C{n}").as_str()).unwrap()
    }

    fn clause(n: u32) -> ClauseId {
        ClauseId::from_uuid(test_uuid(u128::from(n)))
    }

    fn name(s: &str) -> RegressName {
        RegressName::try_from(s).unwrap()
    }

    fn run(case: SimCase) -> CaseRun {
        CaseRun {
            case,
            exit: CaseExit::Normal,
            findings: Vec::new(),
            unreached: Vec::new(),
            undecided: Vec::new(),
            schedule: Some(Digest::of(b"s")),
            trace: Some(Digest::of(b"t")),
        }
    }

    fn found(mut r: CaseRun, step: u32, c: u32, text: &str) -> CaseRun {
        r.findings.push(CaseFinding {
            step,
            clause: clause(c),
            text: text.to_string(),
        });
        r
    }

    #[test]
    fn a_case_is_its_argument_and_has_one_spelling() {
        assert_eq!(SimCase::Seed(3).to_string(), "seed-3");
        assert_eq!(
            SimCase::Regress(name("lease-lost")).to_string(),
            "regress-lease-lost"
        );
        assert_eq!(SimCase::try_from("seed-0"), Ok(SimCase::Seed(0)));
        assert_eq!(
            SimCase::try_from("regress-a1"),
            Ok(SimCase::Regress(name("a1")))
        );
        for bad in [
            "",
            "seed-",
            "seed-03",
            "seed-+3",
            "seed--1",
            "seed-4294967296",
            "regress-",
            "regress-A",
            "regress--a",
            "regress-a b",
            "3",
            "regress-../x",
        ] {
            assert!(SimCase::try_from(bad).is_err(), "{bad:?}");
        }
        assert!(RegressName::try_from(&*"a".repeat(65)).is_err());
    }

    #[test]
    fn the_name_of_a_case_of_the_harness_gives_its_role() {
        assert_eq!(role_of("sweep"), Some(CaseRole::Sweep));
        assert_eq!(role_of("regress-x"), Some(CaseRole::Regress(name("x"))));
        for other in ["Sweep", "regress-", "regress-X", "seed-1", "a_test", ""] {
            assert_eq!(role_of(other), None, "{other:?}");
        }
    }

    #[test]
    fn an_exit_round_trips_through_its_word() {
        for e in [
            CaseExit::Normal,
            CaseExit::Timeout,
            CaseExit::Script(2),
            CaseExit::Script(-1),
            CaseExit::CannotStart("bank.py: No such file or directory".to_string()),
        ] {
            assert_eq!(CaseExit::from_word(&e.word()), Some(e));
        }
        assert_eq!(CaseExit::from_word("script:x"), None);
        // a start that gave no reason is not a word
        assert_eq!(CaseExit::from_word("cannot_start:"), None);
        assert_eq!(CaseExit::from_word("cannot_start: "), None);
        assert_eq!(CaseExit::from_word("cannot_start"), None);
        for r in Reproduction::ALL {
            assert_eq!(Reproduction::from_word(r.word()), Some(r));
        }
    }

    #[test]
    fn a_second_run_that_differs_is_the_scripts_fault_for_a_schedule_and_the_programs_for_a_trace()
    {
        let first = run(SimCase::Seed(0));
        assert_eq!(reproduction(&first, &first.clone()), Reproduction::Same);
        let mut other_schedule = first.clone();
        other_schedule.schedule = Some(Digest::of(b"other"));
        other_schedule.trace = Some(Digest::of(b"also other"));
        assert_eq!(
            reproduction(&first, &other_schedule),
            Reproduction::ScheduleDiffers
        );
        let mut other_trace = first.clone();
        other_trace.trace = Some(Digest::of(b"other"));
        assert_eq!(
            reproduction(&first, &other_trace),
            Reproduction::TraceDiffers
        );
        let mut other_end = first.clone();
        other_end.exit = CaseExit::Timeout;
        assert_eq!(reproduction(&first, &other_end), Reproduction::TraceDiffers);
    }

    #[test]
    fn seed_zero_and_the_first_cases_with_findings_are_run_again() {
        let runs = vec![
            run(SimCase::Seed(0)),
            found(run(SimCase::Seed(1)), 2, 1, "x"),
            run(SimCase::Seed(2)),
            found(run(SimCase::Seed(3)), 2, 1, "x"),
            found(run(SimCase::Seed(4)), 2, 2, "y"),
            found(run(SimCase::Regress(name("r"))), 1, 2, "y"),
        ];
        let seeds = |n: usize| -> Vec<String> {
            rerun_cases(&runs, n)
                .iter()
                .map(ToString::to_string)
                .collect()
        };
        assert_eq!(seeds(0), ["seed-0"]);
        assert_eq!(seeds(2), ["seed-0", "seed-1", "seed-3"]);
        assert_eq!(
            seeds(9),
            ["seed-0", "seed-1", "seed-3", "seed-4", "regress-r"]
        );
        // seed-0 with a finding counts once, and first
        let mut with = runs.clone();
        with[0] = found(run(SimCase::Seed(0)), 1, 1, "z");
        assert_eq!(
            rerun_cases(&with, 1)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["seed-0", "seed-1"]
        );
        // a sweep that did not run seed-0 has no seed-0 to repeat
        assert_eq!(rerun_cases(&runs[1..], 0), []);
    }

    #[test]
    fn the_clauses_found_are_counted_by_cases_and_the_first_is_kept() {
        let outcome = SimOutcome {
            cases: vec![
                run(SimCase::Seed(0)),
                found(found(run(SimCase::Seed(1)), 7, 4, "first"), 9, 4, "again"),
                found(run(SimCase::Seed(2)), 3, 4, "later"),
                found(run(SimCase::Seed(3)), 5, 2, "other"),
            ],
            reruns: Vec::new(),
        };
        let found = outcome.found();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].clause, clause(2));
        assert_eq!(found[1].clause, clause(4));
        assert_eq!(found[1].cases, 2);
        assert_eq!(
            (
                found[1].first_case.to_string(),
                found[1].first_step,
                found[1].first_text.as_str()
            ),
            ("seed-1".to_string(), 7, "first")
        );
    }

    #[test]
    fn a_clause_is_unreached_only_when_every_normal_seed_missed_it() {
        let miss = |mut r: CaseRun, c: u32| {
            r.unreached.push(CaseUnreached {
                clause: clause(c),
                text: format!("needs {c}"),
            });
            r
        };
        let outcome = SimOutcome {
            cases: vec![
                miss(miss(run(SimCase::Seed(0)), 5), 6),
                miss(run(SimCase::Seed(1)), 5),
                // a regress case and a failed seed do not count
                run(SimCase::Regress(name("r"))),
                CaseRun {
                    exit: CaseExit::Timeout,
                    ..run(SimCase::Seed(2))
                },
            ],
            reruns: Vec::new(),
        };
        let unreached = outcome.unreached();
        assert_eq!(unreached.len(), 1);
        assert_eq!(unreached[0].clause, clause(5));
        assert_eq!(unreached[0].text, "needs 5");
        assert!(SimOutcome::default().unreached().is_empty());
    }

    fn unstarted(case: SimCase) -> CaseRun {
        CaseRun {
            exit: CaseExit::CannotStart("no such program".to_string()),
            schedule: None,
            trace: None,
            ..run(case)
        }
    }

    #[test]
    fn a_case_that_could_not_start_the_program_is_counted_apart_and_shows_nothing_of_the_clauses() {
        let outcome = SimOutcome {
            cases: vec![
                unstarted(SimCase::Seed(0)),
                found(run(SimCase::Seed(1)), 2, 4, "broken"),
                unstarted(SimCase::Seed(2)),
            ],
            reruns: Vec::new(),
        };
        assert_eq!(outcome.cannot_start().count(), 2);
        assert!(!outcome.none_started());
        // only the case that drove the program found anything
        assert_eq!(outcome.found().len(), 1);
        // a seed that did not start says nothing of the Clauses it did not reach
        assert!(outcome.unreached().is_empty());
        // the case that could not start is not repeated, and seed-1 is the one with a finding
        assert_eq!(
            rerun_cases(&outcome.cases, 3)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["seed-1"]
        );

        let none = SimOutcome {
            cases: vec![
                unstarted(SimCase::Seed(0)),
                unstarted(SimCase::Regress(name("r"))),
            ],
            reruns: Vec::new(),
        };
        assert!(none.none_started());
        assert!(none.found().is_empty() && none.unreached().is_empty());
        // no case run is not a program that never started
        assert!(!SimOutcome::default().none_started());
    }

    #[test]
    fn two_clauses_of_one_label_in_two_contracts_are_two_findings() {
        // the same label C1 of two Contracts is two Clauses (ids 1 and 2), and a finding stays on the one it was
        // resolved to
        let outcome = SimOutcome {
            cases: vec![
                found(run(SimCase::Seed(0)), 1, 1, "stock"),
                found(
                    found(run(SimCase::Seed(1)), 2, 2, "cli"),
                    3,
                    1,
                    "stock again",
                ),
            ],
            reruns: Vec::new(),
        };
        let found = outcome.found();
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].clause, found[0].cases), (clause(1), 2));
        assert_eq!((found[1].clause, found[1].cases), (clause(2), 1));
    }

    fn cited() -> Vec<CitedClause> {
        let c = |contract: &str, l: u32, id: u32| CitedClause {
            name: ClauseName::new(AssetName::try_from(contract).unwrap(), label(l)),
            id: clause(id),
        };
        vec![c("stock", 1, 10), c("stock", 3, 11), c("cli", 1, 20)]
    }

    #[test]
    fn a_clause_is_named_by_its_contract_and_label() {
        let name = ClauseName::try_from("stock:C3").unwrap();
        assert_eq!(
            (name.contract().as_str(), name.label()),
            ("stock", label(3))
        );
        assert_eq!(name.to_string(), "stock:C3");
        // a contract name has no colon, so the label is what follows the only one
        for bad in [
            "",
            "C1",
            ":C1",
            "stock:",
            "stock:c1",
            "stock:C01",
            "st ock:C1",
            "stock :C1",
            "stock:C1 ",
            "a:b:C1",
            "Stock:C1",
        ] {
            assert!(ClauseName::try_from(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_name_resolves_among_the_clauses_the_sweep_cites_and_nowhere_else() {
        let cited = cited();
        let at = |s: &str| resolve(&cited, &ClauseName::try_from(s).unwrap());
        // C1 is in both Contracts and each name is its own Clause
        assert_eq!(at("stock:C1"), Ok(clause(10)));
        assert_eq!(at("cli:C1"), Ok(clause(20)));
        assert_eq!(at("stock:C3"), Ok(clause(11)));
        // a Contract the sweep cites a Clause of, but not this Clause
        let uncited = at("cli:C3").unwrap_err();
        assert_eq!(
            uncited,
            SimError::UncitedClause(ClauseName::try_from("cli:C3").unwrap())
        );
        assert!(uncited.to_string().contains("does not cite clause cli:C3"));
        // a Contract the sweep cites nothing of (or none at all)
        let unknown = at("fuel:C1").unwrap_err();
        assert_eq!(
            unknown,
            SimError::UnknownContract {
                contract: "fuel".into(),
                cited: vec!["cli".into(), "stock".into()]
            }
        );
        assert!(unknown.to_string().contains("no contract `fuel`"));
    }

    fn basis(harness: Revision, pin: Option<&[u8]>) -> SimBasis {
        SimBasis {
            harness,
            contracts: BTreeMap::from([(ContractId::from_uuid(test_uuid(1)), Revision::first())]),
            interfaces: BTreeMap::from([(
                InterfaceId::from_uuid(test_uuid(2)),
                (Revision::first(), pin.map(Digest::of)),
            )]),
        }
    }

    fn settings(word: &str) -> SettingsDigest {
        SettingsDigest::new(Digest::of(word.as_bytes()))
    }

    fn known(word: &str) -> SettingsNow {
        SettingsNow::Known(settings(word))
    }

    #[test]
    fn a_stamp_says_which_part_moved_and_nothing_when_none_did() {
        let first = Revision::first();
        let stamp = SimStamp::new(basis(first, Some(b"code")), settings("s1"));
        let now = known("s1");
        let contract = ContractId::from_uuid(test_uuid(1));
        let built = InterfaceId::from_uuid(test_uuid(2));
        let moved = |basis: SimBasis, now: &SettingsNow| stamp.moved(&basis, now);
        assert_eq!(moved(basis(first, Some(b"code")), &now), []);
        // the script of the Harness
        assert_eq!(
            moved(basis(first.next(), Some(b"code")), &now),
            [StampPart::Script]
        );
        // a Contract it cites a Clause of: changed, no longer cited, or cited anew
        let mut changed = basis(first, Some(b"code"));
        changed.contracts.insert(contract, first.next());
        assert_eq!(moved(changed, &now), [StampPart::Contract(contract)]);
        let mut dropped = basis(first, Some(b"code"));
        dropped.contracts.remove(&contract);
        assert_eq!(moved(dropped, &now), [StampPart::Contract(contract)]);
        let other = ContractId::from_uuid(test_uuid(7));
        let mut added = basis(first, Some(b"code"));
        added.contracts.insert(other, first);
        assert_eq!(moved(added, &now), [StampPart::Contract(other)]);
        // an Interface it builds: its revision
        let mut revised = basis(first, Some(b"code"));
        revised
            .interfaces
            .insert(built, (first.next(), Some(Digest::of(b"code"))));
        assert_eq!(
            moved(revised, &now),
            [StampPart::Built(built, BuiltPart::Revision)]
        );
        // the code pinned for it: other code, lost, or got
        for pinned in [Some(&b"other"[..]), None] {
            assert_eq!(
                moved(basis(first, pinned), &now),
                [StampPart::Built(built, BuiltPart::PinnedCode)]
            );
        }
        let unpinned = SimStamp::new(basis(first, None), settings("s1"));
        assert_eq!(
            unpinned.moved(&basis(first, Some(b"code")), &now),
            [StampPart::Built(built, BuiltPart::PinnedCode)]
        );
        // an Interface it builds that is no longer built, or is built anew
        let mut left = basis(first, Some(b"code"));
        left.interfaces.remove(&built);
        assert_eq!(
            moved(left, &now),
            [StampPart::Built(built, BuiltPart::Membership)]
        );
        let entered = InterfaceId::from_uuid(test_uuid(8));
        let mut joined = basis(first, Some(b"code"));
        joined.interfaces.insert(entered, (first, None));
        assert_eq!(
            moved(joined, &now),
            [StampPart::Built(entered, BuiltPart::Membership)]
        );
        // both the revision and the pinned code of one Interface
        let mut both = basis(first, Some(b"other"));
        both.interfaces
            .insert(built, (first.next(), Some(Digest::of(b"other"))));
        assert_eq!(
            moved(both, &now),
            [
                StampPart::Built(built, BuiltPart::Revision),
                StampPart::Built(built, BuiltPart::PinnedCode),
            ]
        );
        // the settings: another digest, no `[measure]`, or ones the runner cannot read
        assert_eq!(
            moved(basis(first, Some(b"code")), &known("s2")),
            [StampPart::Settings(SettingsGap::Differ)]
        );
        assert_eq!(
            moved(basis(first, Some(b"code")), &SettingsNow::Unconfigured),
            [StampPart::Settings(SettingsGap::Unconfigured)]
        );
        assert_eq!(
            moved(
                basis(first, Some(b"code")),
                &SettingsNow::Unreadable("no layout".to_string())
            ),
            [StampPart::Settings(SettingsGap::Unreadable(
                "no layout".to_string()
            ))]
        );
        // several at once come in the order of the stamp: the script, the Contracts, the Interfaces, the settings
        let mut many = basis(first.next(), Some(b"other"));
        many.contracts.insert(contract, first.next());
        assert_eq!(
            moved(many, &known("s2")),
            [
                StampPart::Script,
                StampPart::Contract(contract),
                StampPart::Built(built, BuiltPart::PinnedCode),
                StampPart::Settings(SettingsGap::Differ),
            ]
        );
    }

    #[test]
    fn the_fingerprint_follows_every_part_of_the_stamp() {
        let print = |r: Revision, p: Option<&[u8]>, s: &str| {
            SimStamp::new(basis(r, p), settings(s)).fingerprint()
        };
        let base = print(Revision::first(), Some(b"c"), "s");
        assert_eq!(base, print(Revision::first(), Some(b"c"), "s"));
        assert_ne!(base, print(Revision::first().next(), Some(b"c"), "s"));
        assert_ne!(base, print(Revision::first(), Some(b"d"), "s"));
        assert_ne!(base, print(Revision::first(), Some(b"c"), "t"));
    }
}
