//! Checks on the measurements kept for the Interfaces: whether the code was measured, whether the measurement still
//! describes it, whether the run failed, and what the Harness did not notice. They read what a run left in the
//! store and run nothing: a measure takes minutes, so it is an operation of its own and these only judge its record.
//! A measurement that no longer describes the Pin is judged stale before anything else, so a survivor of an old
//! run is never reported for code that has since moved.

use std::borrow::Cow;

use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::interface::Interface;
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::pin::Pin;
use crate::domain::service::measure::{Standing, latest, standing};
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::measure::MeasureKind;

/// The Interfaces of the Project that are pinned, with the latest mutation run of each (if any).
fn pinned<'a>(
    view: &'a CheckView<'a>,
) -> impl Iterator<Item = (&'a Interface, &'a Pin, Option<&'a Measurement>)> + 'a {
    view.after
        .contexts()
        .flat_map(|c| c.interfaces())
        .filter_map(|i| {
            let pin = view.pins.iter().find(|p| p.interface() == i.id())?;
            Some((
                i,
                pin,
                latest(view.measurements, MeasureKind::Mutate, i.id()),
            ))
        })
}

fn settings<'a>(view: &'a CheckView<'a>, m: &Measurement) -> Option<&'a str> {
    view.measure_settings
        .get(&m.interface()?)
        .map(String::as_str)
}

/// The latest mutation run of an Interface failed.
#[derive(Debug, Clone, Copy)]
pub struct MeasureFailed;

static MEASURE_FAILED: Descriptor = Descriptor {
    id: RuleId::from_static("measure.failed"),
    summary: Cow::Borrowed("the latest measurement of an interface's code failed"),
    help: Cow::Borrowed(
        "The run could not measure the code: the code and the harness did not pass before any change, the provider \
           or a command failed, or the time ran out. The evidence of the run is in its record. Fix the cause and run \
           the measure again; an earlier run is not used while the latest one failed.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for MeasureFailed {
    fn descriptor(&self) -> &'static Descriptor {
        &MEASURE_FAILED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        pinned(view)
            .filter_map(|(i, _, m)| {
                let failure = m?.failure()?;
                Some(Finding::new(
                    &MEASURE_FAILED,
                    Some(Subject::Interface(i.id())),
                    format!("the latest mutation run failed ({failure})"),
                ))
            })
            .collect()
    }
}

/// A pinned Interface whose code was never measured.
#[derive(Debug, Clone, Copy)]
pub struct Unmeasured;

static UNMEASURED: Descriptor = Descriptor {
    id: RuleId::from_static("mutation.unmeasured"),
    summary: Cow::Borrowed("a pinned interface whose code was never mutation-tested"),
    help: Cow::Borrowed(
        "Nothing records how strong the harness is against changes of this code. Run the mutate measure for the \
           interface once its harness is in place.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Note,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for Unmeasured {
    fn descriptor(&self) -> &'static Descriptor {
        &UNMEASURED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        pinned(view)
            .filter(|(_, _, m)| m.is_none())
            .map(|(i, _, _)| {
                Finding::new(
                    &UNMEASURED,
                    Some(Subject::Interface(i.id())),
                    "the code of this interface was never mutation-tested",
                )
            })
            .collect()
    }
}

/// A measurement of code that has since moved.
#[derive(Debug, Clone, Copy)]
pub struct Stale;

static STALE: Descriptor = Descriptor {
    id: RuleId::from_static("mutation.stale"),
    summary: Cow::Borrowed("a mutation run that no longer describes the interface's code"),
    help: Cow::Borrowed(
        "The pinned code, the interface, a harness that checks it, a contract it cites, or the measure settings \
           changed after the run. Its result says nothing of the code as it is now. Run the measure again.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for Stale {
    fn descriptor(&self) -> &'static Descriptor {
        &STALE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        pinned(view)
            .filter_map(|(i, pin, m)| {
                let m = m?;
                if m.failure().is_some()
                    || standing(m, view.after, pin, settings(view, m)) == Standing::Current
                {
                    return None;
                }
                Some(Finding::new(
                    &STALE,
                    Some(Subject::Interface(i.id())),
                    "the code, or what it stands on, has moved since the latest mutation run",
                ))
            })
            .collect()
    }
}

/// A mutant no test of the Harness noticed.
#[derive(Debug, Clone, Copy)]
pub struct Survived;

static SURVIVED: Descriptor = Descriptor {
    id: RuleId::from_static("mutation.survived"),
    summary: Cow::Borrowed("a change of the code that no case of the harness noticed"),
    help: Cow::Borrowed(
        "The code was changed at one place and every test still passed, so the harness does not check what that \
           place does. Either a case is missing (change the harness), or the change does not alter behaviour (an \
           equivalent mutant) and the survivor can be accepted with the reason.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Note,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for Survived {
    fn descriptor(&self) -> &'static Descriptor {
        &SURVIVED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        pinned(view)
            .filter_map(|(i, pin, m)| {
                let m = m?;
                if m.failure().is_some()
                    || standing(m, view.after, pin, settings(view, m)) == Standing::Stale
                {
                    return None;
                }
                Some(m.survivors().map(move |s| {
                    Finding::new(
                        &SURVIVED,
                        Some(Subject::Interface(i.id())),
                        s.mutant.describe(),
                    )
                }))
            })
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::fixture::*;
    use super::*;
    use crate::domain::entity::measurement::fixture::{draft, result};
    use crate::domain::entity::measurement::{Measurement, MeasurementDraft};
    use crate::domain::service::pin::current_basis;
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::InterfaceId;
    use crate::domain::vo::measure::{FailureKind, MeasureFailure, MutantOutcome};

    /// Runs `check` over the fixture Project with its Pin and these measurements, the settings being `settings`.
    fn found(
        check: &dyn Check,
        p: &crate::domain::entity::project::Project,
        pins: &[Pin],
        measurements: &[Measurement],
        settings: &BTreeMap<InterfaceId, String>,
    ) -> Vec<Finding> {
        check.run(
            &CheckView {
                before: None,
                after: p,
                pins,
                change_requests: &[],
                measurements,
                measure_settings: settings,
                sim_settings: &BTreeMap::new(),
            },
            &Params::default(),
        )
    }

    /// A run of the pinned code of the fixture Project, standing on what the Project stands on now.
    fn run_of(
        p: &crate::domain::entity::project::Project,
        pin: &Pin,
        n: u128,
        mutants: Vec<crate::domain::vo::measure::MutantResult>,
    ) -> MeasurementDraft {
        let basis = current_basis(p, iface()).unwrap();
        draft(n, iface(), pin.code(), basis, mutants)
    }

    #[test]
    fn a_pinned_interface_is_unmeasured_until_a_run_exists() {
        let p = project();
        let none = BTreeMap::new();
        // not pinned: nothing to measure yet
        assert!(found(&Unmeasured, &p, &[], &[], &none).is_empty());
        let pins = [pin(&p)];
        let first = found(&Unmeasured, &p, &pins, &[], &none);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].subject(), Some(Subject::Interface(iface())));
        assert_eq!(first[0].level(), Level::Note);
        let run = Measurement::new(run_of(&p, &pins[0], 1, vec![])).unwrap();
        assert!(found(&Unmeasured, &p, &pins, &[run], &none).is_empty());
    }

    #[test]
    fn each_survivor_is_a_finding_with_its_site_and_operator() {
        let p = project();
        let pins = [pin(&p)];
        let run = Measurement::new(run_of(
            &p,
            &pins[0],
            1,
            vec![
                result("f/rel/1/aaaa", 7, MutantOutcome::Survived),
                result("f/rel/2/bbbb", 9, MutantOutcome::Killed),
                result("f/rel/3/cccc", 11, MutantOutcome::Survived),
            ],
        ))
        .unwrap();
        let none = BTreeMap::new();
        let all = found(&Survived, &p, &pins, std::slice::from_ref(&run), &none);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].subject(), Some(Subject::Interface(iface())));
        assert_eq!(all[0].level(), Level::Note);
        assert_eq!(
            all[0].message(),
            "f/rel/1/aaaa: line 7, column 5, rel `<` -> `<=`"
        );
        assert!(all[1].message().contains("line 11"));
        assert!(found(&Stale, &p, &pins, &[run], &none).is_empty());
    }

    #[test]
    fn a_run_is_stale_when_the_harness_the_interface_or_a_contract_moves_and_then_reports_no_survivor()
     {
        let mut p = project();
        let pins = [pin(&p)];
        let run = Measurement::new(run_of(
            &p,
            &pins[0],
            1,
            vec![result("f/rel/1/aaaa", 7, MutantOutcome::Survived)],
        ))
        .unwrap();
        let runs = std::slice::from_ref(&run);
        let none = BTreeMap::new();
        assert!(found(&Stale, &p, &pins, runs, &none).is_empty());

        // The Harness moves (its revision is what the stamp records).
        let case = |p: &crate::domain::entity::project::Project| {
            p.contexts()
                .flat_map(|c| c.harnesses())
                .next()
                .map(|h| h.revision())
        };
        let before = case(&p);
        revise_harness(&mut p);
        assert_ne!(before, case(&p));
        let stale = found(&Stale, &p, &pins, runs, &none);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].level(), Level::Warning);
        assert_eq!(stale[0].subject(), Some(Subject::Interface(iface())));
        // stale comes before the verdict: the old survivor is not reported for moved code
        assert!(found(&Survived, &p, &pins, runs, &none).is_empty());

        // The Interface moves.
        let mut q = project();
        put_interface(&mut q, "fn a(x: u8)");
        assert_eq!(found(&Stale, &q, &pins, runs, &none).len(), 1);
        // A Contract the Harness cites moves.
        let mut r = project();
        put_contract(&mut r, "a half rounds to even");
        assert_eq!(found(&Stale, &r, &pins, runs, &none).len(), 1);
    }

    #[test]
    fn a_run_is_stale_when_the_pin_holds_other_code_or_the_settings_differ() {
        let p = project();
        let pins = [pin(&p)];
        let run = Measurement::new(run_of(&p, &pins[0], 1, vec![])).unwrap();
        let runs = std::slice::from_ref(&run);
        // the run recorded the settings "settings"; the runner says the same, or cannot say
        let same = BTreeMap::from([(iface(), "settings".to_string())]);
        let other = BTreeMap::from([(iface(), "settings-2".to_string())]);
        assert!(found(&Stale, &p, &pins, runs, &same).is_empty());
        assert!(found(&Stale, &p, &pins, runs, &BTreeMap::new()).is_empty());
        assert_eq!(found(&Stale, &p, &pins, runs, &other).len(), 1);
        // pinned again with other code
        let repinned = [crate::domain::service::pin::record(
            &p,
            iface(),
            crate::domain::vo::id::PinId::from_uuid(crate::domain::vo::id::test_uuid(51)),
            Digest::of(b"other code"),
        )
        .unwrap()];
        assert_eq!(found(&Stale, &p, &repinned, runs, &same).len(), 1);
    }

    #[test]
    fn a_failed_latest_run_is_reported_and_judged_by_nothing_else() {
        let p = project();
        let pins = [pin(&p)];
        let mut failed = run_of(
            &p,
            &pins[0],
            2,
            vec![result("f/rel/1/aaaa", 7, MutantOutcome::Survived)],
        );
        failed.failure = Some(
            MeasureFailure::new(
                FailureKind::Baseline,
                "the harness failed before any change",
            )
            .unwrap(),
        );
        let run = Measurement::new(failed).unwrap();
        let runs = std::slice::from_ref(&run);
        let none = BTreeMap::new();
        let all = found(&MeasureFailed, &p, &pins, runs, &none);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].level(), Level::Warning);
        assert!(all[0].message().contains("baseline: the harness failed"));
        assert!(found(&Survived, &p, &pins, runs, &none).is_empty());
        assert!(found(&Stale, &p, &pins, runs, &none).is_empty());
        assert!(found(&Unmeasured, &p, &pins, runs, &none).is_empty());
        // an ok latest run is not a failure
        let ok = Measurement::new(run_of(&p, &pins[0], 3, vec![])).unwrap();
        assert!(found(&MeasureFailed, &p, &pins, &[ok], &none).is_empty());
    }

    #[test]
    fn only_the_latest_run_of_an_interface_counts() {
        let p = project();
        let pins = [pin(&p)];
        let old = Measurement::new(run_of(
            &p,
            &pins[0],
            1,
            vec![result("f/rel/1/aaaa", 7, MutantOutcome::Survived)],
        ))
        .unwrap();
        let newer = Measurement::new(run_of(&p, &pins[0], 2, vec![])).unwrap();
        let none = BTreeMap::new();
        assert!(found(&Survived, &p, &pins, &[old, newer], &none).is_empty());
    }
}
