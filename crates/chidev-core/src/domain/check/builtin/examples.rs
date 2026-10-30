//! Checks on the examples of a Harness: whether a Harness of examples was ever run, whether the latest run still
//! describes the model, whether it could finish, and which examples it found failing. They read what a run left in
//! the store and run nothing: running a program once per example takes as long as the program, so `measure` runs it
//! and these only judge its record.
//!
//! None of them is mandatory, so none refuses a pin, a resolution or any other use case, whatever it finds: a
//! builtin check's `mandatory` is fixed by the check and a policy cannot raise it. A project that wants a
//! confirmation refused for a failing example registers an external gate of its own.
//!
//! A run that no longer describes the model (an example, a built Interface or the code pinned for it, or the
//! settings moved) is judged stale before anything else, so a failing example of an old run is never reported for a
//! program that has since changed (`examples.failed` skips a stale run, and `examples.stale` skips a run that
//! failed). A run that could not finish is reported by `examples.error` whether or not the model has moved since,
//! as `sim.error` reports one: it could not finish, whatever it stood on. The checks that say a run is stale, could
//! not finish or found a failing example also run where something is confirmed, since a confirmation must not claim
//! a state the latest run contradicts.

use std::borrow::Cow;

use super::CONFIRM_MOMENTS;
use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Level, Moment};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::harness::Harness;
use crate::domain::entity::measurement::ExamplesRun;
use crate::domain::service::examples::examples_harnesses;
use crate::domain::service::measure::{Standing, examples_standing, latest_examples};
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::measure::SettingsNow;

/// A Harness of examples with the latest run of it (if any).
struct Examples<'a> {
    harness: &'a Harness,
    /// The Subject of a finding about the Harness as a whole: its first Case.
    subject: Subject,
    run: Option<ExamplesRun<'a>>,
}

impl Examples<'_> {
    /// What the settings of a run of the Harness's examples are now.
    fn settings(&self, view: &CheckView<'_>) -> SettingsNow {
        view.examples_settings_now(self.harness.id())
    }

    /// How the latest run stands, if there is one. A run whose settings cannot be compared is stale.
    fn standing(&self, view: &CheckView<'_>) -> Option<Standing> {
        self.run
            .map(|run| examples_standing(run, view.project, view.pins, &self.settings(view)))
    }

    /// Whether there is a run, and it no longer describes the model.
    fn is_stale(&self, view: &CheckView<'_>) -> bool {
        self.standing(view).is_some_and(|s| s.is_stale())
    }
}

/// The Harnesses of examples of the Project that have a Case, each with its latest run. One without a Case has no
/// example to run.
fn harnesses<'a>(view: &'a CheckView<'a>) -> impl Iterator<Item = Examples<'a>> + 'a {
    view.project.contexts().flat_map(move |c| {
        examples_harnesses(c).filter_map(move |h| {
            Some(Examples {
                harness: h,
                subject: Subject::Case(h.cases().first()?.id()),
                run: latest_examples(view.measurements, h.id()),
            })
        })
    })
}

/// A Harness of examples that was never run.
#[derive(Debug, Clone, Copy)]
pub struct ExamplesUnrun;

static EXAMPLES_UNRUN: Descriptor = Descriptor {
    id: RuleId::from_static("examples.unrun"),
    summary: Cow::Borrowed("a harness of examples that was never run"),
    help: Cow::Borrowed(
        "Nothing records what the program answers to the examples. Run the examples measure for the harness once \
           the program it builds is pinned. Until then no clause the examples cite is judged by a run.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for ExamplesUnrun {
    fn descriptor(&self) -> &'static Descriptor {
        &EXAMPLES_UNRUN
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        harnesses(view)
            .filter(|e| e.run.is_none())
            .map(|e| {
                Finding::new(
                    &EXAMPLES_UNRUN,
                    Some(e.subject),
                    "the harness of examples was never run",
                )
            })
            .collect()
    }
}

/// A run of examples of a model that has since moved.
#[derive(Debug, Clone, Copy)]
pub struct ExamplesStale;

static EXAMPLES_STALE: Descriptor = Descriptor {
    id: RuleId::from_static("examples.stale"),
    summary: Cow::Borrowed(
        "a run of examples after which an example of the harness, an interface it builds, the code pinned for it, or the settings moved; a run that failed is told by examples.error",
    ),
    help: Cow::Borrowed(
        "An example of the harness, the interfaces it `builds`, the code pinned for the program, or the measure \
           settings changed after the run. Its result says nothing of the program as it is now. Run the examples \
           measure again.",
    ),
    moments: Cow::Borrowed(CONFIRM_MOMENTS),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for ExamplesStale {
    fn descriptor(&self) -> &'static Descriptor {
        &EXAMPLES_STALE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        harnesses(view)
            .filter_map(|e| {
                let run = e.run?;
                if run.measurement().failure().is_some() {
                    return None;
                }
                let Some(Standing::Stale(moved)) = e.standing(view) else {
                    return None;
                };
                // Every part that moved is said; settings that cannot be compared say why, for the cure is not to
                // run again but to configure `[measure]` or to make its settings readable.
                Some(Finding::new(
                    &EXAMPLES_STALE,
                    Some(e.subject),
                    format!("the latest run of the examples is stale: {}", moved.said()),
                ))
            })
            .collect()
    }
}

/// The latest run of examples could not finish.
#[derive(Debug, Clone, Copy)]
pub struct ExamplesError;

static EXAMPLES_ERROR: Descriptor = Descriptor {
    id: RuleId::from_static("examples.error"),
    summary: Cow::Borrowed(
        "a run of examples that failed (no_build, budget, io, settings_moved, ...): no example was judged",
    ),
    help: Cow::Borrowed(
        "The run could not finish: the program does not build, the time ran out, or a program of the run is not \
           the one the settings were digested with. The evidence of the run is in its record. Fix the cause and \
           run the examples measure again; a failed run is not judged for failing examples.",
    ),
    moments: Cow::Borrowed(CONFIRM_MOMENTS),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for ExamplesError {
    fn descriptor(&self) -> &'static Descriptor {
        &EXAMPLES_ERROR
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        harnesses(view)
            .filter_map(|e| {
                let failure = e.run?.measurement().failure()?;
                Some(Finding::new(
                    &EXAMPLES_ERROR,
                    Some(e.subject),
                    format!("the latest run of the examples failed ({failure})"),
                ))
            })
            .collect()
    }
}

/// An example of the latest run that failed.
#[derive(Debug, Clone, Copy)]
pub struct ExamplesFailed;

static EXAMPLES_FAILED: Descriptor = Descriptor {
    id: RuleId::from_static("examples.failed"),
    summary: Cow::Borrowed(
        "an example that failed in the latest run, one per clause its case cites, with the example's name and why; it refuses no change, and a stale run or a run that failed is not judged for it",
    ),
    help: Cow::Borrowed(
        "The program did not answer as the example expects. Either the program is wrong (change it and pin it \
           again), or the example and the clause read differently (a change request on the clause or the example). \
           The finding is about each clause the example's case cites, with the example's name and the reasons. \
           This rule reports and does not refuse: to have a confirmation refused for a failing example, register \
           an external gate.",
    ),
    moments: Cow::Borrowed(CONFIRM_MOMENTS),
    default_level: Level::Error,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for ExamplesFailed {
    fn descriptor(&self) -> &'static Descriptor {
        &EXAMPLES_FAILED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let mut out = Vec::new();
        for e in harnesses(view) {
            let Some(run) = e.run else { continue };
            let m = run.measurement();
            if m.failure().is_some() || e.is_stale(view) {
                continue;
            }
            let Some(outcome) = m.examples() else {
                continue;
            };
            for (case, reasons) in outcome.failed() {
                // The record holds the Case's id, so a Case that was renamed since is still the one. A run that is
                // not stale was made on the Harness as it is, so the Case is there; a Case that is not cites nothing
                // to report about.
                let Some(found) = e.harness.case(case) else {
                    continue;
                };
                let why = reasons
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                let message = format!("example `{}` failed: {why}", found.name());
                for clause in found.cites().clauses().keys() {
                    out.push(Finding::new(
                        &EXAMPLES_FAILED,
                        Some(Subject::Clause(*clause)),
                        message.clone(),
                    ));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::super::fixture::*;
    use super::*;
    use crate::domain::entity::measurement::Measurement;
    use crate::domain::entity::measurement::fixture::{examples_draft, same_settings, settings_of};
    use crate::domain::entity::pin::Pin;
    use crate::domain::entity::project::Project;
    use crate::domain::service::examples::basis_of;
    use crate::domain::service::pin::record;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::example::Example;
    use crate::domain::vo::examples_run::{
        ExampleRecord, ExampleResult, ExamplesOutcome, Got, Reason,
    };
    use crate::domain::vo::id::{CaseId, HarnessId, PinId, test_uuid};
    use crate::domain::vo::measure::{FailureKind, MeasureFailure};
    use crate::domain::vo::revision::Revision;

    fn hid() -> HarnessId {
        HarnessId::from_uuid(test_uuid(600))
    }

    fn case_id(n: u128) -> CaseId {
        CaseId::from_uuid(test_uuid(n))
    }

    /// The examples `a1` (Case 60, citing C1 and C2) and `a2` (Case 61, citing C1), the expected output of `a1`
    /// being `expected`.
    fn put_examples(p: &mut Project, expected: &str) {
        put_examples_named(p, "a1", expected);
    }

    /// As [`put_examples`], the first Case being named `first`.
    fn put_examples_named(p: &mut Project, first: &str, expected: &str) {
        let cites = |clauses: &[u128]| {
            CaseCites::of(
                BTreeMap::new(),
                clauses
                    .iter()
                    .map(|c| (clause(*c), Revision::first()))
                    .collect(),
            )
        };
        let example = |text: &str| {
            Example::new(String::new(), String::new(), text.to_string(), None).unwrap()
        };
        p.put_examples_harness(
            ctx(),
            hid(),
            BTreeSet::from([iface()]),
            vec![
                (
                    case_id(60),
                    CaseName::try_from(first).unwrap(),
                    example(expected),
                    cites(&[30, 31]),
                ),
                (
                    case_id(61),
                    CaseName::try_from("a2").unwrap(),
                    example("#exit 0\n"),
                    cites(&[30]),
                ),
            ],
        )
        .unwrap();
    }

    fn examples_project() -> Project {
        let mut p = project();
        put_examples(&mut p, "#exit 0\n");
        p
    }

    fn passed(n: u128) -> ExampleRecord {
        ExampleRecord {
            case: case_id(n),
            result: ExampleResult::Passed,
        }
    }

    fn failed(n: u128, reasons: Vec<Reason>) -> ExampleRecord {
        ExampleRecord {
            case: case_id(n),
            result: ExampleResult::Failed(reasons),
        }
    }

    /// A run of the Harness as the Project is now (with these Pins), which gave these records.
    fn ran(p: &Project, pins: &[Pin], n: u128, records: Vec<ExampleRecord>) -> Measurement {
        Measurement::new(examples_draft(
            n,
            hid(),
            basis_of(p, hid(), pins).unwrap(),
            ExamplesOutcome::new(records).unwrap(),
        ))
        .unwrap()
    }

    fn found(
        check: &dyn Check,
        p: &Project,
        pins: &[Pin],
        runs: &[Measurement],
        settings: SettingsNow,
    ) -> Vec<Finding> {
        check.run(
            &CheckView {
                project: p,
                pins,
                change_requests: &[],
                cruxes: &[],
                measurements: runs,
                measure_settings: &BTreeMap::new(),
                sim_settings: &BTreeMap::new(),
                examples_settings: &BTreeMap::from([(hid(), settings)]),
                resolving: None,
            },
            &Params::default(),
        )
    }

    fn all() -> [&'static dyn Check; 4] {
        [
            &ExamplesUnrun as &dyn Check,
            &ExamplesStale,
            &ExamplesError,
            &ExamplesFailed,
        ]
    }

    fn exit_1() -> Vec<Reason> {
        vec![Reason::Exit { got: 1 }]
    }

    #[test]
    fn a_project_without_a_harness_of_examples_has_nothing_for_these_rules_to_say() {
        let p = project();
        for check in all() {
            assert!(found(check, &p, &[], &[], same_settings()).is_empty());
        }
    }

    #[test]
    fn a_harness_of_examples_is_found_until_it_is_run_and_the_finding_is_about_its_first_case() {
        let p = examples_project();
        let unrun = found(&ExamplesUnrun, &p, &[], &[], same_settings());
        assert_eq!(unrun.len(), 1);
        assert_eq!(unrun[0].subject(), Some(Subject::Case(case_id(60))));
        assert_eq!(unrun[0].message(), "the harness of examples was never run");
        let runs = [ran(&p, &[], 1, vec![passed(60), passed(61)])];
        assert!(found(&ExamplesUnrun, &p, &[], &runs, same_settings()).is_empty());
        // only a harness of examples is found: the tests harness of the fixture is not
        assert_eq!(
            found(&ExamplesUnrun, &project(), &[], &[], same_settings()).len(),
            0
        );
    }

    #[test]
    fn a_run_that_describes_the_model_is_current_and_judged_by_what_it_found() {
        let p = examples_project();
        let runs = [ran(&p, &[], 1, vec![passed(60), passed(61)])];
        for check in all() {
            assert!(
                found(check, &p, &[], &runs, same_settings()).is_empty(),
                "{}",
                check.descriptor().id
            );
        }
    }

    #[test]
    fn an_example_that_failed_is_one_finding_for_each_clause_its_case_cites() {
        let p = examples_project();
        let why = vec![
            Reason::Output(Got::Line {
                line: 2,
                got: Some("two".to_string()),
            }),
            Reason::Exit { got: 3 },
        ];
        let runs = [ran(&p, &[], 1, vec![failed(60, why), passed(61)])];
        let all = found(&ExamplesFailed, &p, &[], &runs, same_settings());
        // a1 cites C1 (30) and C2 (31): one finding for each, in the order of the ids
        assert_eq!(
            all.iter().map(Finding::subject).collect::<Vec<_>>(),
            [
                Some(Subject::Clause(clause(30))),
                Some(Subject::Clause(clause(31)))
            ]
        );
        for f in &all {
            assert_eq!(
                f.message(),
                "example `a1` failed: standard output differs at line 2: got \"two\"; exit code: got 3"
            );
            assert_eq!(f.level(), Level::Error);
        }
        // a2, which cites C1 alone, failing too: C1 has two findings, one for each example
        let runs = [ran(
            &p,
            &[],
            1,
            vec![failed(60, exit_1()), failed(61, exit_1())],
        )];
        let both = found(&ExamplesFailed, &p, &[], &runs, same_settings());
        assert_eq!(
            both.iter().map(Finding::subject).collect::<Vec<_>>(),
            [
                Some(Subject::Clause(clause(30))),
                Some(Subject::Clause(clause(31))),
                Some(Subject::Clause(clause(30)))
            ]
        );
        assert_eq!(both[2].message(), "example `a2` failed: exit code: got 1");
    }

    #[test]
    fn the_name_of_an_example_is_the_one_the_case_has_now() {
        // the record keeps the id of the Case; the Case is named in the model, here under another name than the one
        // it was run under, and the run is made on the Harness as it is
        let mut p = examples_project();
        put_examples_named(&mut p, "renamed", "#exit 0\n");
        let runs = [ran(&p, &[], 1, vec![failed(60, exit_1()), passed(61)])];
        let all = found(&ExamplesFailed, &p, &[], &runs, same_settings());
        assert_eq!(all.len(), 2);
        assert!(
            all.iter()
                .all(|f| f.message() == "example `renamed` failed: exit code: got 1")
        );
    }

    #[test]
    fn a_changed_example_makes_the_run_stale_naming_the_examples_and_it_is_not_judged_for_failures()
    {
        let mut p = examples_project();
        let runs = [ran(&p, &[], 1, vec![failed(60, exit_1()), passed(61)])];
        assert_eq!(
            found(&ExamplesFailed, &p, &[], &runs, same_settings()).len(),
            2
        );
        put_examples(&mut p, "1\n#exit 0\n");
        let stale = found(&ExamplesStale, &p, &[], &runs, same_settings());
        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0].message(),
            "the latest run of the examples is stale: examples"
        );
        assert_eq!(stale[0].subject(), Some(Subject::Case(case_id(60))));
        // no failure of an old run is reported for an example that has since changed
        assert!(found(&ExamplesFailed, &p, &[], &runs, same_settings()).is_empty());
        assert!(found(&ExamplesError, &p, &[], &runs, same_settings()).is_empty());
    }

    #[test]
    fn code_pinned_again_for_a_built_interface_makes_the_run_stale() {
        let p = examples_project();
        let pin = |code: &[u8]| {
            record(
                &p,
                iface(),
                PinId::from_uuid(test_uuid(70)),
                Digest::of(code),
            )
            .unwrap()
        };
        let runs = [ran(&p, &[pin(b"one")], 1, vec![passed(60), passed(61)])];
        assert!(found(&ExamplesStale, &p, &[pin(b"one")], &runs, same_settings()).is_empty());
        let stale = found(&ExamplesStale, &p, &[pin(b"two")], &runs, same_settings());
        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0].message(),
            "the latest run of the examples is stale: built-code"
        );
        // and a first Pin, or a lost one, is code pinned that is another
        let none = found(&ExamplesStale, &p, &[], &runs, same_settings());
        assert_eq!(none.len(), 1);
    }

    #[test]
    fn settings_that_differ_or_cannot_be_compared_make_the_run_stale_and_say_why() {
        let p = examples_project();
        let runs = [ran(&p, &[], 1, vec![passed(60), passed(61)])];
        let said = |now: SettingsNow| {
            found(&ExamplesStale, &p, &[], &runs, now)
                .iter()
                .map(|f| f.message().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            said(SettingsNow::Known(settings_of("another"))),
            ["the latest run of the examples is stale: settings"]
        );
        assert_eq!(
            said(SettingsNow::Unconfigured),
            [
                "the latest run of the examples is stale: settings-unconfigured (measure is not configured)"
            ]
        );
        assert_eq!(
            said(SettingsNow::Unreadable("no layout".to_string())),
            [
                "the latest run of the examples is stale: settings-unreadable (the settings cannot be read: no layout)"
            ]
        );
    }

    #[test]
    fn a_run_that_failed_is_an_error_and_is_neither_stale_nor_judged_for_failures() {
        let mut p = examples_project();
        let mut draft = examples_draft(
            1,
            hid(),
            basis_of(&p, hid(), &[]).unwrap(),
            ExamplesOutcome::new(vec![failed(60, exit_1())]).unwrap(),
        );
        draft.failure = Some(MeasureFailure::of(
            FailureKind::NoBuild,
            "the program does not build",
        ));
        let runs = [Measurement::new(draft).unwrap()];
        let error = found(&ExamplesError, &p, &[], &runs, same_settings());
        assert_eq!(error.len(), 1);
        assert_eq!(
            error[0].message(),
            "the latest run of the examples failed (no_build: the program does not build)"
        );
        assert_eq!(error[0].level(), Level::Warning);
        assert!(found(&ExamplesFailed, &p, &[], &runs, same_settings()).is_empty());
        assert!(found(&ExamplesStale, &p, &[], &runs, same_settings()).is_empty());
        // still an error once the model has moved: the run could not finish, whatever it stood on
        put_examples(&mut p, "1\n#exit 0\n");
        assert_eq!(
            found(&ExamplesError, &p, &[], &runs, same_settings()).len(),
            1
        );
        assert!(found(&ExamplesStale, &p, &[], &runs, same_settings()).is_empty());
        assert!(found(&ExamplesUnrun, &p, &[], &runs, same_settings()).is_empty());
    }

    #[test]
    fn only_the_latest_run_of_a_harness_counts() {
        let p = examples_project();
        let runs = [
            ran(&p, &[], 1, vec![failed(60, exit_1()), passed(61)]),
            ran(&p, &[], 2, vec![passed(60), passed(61)]),
        ];
        assert!(found(&ExamplesFailed, &p, &[], &runs, same_settings()).is_empty());
        let runs = [runs[1].clone(), runs[0].clone()];
        assert_eq!(
            found(&ExamplesFailed, &p, &[], &runs, same_settings()).len(),
            2
        );
    }

    #[test]
    fn a_harness_whose_settings_were_not_read_is_stale() {
        let p = examples_project();
        let runs = [ran(&p, &[], 1, vec![passed(60), passed(61)])];
        let stale = ExamplesStale.run(
            &CheckView {
                project: &p,
                pins: &[],
                change_requests: &[],
                cruxes: &[],
                measurements: &runs,
                measure_settings: &BTreeMap::new(),
                sim_settings: &BTreeMap::new(),
                examples_settings: &BTreeMap::new(),
                resolving: None,
            },
            &Params::default(),
        );
        assert_eq!(stale.len(), 1);
        assert!(stale[0].message().contains("settings-unreadable"));
    }

    #[test]
    fn the_moments_levels_and_flags_of_the_rules_are_as_decided() {
        let at = |c: &dyn Check| c.descriptor().moments.to_vec();
        assert_eq!(at(&ExamplesUnrun), [Moment::Check]);
        for check in [
            &ExamplesStale as &dyn Check,
            &ExamplesError,
            &ExamplesFailed,
        ] {
            assert_eq!(
                at(check),
                [Moment::Pin, Moment::Resolve, Moment::Check],
                "{}",
                check.descriptor().id
            );
        }
        let level = |c: &dyn Check| c.descriptor().default_level;
        assert_eq!(level(&ExamplesUnrun), Level::Warning);
        assert_eq!(level(&ExamplesStale), Level::Warning);
        assert_eq!(level(&ExamplesError), Level::Warning);
        assert_eq!(level(&ExamplesFailed), Level::Error);
        // no rule of the examples is mandatory: none refuses anything, and a policy cannot make one
        for check in all() {
            assert!(!check.descriptor().mandatory, "{}", check.descriptor().id);
            assert!(check.descriptor().id.as_str().starts_with("examples."));
        }
    }
}
