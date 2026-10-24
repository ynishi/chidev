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
use super::id::{ContractId, InterfaceId};
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
    /// A stamp has no settings.
    #[error("a stamp names the settings the run was made with")]
    BlankSettings,
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

/// A promise the program broke in a run, as the script said it: after which step, of which Clause, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseFinding {
    /// The step of the trace (from 1) after which the Clause did not hold.
    pub step: u32,
    /// The Clause that did not hold.
    pub label: ClauseLabel,
    /// What did not hold, in the script's words.
    pub text: String,
}

/// A Clause the run could not check because the situation it needs did not come up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseUnreached {
    /// The Clause.
    pub label: ClauseLabel,
    /// The situation that was needed, in the script's words.
    pub text: String,
}

/// How the run of the script for one case ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseExit {
    /// Exit 0 or 1 and a report that follows the protocol.
    Normal,
    /// Any other exit code (a signal is -1), or exit 1 that names no finding: the script itself failed.
    Script(i32),
    /// The script did not end within the time allowed for a case.
    Timeout,
}

impl CaseExit {
    /// The word the record keeps: `normal`, `timeout` or `script:<code>`.
    pub fn word(self) -> String {
        match self {
            CaseExit::Normal => "normal".to_string(),
            CaseExit::Script(code) => format!("script:{code}"),
            CaseExit::Timeout => "timeout".to_string(),
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
                .map(CaseExit::Script),
        }
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

/// The cases to run a second time: `seed-0` (the first case of the sweep) if it was run, then up to `sample` of
/// the cases that had findings, in the order they were run. A finding is what is worth reproducing, so those are
/// the cases a person will read.
pub fn rerun_cases(runs: &[CaseRun], sample: usize) -> Vec<SimCase> {
    let mut chosen: Vec<SimCase> = Vec::new();
    if runs.iter().any(|r| r.case == SimCase::Seed(0)) {
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
    pub label: ClauseLabel,
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
    /// The Clauses with a finding, in the order of their labels.
    pub fn found(&self) -> Vec<FoundClause> {
        let mut by_label: BTreeMap<ClauseLabel, FoundClause> = BTreeMap::new();
        for run in &self.cases {
            let mut seen = BTreeSet::new();
            for f in &run.findings {
                let new_in_case = seen.insert(f.label);
                let entry = by_label.entry(f.label).or_insert_with(|| FoundClause {
                    label: f.label,
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
        by_label.into_values().collect()
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
                    .all(|r| r.unreached.iter().any(|x| x.label == u.label))
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
}

/// What a simulation stands on, as the model gives it: the sim Harness, the Contracts its Cases cite, and the
/// Interfaces whose Items they cite, each with the code of its Pin (none when it has none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimBasis {
    /// The revision of the sim Harness.
    pub harness: Revision,
    /// The revision of each Contract with a Clause a Case cites.
    pub contracts: BTreeMap<ContractId, Revision>,
    /// The revision of each Interface with an Item a Case cites, and the digest of the code pinned for it.
    pub interfaces: BTreeMap<InterfaceId, (Revision, Option<Digest>)>,
}

/// A simulation's tie to what it ran: the basis, and the settings (commands, limits, seeds, interpreter) it ran
/// under. A simulation is *current* while the model and the settings still give this stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimStamp {
    basis: SimBasis,
    settings: String,
}

impl SimStamp {
    /// The stamp of a simulation that stood on `basis` and ran under `settings`.
    pub fn new(basis: SimBasis, settings: impl Into<String>) -> Result<SimStamp, SimError> {
        let settings = settings.into();
        if settings.trim().is_empty() {
            return Err(SimError::BlankSettings);
        }
        Ok(SimStamp { basis, settings })
    }

    /// What it stood on.
    pub fn basis(&self) -> &SimBasis {
        &self.basis
    }

    /// The settings digest.
    pub fn settings(&self) -> &str {
        &self.settings
    }

    /// Whether this is the stamp of a simulation that stood on `basis`; with `settings` given, also under them.
    /// Without them the settings are not compared (nothing could say what they are now).
    pub fn holds(&self, basis: &SimBasis, settings: Option<&str>) -> bool {
        self.basis == *basis && settings.is_none_or(|s| s == self.settings)
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

    fn label(n: u32) -> ClauseLabel {
        ClauseLabel::try_from(format!("C{n}").as_str()).unwrap()
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
            label: label(c),
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
        ] {
            assert_eq!(CaseExit::from_word(&e.word()), Some(e));
        }
        assert_eq!(CaseExit::from_word("script:x"), None);
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
        assert_eq!(found[0].label, label(2));
        assert_eq!(found[1].label, label(4));
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
                label: label(c),
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
        assert_eq!(unreached[0].label, label(5));
        assert_eq!(unreached[0].text, "needs 5");
        assert!(SimOutcome::default().unreached().is_empty());
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

    #[test]
    fn a_stamp_holds_for_what_it_ran_and_not_for_any_moved_part() {
        let stamp = SimStamp::new(basis(Revision::first(), Some(b"code")), "s1").unwrap();
        assert!(stamp.holds(&basis(Revision::first(), Some(b"code")), Some("s1")));
        assert!(stamp.holds(&basis(Revision::first(), Some(b"code")), None));
        assert!(!stamp.holds(&basis(Revision::first(), Some(b"code")), Some("s2")));
        assert!(!stamp.holds(&basis(Revision::first().next(), Some(b"code")), None));
        assert!(!stamp.holds(&basis(Revision::first(), Some(b"other")), None));
        assert!(!stamp.holds(&basis(Revision::first(), None), None));
        assert_eq!(
            SimStamp::new(basis(Revision::first(), None), " "),
            Err(SimError::BlankSettings)
        );
        let print = |r: Revision, p: Option<&[u8]>, s: &str| {
            SimStamp::new(basis(r, p), s).unwrap().fingerprint()
        };
        let base = print(Revision::first(), Some(b"c"), "s");
        assert_eq!(base, print(Revision::first(), Some(b"c"), "s"));
        assert_ne!(base, print(Revision::first().next(), Some(b"c"), "s"));
        assert_ne!(base, print(Revision::first(), Some(b"d"), "s"));
        assert_ne!(base, print(Revision::first(), Some(b"c"), "t"));
    }
}
