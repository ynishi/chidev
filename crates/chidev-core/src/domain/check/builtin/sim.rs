//! Checks on the simulations: whether a sim Harness has the shape the protocol runs, whether every Clause of the
//! Contracts it takes on is named by it, and what the latest simulation found. They read what a run left in the store and run
//! nothing: a simulation takes as long as its program, so `measure` runs it and these only judge its record.
//!
//! A simulation that no longer describes the model (the script, a Contract or an Interface it cites, or the code
//! pinned for it moved) is judged stale before anything else, so a finding of an old run is never reported for a
//! program that has since changed. One thing is judged even then: a second run that did not reproduce the first
//! says the script or the program is not deterministic, and no later change makes that untrue of that run.

use std::borrow::Cow;
use std::collections::BTreeSet;

use super::ALL_HOOKS;
use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::context::Context;
use crate::domain::entity::harness::{Case, Harness};
use crate::domain::entity::measurement::Measurement;
use crate::domain::entity::project::Project;
use crate::domain::service::measure::{Standing, latest_sim, sim_standing};
use crate::domain::service::sim::{ShapeAbout, shape, sim_harnesses, sweep};
use crate::domain::vo::clause::ClauseLabel;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::id::ClauseId;
use crate::domain::vo::sim::{CaseRole, Reproduction, SimCase, role_of};

/// A sim Harness with its sweep Case and the latest simulation of it (if any).
struct Sim<'a> {
    harness: &'a Harness,
    sweep: &'a Case,
    run: Option<&'a Measurement>,
}

impl Sim<'_> {
    /// Whether the run still describes the model. A run that cannot be compared is stale.
    fn standing(&self, view: &CheckView<'_>) -> Option<Standing> {
        let m = self.run?;
        Some(sim_standing(
            m,
            view.after,
            view.pins,
            view.sim_settings
                .get(&self.harness.id())
                .map(String::as_str),
        ))
    }
}

/// The sim Harnesses of the Project that have a sweep Case (one without is a fault of shape, found by
/// `sim.shape`), each with its latest simulation.
fn sims<'a>(view: &'a CheckView<'a>) -> impl Iterator<Item = (&'a Context, Sim<'a>)> + 'a {
    view.after.contexts().flat_map(move |c| {
        sim_harnesses(c).filter_map(move |h| {
            Some((
                c,
                Sim {
                    harness: h,
                    sweep: sweep(h)?,
                    run: latest_sim(view.measurements, h.id()),
                },
            ))
        })
    })
}

/// The label of the Clause `id` in the model; none when no Contract has it.
fn label_of(project: &Project, id: ClauseId) -> Option<ClauseLabel> {
    project
        .contexts()
        .find_map(|c| c.contract_of(id))
        .and_then(|k| k.clause(id))
        .map(|c| c.content().label())
}

/// A sim Harness is not the shape the protocol runs.
#[derive(Debug, Clone, Copy)]
pub struct SimShape;

static SIM_SHAPE: Descriptor = Descriptor {
    id: RuleId::from_static("sim.shape"),
    summary: Cow::Borrowed(
        "a sim harness that is not one `sweep` case and `regress-<name>` cases citing clauses",
    ),
    help: Cow::Borrowed(
        "A sim harness is run case by case under the protocol chidev-sim 2: the one case named `sweep` stands for \
           the seeded runs and cites every clause the script checks (of any of the context's contracts; the script \
           names a clause `<contract>:<label>`), each `regress-<name>` case stands for a fixed schedule, and each \
           cites the clauses it checks and no item. The harness names in `builds` the interfaces whose pinned code \
           the program is built from. A context has at most one. Rename or add the cases, cite the clauses, name \
           the interfaces to build, and put the harness again.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Check]),
    default_level: Level::Error,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimShape {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_SHAPE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        view.after
            .contexts()
            .flat_map(shape)
            .map(|p| {
                let subject = match p.about {
                    ShapeAbout::Context(c) => Subject::Context(c),
                    ShapeAbout::Case(c) => Subject::Case(c),
                };
                Finding::new(&SIM_SHAPE, Some(subject), p.message)
            })
            .collect()
    }
}

/// A Clause of a Contract that a sim Harness takes on (cites a Clause of), which no sim Case cites.
#[derive(Debug, Clone, Copy)]
pub struct SimUncitedClause;

static SIM_UNCITED_CLAUSE: Descriptor = Descriptor {
    id: RuleId::from_static("sim.uncited-clause"),
    summary: Cow::Borrowed(
        "a clause of a contract a sim harness takes on (cites a clause of) that no sim case cites",
    ),
    help: Cow::Borrowed(
        "The simulation takes this contract on (the sweep cites a clause of it) but never checks this clause. \
           Have the script check it and cite it in the sweep case, or open a change request that addresses the \
           finding if the clause is not for a simulation to check. A contract the sweep cites no clause of is \
           left to the tests, and is not reported.",
    ),
    hooks: Cow::Borrowed(ALL_HOOKS),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimUncitedClause {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_UNCITED_CLAUSE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let mut out = Vec::new();
        for c in view.after.contexts() {
            let cited: BTreeSet<ClauseId> = sim_harnesses(c)
                .flat_map(|h| h.cases())
                .flat_map(|k| k.cites().clauses().keys().copied())
                .collect();
            // Only a Contract the sim cites at least one Clause of is its to cover; the others are for the tests.
            for contract in c
                .contracts()
                .filter(|k| k.clauses().iter().any(|cl| cited.contains(&cl.id())))
            {
                for clause in contract.clauses() {
                    if !cited.contains(&clause.id()) {
                        out.push(Finding::new(
                            &SIM_UNCITED_CLAUSE,
                            Some(Subject::Clause(clause.id())),
                            format!("no sim case cites clause {}", clause.content().label()),
                        ));
                    }
                }
            }
        }
        out
    }
}

/// A sim Harness that was never simulated.
#[derive(Debug, Clone, Copy)]
pub struct SimUnrun;

static SIM_UNRUN: Descriptor = Descriptor {
    id: RuleId::from_static("sim.unrun"),
    summary: Cow::Borrowed("a sim harness that was never simulated"),
    help: Cow::Borrowed(
        "Nothing records what the script finds in the program. Run the simulate measure for the harness once the \
           program it drives is pinned.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimUnrun {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_UNRUN
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        sims(view)
            .filter(|(_, s)| s.run.is_none())
            .map(|(_, s)| {
                Finding::new(
                    &SIM_UNRUN,
                    Some(Subject::Case(s.sweep.id())),
                    "the sim harness was never simulated",
                )
            })
            .collect()
    }
}

/// A simulation of a model that has since moved.
#[derive(Debug, Clone, Copy)]
pub struct SimStale;

static SIM_STALE: Descriptor = Descriptor {
    id: RuleId::from_static("sim.stale"),
    summary: Cow::Borrowed("a simulation that no longer describes the sim harness and the program"),
    help: Cow::Borrowed(
        "The script, the cases or `builds` of the harness, a contract it cites a clause of, a built interface, \
           the code pinned for the program, or the measure \
           settings changed after the run. Its result says nothing of the program as it is now. Run the simulate \
           measure again.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimStale {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_STALE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        sims(view)
            .filter(|(_, s)| {
                s.run.is_some_and(|m| m.failure().is_none())
                    && s.standing(view) == Some(Standing::Stale)
            })
            .map(|(_, s)| {
                Finding::new(
                    &SIM_STALE,
                    Some(Subject::Case(s.sweep.id())),
                    "the script, what it cites, the pinned code or the settings have moved since the latest simulation",
                )
            })
            .collect()
    }
}

/// A Clause the program broke in some case of the latest simulation.
#[derive(Debug, Clone, Copy)]
pub struct SimFinding;

static SIM_FINDING: Descriptor = Descriptor {
    id: RuleId::from_static("sim.finding"),
    summary: Cow::Borrowed("a clause the program broke in a simulated case"),
    help: Cow::Borrowed(
        "The script drove the program through a schedule and saw a promise of this clause not hold. Either the \
           program is wrong (change it and pin it again), or the clause and the script read it differently (a \
           change request on the clause or the script). The case and step are in the message; run the case again \
           to see its schedule and trace.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Error,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimFinding {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_FINDING
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let mut out = Vec::new();
        for (_, s) in sims(view) {
            let Some(m) = s.run else { continue };
            if m.failure().is_some() || s.standing(view) == Some(Standing::Stale) {
                continue;
            }
            let Some(outcome) = m.sim() else { continue };
            for f in outcome.found() {
                // The record holds the Clause's id; a Clause the model has not any more is not reported.
                let Some(label) = label_of(view.after, f.clause) else {
                    continue;
                };
                out.push(Finding::new(
                    &SIM_FINDING,
                    Some(Subject::Clause(f.clause)),
                    format!(
                        "clause {label} broke in {} case(s); first {} step {}: {}",
                        f.cases, f.first_case, f.first_step, f.first_text
                    ),
                ));
            }
        }
        out
    }
}

/// A Clause whose situation no case of the latest simulation reached.
#[derive(Debug, Clone, Copy)]
pub struct SimUnreached;

static SIM_UNREACHED: Descriptor = Descriptor {
    id: RuleId::from_static("sim.unreached"),
    summary: Cow::Borrowed("a clause whose situation no simulated case reached"),
    help: Cow::Borrowed(
        "The script cites this clause but no seed drove the program to the situation the clause needs, so the \
           clause was not checked. Change the schedule the script makes (or add a regress case for the \
           situation) so that the situation comes up.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Error,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimUnreached {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_UNREACHED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let mut out = Vec::new();
        for (_, s) in sims(view) {
            let Some(m) = s.run else { continue };
            if m.failure().is_some() || s.standing(view) == Some(Standing::Stale) {
                continue;
            }
            let Some(outcome) = m.sim() else { continue };
            for u in outcome.unreached() {
                let Some(label) = label_of(view.after, u.clause) else {
                    continue;
                };
                out.push(Finding::new(
                    &SIM_UNREACHED,
                    Some(Subject::Clause(u.clause)),
                    format!(
                        "no seed reached the situation clause {label} needs: {}",
                        u.text
                    ),
                ));
            }
        }
        out
    }
}

/// A case that gave another schedule or another trace when it was run a second time.
#[derive(Debug, Clone, Copy)]
pub struct SimNondeterministic;

static SIM_NONDETERMINISTIC: Descriptor = Descriptor {
    id: RuleId::from_static("sim.nondeterministic"),
    summary: Cow::Borrowed("a simulated case that did not give the same schedule and trace twice"),
    help: Cow::Borrowed(
        "A case was run again in a fresh copy. A different schedule means the script draws something besides its \
           seed (a clock, the order of a set, an unseeded random number): fix the script. The same schedule with a \
           different trace means the program draws its own order: fix the program. A simulation that cannot be \
           repeated finds nothing a person can reproduce, so this cannot be turned down.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Error,
    mandatory: true,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimNondeterministic {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_NONDETERMINISTIC
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let mut out = Vec::new();
        for (_, s) in sims(view) {
            let Some(m) = s.run else { continue };
            if s.standing(view) == Some(Standing::Stale) {
                continue;
            }
            let Some(outcome) = m.sim() else { continue };
            for r in outcome.differing() {
                let case = match &r.case {
                    SimCase::Regress(n) => s
                        .harness
                        .cases()
                        .iter()
                        .find(|c| role_of(c.name().as_str()) == Some(CaseRole::Regress(n.clone())))
                        .unwrap_or(s.sweep),
                    SimCase::Seed(_) => s.sweep,
                };
                let what = match r.result {
                    Reproduction::ScheduleDiffers => {
                        "gave two schedules (the script draws something besides its seed)"
                    }
                    Reproduction::TraceDiffers => {
                        "gave two traces for one schedule (the program draws its own order)"
                    }
                    Reproduction::Same => continue,
                };
                out.push(Finding::new(
                    &SIM_NONDETERMINISTIC,
                    Some(Subject::Case(case.id())),
                    format!("{} {what}", r.case),
                ));
            }
        }
        out
    }
}

/// The latest simulation failed, or a case of it ended abnormally.
#[derive(Debug, Clone, Copy)]
pub struct SimError;

static SIM_ERROR: Descriptor = Descriptor {
    id: RuleId::from_static("sim.error"),
    summary: Cow::Borrowed("a simulation that failed, or whose script failed for some case"),
    help: Cow::Borrowed(
        "The run could not finish (the program does not build, the script could not start the program in any case, \
           the script broke the protocol, the time ran out), or the script itself failed for a case (an exit other \
           than 0 or 1, too slow, or the program could not be started in it). The evidence of the run is in its \
           record. Fix the cause and run the simulate measure again; a failed run is not judged for findings, and a \
           case that could not start the program found nothing and reached nothing.",
    ),
    hooks: Cow::Borrowed(&[Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimError {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_ERROR
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let mut out = Vec::new();
        for (_, s) in sims(view) {
            let Some(m) = s.run else { continue };
            let subject = Some(Subject::Case(s.sweep.id()));
            if let Some(failure) = m.failure() {
                out.push(Finding::new(
                    &SIM_ERROR,
                    subject,
                    format!("the latest simulation failed ({failure})"),
                ));
                continue;
            }
            if s.standing(view) == Some(Standing::Stale) {
                continue;
            }
            let Some(outcome) = m.sim() else { continue };
            let abnormal: Vec<String> = outcome
                .abnormal()
                .map(|r| format!("{} ended {}", r.case, r.exit.word()))
                .collect();
            if let Some(first) = abnormal.first() {
                let unstarted = outcome.cannot_start().count();
                let apart = if unstarted == 0 {
                    String::new()
                } else {
                    format!("; the program could not be started in {unstarted} of them")
                };
                out.push(Finding::new(
                    &SIM_ERROR,
                    subject,
                    format!(
                        "the script failed in {} case(s); first: {first}{apart}",
                        abnormal.len()
                    ),
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::fixture::*;
    use super::*;
    use crate::domain::entity::measurement::fixture::sim_draft;
    use crate::domain::service::sim::basis_of;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseText};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::harness_kind::HarnessKind;
    use crate::domain::vo::id::{CaseId, ContractId, HarnessId, test_uuid};
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::measure::{FailureKind, MeasureFailure};
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::sim::{
        CaseExit, CaseFinding, CaseRun, CaseUnreached, Rerun, SimOutcome,
    };
    use crate::domain::vo::source::Source;
    use crate::domain::vo::title::Title;

    fn hid() -> HarnessId {
        HarnessId::from_uuid(test_uuid(500))
    }

    fn sweep_id() -> CaseId {
        CaseId::from_uuid(test_uuid(50))
    }

    fn regress_id() -> CaseId {
        CaseId::from_uuid(test_uuid(51))
    }

    /// The citations of a Case of a sim: these Clauses and no Item.
    fn cites(clauses: &[u128]) -> CaseCites {
        CaseCites::of(
            BTreeMap::new(),
            clauses
                .iter()
                .map(|c| (clause(*c), Revision::first()))
                .collect(),
        )
    }

    /// Puts the sim Harness, which builds Interface 100: a sweep citing C1 and C2 and a regress case citing C2,
    /// with this script.
    fn put_sim(p: &mut Project, script: &str) {
        let at = |n| LineRange::new(n, n).unwrap();
        p.put_harness_as(
            ctx(),
            hid(),
            Some(HarnessKind::Sim),
            BTreeSet::from([iface()]),
            Source::new(format!("{script}\nx\n")),
            vec![
                (
                    sweep_id(),
                    CaseName::try_from("sweep").unwrap(),
                    at(1),
                    cites(&[30, 31]),
                ),
                (
                    regress_id(),
                    CaseName::try_from("regress-lost").unwrap(),
                    at(2),
                    cites(&[31]),
                ),
            ],
        )
        .unwrap();
    }

    fn sim_project() -> Project {
        let mut p = project();
        put_sim(&mut p, "script");
        p
    }

    fn run_of(case: SimCase) -> CaseRun {
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

    /// The run with a finding of the Clause with this id (30 is C1 and 31 is C2 of the Contract of the fixture).
    fn broke(mut r: CaseRun, step: u32, c: u128, text: &str) -> CaseRun {
        r.findings.push(CaseFinding {
            step,
            clause: clause(c),
            text: text.to_string(),
        });
        r
    }

    /// A simulation of the Harness as the Project is now.
    fn simulated(p: &Project, n: u128, outcome: SimOutcome) -> Measurement {
        Measurement::new(sim_draft(
            n,
            hid(),
            basis_of(p, hid(), &[]).unwrap(),
            outcome,
        ))
        .unwrap()
    }

    fn found(
        check: &dyn Check,
        p: &Project,
        runs: &[Measurement],
        settings: &BTreeMap<HarnessId, String>,
    ) -> Vec<Finding> {
        check.run(
            &CheckView {
                before: None,
                after: p,
                pins: &[],
                change_requests: &[],
                measurements: runs,
                measure_settings: &BTreeMap::new(),
                sim_settings: settings,
            },
            &Params::default(),
        )
    }

    fn none() -> BTreeMap<HarnessId, String> {
        BTreeMap::new()
    }

    #[test]
    fn a_context_without_a_sim_harness_has_nothing_for_these_rules_to_say() {
        let p = project();
        for check in [
            &SimShape as &dyn Check,
            &SimUncitedClause,
            &SimUnrun,
            &SimStale,
            &SimFinding,
            &SimUnreached,
            &SimNondeterministic,
            &SimError,
        ] {
            assert!(found(check, &p, &[], &none()).is_empty());
        }
    }

    #[test]
    fn a_malformed_sim_harness_is_a_shape_error_on_the_context_or_the_case() {
        let mut p = project();
        let at = LineRange::new(1, 1).unwrap();
        p.put_harness_as(
            ctx(),
            hid(),
            Some(HarnessKind::Sim),
            BTreeSet::from([iface()]),
            Source::new("x\n".into()),
            vec![(
                sweep_id(),
                CaseName::try_from("a_test").unwrap(),
                at,
                cites(&[30]),
            )],
        )
        .unwrap();
        let all = found(&SimShape, &p, &[], &none());
        let subjects: Vec<_> = all.iter().map(Finding::subject).collect();
        assert_eq!(
            subjects,
            [
                Some(Subject::Context(ctx())),
                Some(Subject::Case(sweep_id()))
            ]
        );
        assert!(all.iter().all(|f| f.level() == Level::Error));
        // and a well-formed one is quiet
        assert!(found(&SimShape, &sim_project(), &[], &none()).is_empty());
    }

    /// Adds a second Contract to `core`, whose Clauses 32 and 33 are C1 and C2 again.
    fn put_second_contract(p: &mut Project) {
        let c = |n: u128, l: &str, t: &str| {
            (
                clause(n),
                ClauseContent::new(
                    ClauseLabel::try_from(l).unwrap(),
                    ClauseText::try_from(t).unwrap(),
                ),
            )
        };
        p.put_contract(
            ctx(),
            ContractId::from_uuid(test_uuid(301)),
            Title::try_from("U").unwrap(),
            vec![c(32, "C1", "a line is whole"), c(33, "C2", "a flag is off")],
        )
        .unwrap();
    }

    /// Puts a sim whose sweep cites these Clauses, with the script `script`.
    fn put_sim_citing(p: &mut Project, script: &str, clauses: &[u128]) {
        p.put_harness_as(
            ctx(),
            hid(),
            Some(HarnessKind::Sim),
            BTreeSet::from([iface()]),
            Source::new(format!("{script}\n")),
            vec![(
                sweep_id(),
                CaseName::try_from("sweep").unwrap(),
                LineRange::new(1, 1).unwrap(),
                cites(clauses),
            )],
        )
        .unwrap();
    }

    #[test]
    fn a_sim_taking_on_two_contracts_reports_each_finding_on_the_clause_of_its_own_contract() {
        // both Contracts have a C1 (30 and 32): the sweep cites all four Clauses, which is a sound shape
        let mut p = project();
        put_second_contract(&mut p);
        put_sim_citing(&mut p, "script", &[30, 31, 32, 33]);
        assert!(found(&SimShape, &p, &[], &none()).is_empty());
        assert!(found(&SimUncitedClause, &p, &[], &none()).is_empty());
        let outcome = SimOutcome {
            cases: vec![
                run_of(SimCase::Seed(0)),
                broke(run_of(SimCase::Seed(1)), 4, 32, "the second contract's C1"),
            ],
            reruns: Vec::new(),
        };
        let run = simulated(&p, 1, outcome);
        let all = found(&SimFinding, &p, std::slice::from_ref(&run), &none());
        // the finding is on Clause 32, not on 30, though both are labelled C1
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(32))));
        assert!(all[0].message().starts_with("clause C1 broke in 1 case(s)"));
        // and the second Contract's C2 unreached lands on 33
        let missed = SimOutcome {
            cases: vec![CaseRun {
                unreached: vec![CaseUnreached {
                    clause: clause(33),
                    text: "no flag".into(),
                }],
                ..run_of(SimCase::Seed(0))
            }],
            reruns: Vec::new(),
        };
        let run = simulated(&p, 2, missed);
        let all = found(&SimUnreached, &p, std::slice::from_ref(&run), &none());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(33))));
    }

    #[test]
    fn uncited_clauses_are_reported_only_in_contracts_the_sim_cites_something_of() {
        let mut p = project();
        put_second_contract(&mut p);
        // the sim takes on the first Contract only: the second one's four-way silence is the tests' business
        put_sim_citing(&mut p, "script", &[30]);
        let all = found(&SimUncitedClause, &p, &[], &none());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(31))));
        // citing one Clause of the second takes the whole Contract on
        put_sim_citing(&mut p, "script", &[30, 31, 32]);
        let all = found(&SimUncitedClause, &p, &[], &none());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(33))));
        // no Clause cited at all: nothing is taken on
        put_sim_citing(&mut p, "script", &[]);
        assert!(found(&SimUncitedClause, &p, &[], &none()).is_empty());
    }

    #[test]
    fn a_simulation_is_stale_when_a_contract_it_takes_on_or_an_interface_it_builds_moves_not_for_one_it_does_not()
     {
        let mut p = project();
        put_second_contract(&mut p);
        put_sim_citing(&mut p, "script", &[30, 32]);
        let run = simulated(&p, 1, SimOutcome::default());
        let runs = std::slice::from_ref(&run);
        assert!(found(&SimStale, &p, runs, &none()).is_empty());
        // the second Contract changes: the sim cites a Clause of it
        let mut q = p.clone();
        q.put_contract(
            ctx(),
            ContractId::from_uuid(test_uuid(301)),
            Title::try_from("U").unwrap(),
            vec![(
                clause(32),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("a line is whole, still").unwrap(),
                ),
            )],
        )
        .unwrap();
        assert_eq!(found(&SimStale, &q, runs, &none()).len(), 1);
        // a Contract it does not cite a Clause of moves: the simulation stands
        let mut r = p.clone();
        r.put_contract(
            ctx(),
            ContractId::from_uuid(test_uuid(302)),
            Title::try_from("V").unwrap(),
            vec![(
                clause(34),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("unrelated").unwrap(),
                ),
            )],
        )
        .unwrap();
        assert!(found(&SimStale, &r, runs, &none()).is_empty());
        // the Pin of the built Interface: pinning it (tests check it) moves the sim, as the code it ran is new
        let pins = [pin(&p)];
        let view = CheckView {
            before: None,
            after: &p,
            pins: &pins,
            change_requests: &[],
            measurements: runs,
            measure_settings: &BTreeMap::new(),
            sim_settings: &none(),
        };
        assert_eq!(SimStale.run(&view, &Params::default()).len(), 1);
    }

    #[test]
    fn a_clause_no_sim_case_cites_is_found_only_where_there_is_a_sim_harness() {
        let mut p = project();
        // the Context `core` has Clauses 30 and 31 and a Harness of tests citing 30; `empty` has none
        assert!(found(&SimUncitedClause, &p, &[], &none()).is_empty());
        let at = LineRange::new(1, 1).unwrap();
        p.put_harness_as(
            ctx(),
            hid(),
            Some(HarnessKind::Sim),
            BTreeSet::from([iface()]),
            Source::new("x\n".into()),
            vec![(
                sweep_id(),
                CaseName::try_from("sweep").unwrap(),
                at,
                cites(&[30]),
            )],
        )
        .unwrap();
        let all = found(&SimUncitedClause, &p, &[], &none());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(31))));
        assert_eq!(all[0].level(), Level::Warning);
        assert!(all[0].message().contains("C2"));
        assert!(found(&SimUncitedClause, &sim_project(), &[], &none()).is_empty());
    }

    #[test]
    fn a_sim_harness_is_unrun_until_a_simulation_exists_and_is_stale_when_the_model_moves() {
        let mut p = sim_project();
        let unrun = found(&SimUnrun, &p, &[], &none());
        assert_eq!(unrun.len(), 1);
        assert_eq!(unrun[0].subject(), Some(Subject::Case(sweep_id())));
        assert_eq!(unrun[0].level(), Level::Warning);
        let run = simulated(&p, 1, SimOutcome::default());
        let runs = std::slice::from_ref(&run);
        assert!(found(&SimUnrun, &p, runs, &none()).is_empty());
        assert!(found(&SimStale, &p, runs, &none()).is_empty());

        // the script moves
        put_sim(&mut p, "script, changed");
        let stale = found(&SimStale, &p, runs, &none());
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].subject(), Some(Subject::Case(sweep_id())));
        assert_eq!(stale[0].level(), Level::Warning);

        // a cited Contract moves
        let mut q = sim_project();
        put_contract(&mut q, "a half rounds to even");
        assert_eq!(found(&SimStale, &q, runs, &none()).len(), 1);
        // a cited Interface moves
        let mut r = sim_project();
        put_interface(&mut r, "fn a(x: u8)");
        assert_eq!(found(&SimStale, &r, runs, &none()).len(), 1);
    }

    #[test]
    fn a_simulation_is_stale_when_the_settings_differ_or_the_pinned_code_is_not_the_one_it_ran() {
        let p = sim_project();
        let run = simulated(&p, 1, SimOutcome::default());
        let runs = std::slice::from_ref(&run);
        let same = BTreeMap::from([(hid(), "settings".to_string())]);
        let other = BTreeMap::from([(hid(), "settings-2".to_string())]);
        assert!(found(&SimStale, &p, runs, &same).is_empty());
        assert!(found(&SimStale, &p, runs, &none()).is_empty());
        assert_eq!(found(&SimStale, &p, runs, &other).len(), 1);
        // the interface is pinned (to other code than the run saw, which saw none)
        let pins = [pin(&p)];
        let with_pin = SimStale.run(
            &CheckView {
                before: None,
                after: &p,
                pins: &pins,
                change_requests: &[],
                measurements: runs,
                measure_settings: &BTreeMap::new(),
                sim_settings: &same,
            },
            &Params::default(),
        );
        assert_eq!(with_pin.len(), 1);
    }

    #[test]
    fn each_clause_with_findings_is_one_error_naming_the_first_case_and_step() {
        let p = sim_project();
        let outcome = SimOutcome {
            cases: vec![
                run_of(SimCase::Seed(0)),
                broke(
                    broke(run_of(SimCase::Seed(1)), 7, 31, "capacity 0 accepted"),
                    9,
                    31,
                    "again",
                ),
                broke(run_of(SimCase::Seed(2)), 3, 31, "later"),
                broke(run_of(SimCase::Seed(3)), 5, 30, "other clause"),
            ],
            reruns: Vec::new(),
        };
        let run = simulated(&p, 1, outcome);
        let runs = std::slice::from_ref(&run);
        let all = found(&SimFinding, &p, runs, &none());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(30))));
        assert_eq!(all[1].subject(), Some(Subject::Clause(clause(31))));
        assert!(all.iter().all(|f| f.level() == Level::Error));
        assert_eq!(
            all[1].message(),
            "clause C2 broke in 2 case(s); first seed-1 step 7: capacity 0 accepted"
        );
        // judged after staleness: a moved script reports no finding of the old run
        let mut moved = p.clone();
        put_sim(&mut moved, "changed");
        assert!(found(&SimFinding, &moved, runs, &none()).is_empty());
    }

    #[test]
    fn a_clause_every_seed_missed_is_unreached() {
        let p = sim_project();
        let miss = |mut r: CaseRun| {
            r.unreached.push(CaseUnreached {
                clause: clause(31),
                text: "capacity below 1".into(),
            });
            r
        };
        let outcome = SimOutcome {
            cases: vec![
                miss(run_of(SimCase::Seed(0))),
                miss(run_of(SimCase::Seed(1))),
            ],
            reruns: Vec::new(),
        };
        let run = simulated(&p, 1, outcome);
        let all = found(&SimUnreached, &p, std::slice::from_ref(&run), &none());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(31))));
        assert_eq!(all[0].level(), Level::Error);
        assert!(all[0].message().contains("capacity below 1"));
        // one seed that reached it clears it
        let outcome = SimOutcome {
            cases: vec![miss(run_of(SimCase::Seed(0))), run_of(SimCase::Seed(1))],
            reruns: Vec::new(),
        };
        let run = simulated(&p, 2, outcome);
        assert!(found(&SimUnreached, &p, std::slice::from_ref(&run), &none()).is_empty());
    }

    #[test]
    fn a_rerun_that_differs_is_a_mandatory_error_on_the_case_and_says_whose_fault() {
        assert!(SimNondeterministic.descriptor().mandatory);
        let p = sim_project();
        let regress = SimCase::try_from("regress-lost").unwrap();
        let outcome = SimOutcome {
            cases: vec![run_of(SimCase::Seed(0)), run_of(regress.clone())],
            reruns: vec![
                Rerun {
                    case: SimCase::Seed(0),
                    result: Reproduction::ScheduleDiffers,
                },
                Rerun {
                    case: regress,
                    result: Reproduction::TraceDiffers,
                },
                Rerun {
                    case: SimCase::Seed(4),
                    result: Reproduction::Same,
                },
            ],
        };
        let run = simulated(&p, 1, outcome);
        let all = found(
            &SimNondeterministic,
            &p,
            std::slice::from_ref(&run),
            &none(),
        );
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].subject(), Some(Subject::Case(sweep_id())));
        assert!(all[0].message().starts_with("seed-0 gave two schedules"));
        assert_eq!(all[1].subject(), Some(Subject::Case(regress_id())));
        assert!(all[1].message().starts_with("regress-lost gave two traces"));
        assert!(all.iter().all(|f| f.level() == Level::Error));
        // not judged for a run that stood on an older model
        let mut moved = p.clone();
        put_sim(&mut moved, "changed");
        assert!(
            found(
                &SimNondeterministic,
                &moved,
                std::slice::from_ref(&run),
                &none()
            )
            .is_empty()
        );
    }

    #[test]
    fn a_failed_run_or_a_script_that_failed_is_a_warning_and_a_failed_run_is_judged_by_nothing_else()
     {
        let p = sim_project();
        let mut failed = sim_draft(
            2,
            hid(),
            basis_of(&p, hid(), &[]).unwrap(),
            SimOutcome {
                cases: vec![broke(run_of(SimCase::Seed(0)), 1, 30, "x")],
                reruns: Vec::new(),
            },
        );
        failed.failure =
            Some(MeasureFailure::new(FailureKind::NoBuild, "the program does not build").unwrap());
        let run = Measurement::new(failed).unwrap();
        let runs = std::slice::from_ref(&run);
        let all = found(&SimError, &p, runs, &none());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].level(), Level::Warning);
        assert!(
            all[0]
                .message()
                .contains("no_build: the program does not build")
        );
        assert!(found(&SimFinding, &p, runs, &none()).is_empty());
        assert!(found(&SimStale, &p, runs, &none()).is_empty());
        assert!(found(&SimUnrun, &p, runs, &none()).is_empty());

        // a script that crashed for a case
        let crashed = simulated(
            &p,
            3,
            SimOutcome {
                cases: vec![
                    run_of(SimCase::Seed(0)),
                    CaseRun {
                        exit: CaseExit::Script(2),
                        ..run_of(SimCase::Seed(1))
                    },
                    CaseRun {
                        exit: CaseExit::Timeout,
                        ..run_of(SimCase::Seed(2))
                    },
                ],
                reruns: Vec::new(),
            },
        );
        let all = found(&SimError, &p, std::slice::from_ref(&crashed), &none());
        assert_eq!(all.len(), 1);
        assert_eq!(
            all[0].message(),
            "the script failed in 2 case(s); first: seed-1 ended script:2"
        );
        let clean = simulated(&p, 4, SimOutcome::default());
        assert!(found(&SimError, &p, &[clean], &none()).is_empty());
    }

    fn unstarted(case: SimCase) -> CaseRun {
        CaseRun {
            exit: CaseExit::CannotStart("python3: No such file or directory".to_string()),
            schedule: None,
            trace: None,
            ..run_of(case)
        }
    }

    #[test]
    fn a_case_that_could_not_start_the_program_is_a_script_error_and_gives_no_finding() {
        let p = sim_project();
        let run = simulated(
            &p,
            5,
            SimOutcome {
                cases: vec![
                    unstarted(SimCase::Seed(0)),
                    broke(run_of(SimCase::Seed(1)), 2, 30, "broke"),
                    unstarted(SimCase::Seed(2)),
                ],
                reruns: Vec::new(),
            },
        );
        let runs = std::slice::from_ref(&run);
        let errors = found(&SimError, &p, runs, &none());
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message(),
            "the script failed in 2 case(s); first: seed-0 ended \
             cannot_start:python3: No such file or directory; \
             the program could not be started in 2 of them"
        );
        // the one case that drove the program is the only finding; the two that did not add none
        let findings = found(&SimFinding, &p, runs, &none());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message().contains("broke in 1 case(s)"));
    }

    #[test]
    fn a_run_in_which_no_case_started_the_program_fails_and_only_sim_error_speaks() {
        let p = sim_project();
        let mut failed = sim_draft(
            6,
            hid(),
            basis_of(&p, hid(), &[]).unwrap(),
            SimOutcome {
                cases: vec![
                    unstarted(SimCase::Seed(0)),
                    unstarted(SimCase::Seed(1)),
                    unstarted(SimCase::Regress(
                        crate::domain::vo::sim::RegressName::try_from("lost").unwrap(),
                    )),
                ],
                reruns: Vec::new(),
            },
        );
        failed.failure = Some(MeasureFailure::of(
            FailureKind::NoStart,
            "the script could not start the program in any of the 3 case(s)",
        ));
        let run = Measurement::new(failed).unwrap();
        let runs = std::slice::from_ref(&run);
        let errors = found(&SimError, &p, runs, &none());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].level(), Level::Warning);
        assert!(
            errors[0]
                .message()
                .contains("no_start: the script could not start")
        );
        assert!(found(&SimFinding, &p, runs, &none()).is_empty());
        assert!(found(&SimUnreached, &p, runs, &none()).is_empty());
        assert!(found(&SimNondeterministic, &p, runs, &none()).is_empty());
        assert!(found(&SimStale, &p, runs, &none()).is_empty());
    }

    #[test]
    fn only_the_latest_simulation_of_a_harness_counts() {
        let p = sim_project();
        let old = simulated(
            &p,
            1,
            SimOutcome {
                cases: vec![broke(run_of(SimCase::Seed(0)), 1, 30, "x")],
                reruns: Vec::new(),
            },
        );
        let newer = simulated(&p, 2, SimOutcome::default());
        assert!(found(&SimFinding, &p, &[old, newer], &none()).is_empty());
    }
}
