//! Checks on the simulations: whether a sim Harness is kept in a shape the protocol does not run, whether every
//! Clause of the Contracts it takes on is named by it, and what the latest simulation found. They read what a run left in the store and run
//! nothing: a simulation takes as long as its program, so `measure` runs it and these only judge its record.
//!
//! A simulation that no longer describes the model (the script, a Contract or an Interface it cites, or the code
//! pinned for it moved) is judged stale before anything else, so a finding of an old run is never reported for a
//! program that has since changed. One thing is judged even then: a second run that did not reproduce the first
//! says the script or the program is not deterministic, and no later change makes that untrue of that run.

use std::borrow::Cow;
use std::collections::BTreeSet;

use super::CONFIRM_MOMENTS;
use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Level, Moment};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::context::Context;
use crate::domain::entity::harness::{Case, Harness, SimShapeProblem};
use crate::domain::entity::measurement::SimulationRun;
use crate::domain::entity::project::Project;
use crate::domain::rule::rule;
use crate::domain::service::measure::{Standing, latest_sim, sim_standing};
use crate::domain::service::sim::{sim_harnesses, sweep};
use crate::domain::vo::clause::ClauseLabel;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::id::ClauseId;
use crate::domain::vo::measure::SettingsNow;
use crate::domain::vo::sim::{CaseRole, Reproduction, SimCase, role_of};

/// A sim Harness with its sweep Case and the latest simulation of it (if any).
struct Sim<'a> {
    harness: &'a Harness,
    sweep: &'a Case,
    run: Option<SimulationRun<'a>>,
}

impl Sim<'_> {
    /// What the settings of a simulation of the Harness are now.
    fn settings(&self, view: &CheckView<'_>) -> SettingsNow {
        view.sim_settings_now(self.harness.id())
    }

    /// Whether there is a run, and it no longer describes the model. A run whose settings cannot be compared is
    /// stale.
    fn is_stale(&self, view: &CheckView<'_>) -> bool {
        self.run.is_some_and(|run| {
            sim_standing(run, view.project, view.pins, &self.settings(view)).is_stale()
        })
    }
}

/// The sim Harnesses of the Project that have a sweep Case (one without is a fault of shape, found by
/// `sim.shape-kept`), each with its latest simulation.
fn sims<'a>(view: &'a CheckView<'a>) -> impl Iterator<Item = (&'a Context, Sim<'a>)> + 'a {
    view.project.contexts().flat_map(move |c| {
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
        .map(|c| c.label())
}

/// A sim Harness kept in a shape the protocol does not run: the store has it from before a put was held to the
/// shape (the `sim.*` rules), or it stands beside one that is.
#[derive(Debug, Clone, Copy)]
pub struct SimShapeKept;

static SIM_SHAPE_KEPT: Descriptor = Descriptor {
    id: RuleId::from_static("sim.shape-kept"),
    summary: Cow::Borrowed(
        "a sim harness kept in a shape a put of it is refused for (one the store held before the rule, or one beside another sim harness of its context), about the case, the clause or the context; it refuses no change, and one label in two contracts of the sweep is fine",
    ),
    help: Cow::Borrowed(
        "A put of a sim harness that breaks one of the `sim.*` invariants is refused; the finding names the one it \
           breaks, and `explain` gives its help. This one was kept before that was so, and a change that does not \
           put it goes through. It cannot be simulated. Put it again in the shape the invariant asks for.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
    default_level: Level::Error,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for SimShapeKept {
    fn descriptor(&self) -> &'static Descriptor {
        &SIM_SHAPE_KEPT
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let mut out = Vec::new();
        // A problem of a Context comes with each of its sim Harnesses, and a Clause foreign to a Context may be
        // cited by several Cases: each is reported once.
        let mut foreign: BTreeSet<ClauseId> = BTreeSet::new();
        for context in view.project.contexts() {
            let mut seen: Vec<SimShapeProblem> = Vec::new();
            for harness in sim_harnesses(context) {
                for problem in context.sim_problems(harness) {
                    if seen.contains(&problem) {
                        continue;
                    }
                    let subject = match &problem {
                        SimShapeProblem::OnePerContext { .. }
                        | SimShapeProblem::SweepOnce { extra: None, .. }
                        | SimShapeProblem::BuildsNone { .. } => Subject::Context(context.id()),
                        SimShapeProblem::SweepOnce {
                            extra: Some(case), ..
                        }
                        | SimShapeProblem::CaseName { case, .. }
                        | SimShapeProblem::CaseCitesNone { case, .. } => Subject::Case(*case),
                        SimShapeProblem::ClauseForeign { clause, .. } => {
                            if !foreign.insert(*clause) {
                                continue;
                            }
                            Subject::Clause(*clause)
                        }
                    };
                    // The words are the invariant's own: the rule the kept shape breaks, by id and summary.
                    let violation = problem.violation();
                    let broken = violation.rule();
                    let message = match rule(broken.as_str()) {
                        Some(r) => format!("kept against `{broken}`: {}", r.summary),
                        None => format!("kept against `{broken}`"),
                    };
                    seen.push(problem);
                    out.push(Finding::new(&SIM_SHAPE_KEPT, Some(subject), message));
                }
            }
        }
        out
    }
}

/// A Clause of a Contract that a sim Harness takes on (cites a Clause of), which no sim Case cites.
#[derive(Debug, Clone, Copy)]
pub struct SimUncitedClause;

static SIM_UNCITED_CLAUSE: Descriptor = Descriptor {
    id: RuleId::from_static("sim.uncited-clause"),
    summary: Cow::Borrowed(
        "a clause that no sim case cites, of a contract a sim harness takes on (cites at least one clause of); a contract the sim cites nothing of is left to the tests",
    ),
    help: Cow::Borrowed(
        "The simulation takes this contract on (the sweep cites a clause of it) but never checks this clause. \
           Have the script check it and cite it in the sweep case, or open a change request that addresses the \
           finding if the clause is not for a simulation to check. A contract the sweep cites no clause of is \
           left to the tests, and is not reported.",
    ),
    moments: Cow::Borrowed(CONFIRM_MOMENTS),
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
        for c in view.project.contexts() {
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
                            format!("no sim case cites clause {}", clause.label()),
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
    moments: Cow::Borrowed(&[Moment::Check]),
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
    summary: Cow::Borrowed(
        "a simulation after which the script, a case or the builds of the harness, a contract it cites a clause of, a built interface, the code pinned for it, or the settings moved",
    ),
    help: Cow::Borrowed(
        "The script, the cases or `builds` of the harness, a contract it cites a clause of, a built interface, \
           the code pinned for the program, or the measure \
           settings changed after the run. Its result says nothing of the program as it is now. Run the simulate \
           measure again.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
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
            .filter_map(|(_, s)| {
                let run = s.run?;
                if run.measurement().failure().is_some() {
                    return None;
                }
                let settings = s.settings(view);
                let Standing::Stale(moved) = sim_standing(run, view.project, view.pins, &settings)
                else {
                    return None;
                };
                // Every part that moved is said; settings that cannot be compared say why, for the cure is not to
                // run again but to configure `[measure]` or to make its settings readable.
                Some(Finding::new(
                    &SIM_STALE,
                    Some(Subject::Case(s.sweep.id())),
                    format!("the latest simulation is stale: {}", moved.said()),
                ))
            })
            .collect()
    }
}

/// A Clause the program broke in some case of the latest simulation.
#[derive(Debug, Clone, Copy)]
pub struct SimFinding;

static SIM_FINDING: Descriptor = Descriptor {
    id: RuleId::from_static("sim.finding"),
    summary: Cow::Borrowed(
        "a clause the program broke in a simulated case, one per clause, with the first case and step",
    ),
    help: Cow::Borrowed(
        "The script drove the program through a schedule and saw a promise of this clause not hold. Either the \
           program is wrong (change it and pin it again), or the clause and the script read it differently (a \
           change request on the clause or the script). The case and step are in the message; run the case again \
           to see its schedule and trace.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
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
            let Some(run) = s.run else { continue };
            let m = run.measurement();
            if m.failure().is_some() || s.is_stale(view) {
                continue;
            }
            let Some(outcome) = m.sim() else { continue };
            for f in outcome.found() {
                // The record holds the Clause's id; a Clause the model has not any more is not reported.
                let Some(label) = label_of(view.project, f.clause) else {
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
    summary: Cow::Borrowed("a clause whose situation no simulated case reached, one per clause"),
    help: Cow::Borrowed(
        "The script cites this clause but no seed drove the program to the situation the clause needs, so the \
           clause was not checked. Change the schedule the script makes (or add a regress case for the \
           situation) so that the situation comes up.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
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
            let Some(run) = s.run else { continue };
            let m = run.measurement();
            if m.failure().is_some() || s.is_stale(view) {
                continue;
            }
            let Some(outcome) = m.sim() else { continue };
            for u in outcome.unreached() {
                let Some(label) = label_of(view.project, u.clause) else {
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
    summary: Cow::Borrowed(
        "a simulated case that did not give the same schedule and trace twice, about the case; no policy lowers it",
    ),
    help: Cow::Borrowed(
        "A case was run again in a fresh copy. A different schedule means the script draws something besides its \
           seed (a clock, the order of a set, an unseeded random number): fix the script. The same schedule with a \
           different trace means the program draws its own order: fix the program. A simulation that cannot be \
           repeated finds nothing a person can reproduce, so this cannot be turned down.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
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
            let Some(run) = s.run else { continue };
            if s.is_stale(view) {
                continue;
            }
            let Some(outcome) = run.measurement().sim() else {
                continue;
            };
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
    summary: Cow::Borrowed(
        "a simulation whose run failed (no_build, no_start, protocol, budget, ...), or whose script failed for a case or could not start the program in it (CANNOT_START)",
    ),
    help: Cow::Borrowed(
        "The run could not finish (the program does not build, the script could not start the program in any case, \
           the script broke the protocol, the time ran out), or the script itself failed for a case (an exit other \
           than 0 or 1, too slow, or the program could not be started in it). The evidence of the run is in its \
           record. Fix the cause and run the simulate measure again; a failed run is not judged for findings, and a \
           case that could not start the program found nothing and reached nothing.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
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
            let Some(run) = s.run else { continue };
            let m = run.measurement();
            let subject = Some(Subject::Case(s.sweep.id()));
            if let Some(failure) = m.failure() {
                out.push(Finding::new(
                    &SIM_ERROR,
                    subject,
                    format!("the latest simulation failed ({failure})"),
                ));
                continue;
            }
            if s.is_stale(view) {
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
    use crate::domain::entity::measurement::Measurement;
    use crate::domain::entity::measurement::fixture::{same_settings, settings_of, sim_draft};
    use crate::domain::service::sim::basis_of;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseText};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::harness_kind::CodeKind;
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
            Some(CodeKind::Sim),
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
        settings: &BTreeMap<HarnessId, SettingsNow>,
    ) -> Vec<Finding> {
        check.run(
            &CheckView {
                project: p,
                pins: &[],
                change_requests: &[],
                cruxes: &[],
                measurements: runs,
                measure_settings: &BTreeMap::new(),
                sim_settings: settings,
                resolving: None,
            },
            &Params::default(),
        )
    }

    /// The settings of the sim Harness as they are now: `now`.
    fn settings_now(now: SettingsNow) -> BTreeMap<HarnessId, SettingsNow> {
        BTreeMap::from([(hid(), now)])
    }

    /// The settings of the sim Harness as they are now: the ones its runs were made with.
    fn unchanged() -> BTreeMap<HarnessId, SettingsNow> {
        settings_now(same_settings())
    }

    #[test]
    fn a_context_without_a_sim_harness_has_nothing_for_these_rules_to_say() {
        let p = project();
        for check in [
            &SimShapeKept as &dyn Check,
            &SimUncitedClause,
            &SimUnrun,
            &SimStale,
            &SimFinding,
            &SimUnreached,
            &SimNondeterministic,
            &SimError,
        ] {
            assert!(found(check, &p, &[], &unchanged()).is_empty());
        }
    }

    /// A sim Harness `id` that builds Interface 100, with these Cases `(id, name, Clauses cited)`, as a store kept
    /// it: whatever its shape, for it was not put.
    fn kept_sim(id: u128, cases: &[(u128, &str, &[u128])]) -> Harness {
        Harness::of_kind(
            CodeKind::Sim,
            HarnessId::from_uuid(test_uuid(id)),
            BTreeSet::from([iface()]),
            Source::new("x\n".into()),
            cases
                .iter()
                .map(|(n, name, clauses)| {
                    (
                        CaseId::from_uuid(test_uuid(*n)),
                        CaseName::try_from(*name).unwrap(),
                        LineRange::new(1, 1).unwrap(),
                        cites(clauses),
                    )
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn a_sim_harness_kept_malformed_is_a_shape_error_on_the_context_or_the_case() {
        let p = project().with_harnesses_kept(ctx(), vec![kept_sim(500, &[(50, "a_test", &[30])])]);
        let all = found(&SimShapeKept, &p, &[], &unchanged());
        let subjects: Vec<_> = all.iter().map(Finding::subject).collect();
        assert_eq!(
            subjects,
            [
                Some(Subject::Context(ctx())),
                Some(Subject::Case(sweep_id()))
            ]
        );
        assert!(all.iter().all(|f| f.level() == Level::Error));
        // the words are the broken invariant's, by id and summary
        assert_eq!(
            all.iter().map(Finding::message).collect::<Vec<_>>(),
            [
                format!(
                    "kept against `sim.sweep-once`: {}",
                    rule("sim.sweep-once").unwrap().summary
                ),
                format!(
                    "kept against `sim.case-name`: {}",
                    rule("sim.case-name").unwrap().summary
                ),
            ]
        );
        // and a well-formed one is quiet
        assert!(found(&SimShapeKept, &sim_project(), &[], &unchanged()).is_empty());
    }

    #[test]
    fn two_sim_harnesses_kept_in_a_context_are_one_finding_on_the_context() {
        let p = project().with_harnesses_kept(
            ctx(),
            vec![
                kept_sim(500, &[(50, "sweep", &[30])]),
                kept_sim(501, &[(60, "sweep", &[31])]),
            ],
        );
        let all = found(&SimShapeKept, &p, &[], &unchanged());
        assert_eq!(
            all.iter().map(Finding::subject).collect::<Vec<_>>(),
            [Some(Subject::Context(ctx()))]
        );
    }

    #[test]
    fn a_foreign_clause_is_a_finding_on_the_clause_once_however_many_cases_cite_it() {
        // Clause 90 is in a Contract of `empty`, which `core` uses: it is reached but is in no Contract of `core`.
        // Two Cases cite it, and the sweep cites a Clause of its own too
        let mut p = project();
        p.put_contract(
            empty_ctx(),
            ContractId::from_uuid(test_uuid(310)),
            Title::try_from("B").unwrap(),
            vec![(
                clause(90),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("a base promise").unwrap(),
                ),
            )],
        )
        .unwrap();
        p.set_uses(ctx(), BTreeSet::from([empty_ctx()])).unwrap();
        let p = p.with_harnesses_kept(
            ctx(),
            vec![kept_sim(
                500,
                &[(50, "sweep", &[30, 90]), (51, "regress-a", &[90])],
            )],
        );
        let all = found(&SimShapeKept, &p, &[], &unchanged());
        assert_eq!(
            all.iter().map(Finding::subject).collect::<Vec<_>>(),
            [Some(Subject::Clause(clause(90)))]
        );
        // the message says no id, for the subject is named by the application
        let message = all[0].message();
        assert!(!message.contains(&clause(90).to_string()), "{message}");
    }

    #[test]
    fn the_findings_of_a_kept_shape_are_not_a_moment_of_an_edit() {
        // a put is refused by the rules (`sim.*`), so the check runs on demand only
        assert_eq!(SimShapeKept.descriptor().moments.as_ref(), &[Moment::Check]);
        assert!(!SimShapeKept.descriptor().mandatory);
        assert_eq!(SimShapeKept.descriptor().default_level, Level::Error);
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
            Some(CodeKind::Sim),
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
        assert!(found(&SimShapeKept, &p, &[], &unchanged()).is_empty());
        assert!(found(&SimUncitedClause, &p, &[], &unchanged()).is_empty());
        let outcome = SimOutcome {
            cases: vec![
                run_of(SimCase::Seed(0)),
                broke(run_of(SimCase::Seed(1)), 4, 32, "the second contract's C1"),
            ],
            reruns: Vec::new(),
        };
        let run = simulated(&p, 1, outcome);
        let all = found(&SimFinding, &p, std::slice::from_ref(&run), &unchanged());
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
        let all = found(&SimUnreached, &p, std::slice::from_ref(&run), &unchanged());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(33))));
    }

    #[test]
    fn uncited_clauses_are_reported_only_in_contracts_the_sim_cites_something_of() {
        let mut p = project();
        put_second_contract(&mut p);
        // the sim takes on the first Contract only: the second one's four-way silence is the tests' business
        put_sim_citing(&mut p, "script", &[30]);
        let all = found(&SimUncitedClause, &p, &[], &unchanged());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(31))));
        // citing one Clause of the second takes the whole Contract on
        put_sim_citing(&mut p, "script", &[30, 31, 32]);
        let all = found(&SimUncitedClause, &p, &[], &unchanged());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(33))));
        // no Clause cited at all (a shape a put is refused for, kept by a store): nothing is taken on
        let kept = p.with_harnesses_kept(ctx(), vec![kept_sim(500, &[(50, "sweep", &[])])]);
        assert!(found(&SimUncitedClause, &kept, &[], &unchanged()).is_empty());
    }

    #[test]
    fn a_simulation_is_stale_when_a_contract_it_takes_on_or_an_interface_it_builds_moves_not_for_one_it_does_not()
     {
        let mut p = project();
        put_second_contract(&mut p);
        put_sim_citing(&mut p, "script", &[30, 32]);
        let run = simulated(&p, 1, SimOutcome::default());
        let runs = std::slice::from_ref(&run);
        assert!(found(&SimStale, &p, runs, &unchanged()).is_empty());
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
        assert_eq!(found(&SimStale, &q, runs, &unchanged()).len(), 1);
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
        assert!(found(&SimStale, &r, runs, &unchanged()).is_empty());
        // the Pin of the built Interface: pinning it (tests check it) moves the sim, as the code it ran is new
        let pins = [pin(&p)];
        let view = CheckView {
            project: &p,
            pins: &pins,
            change_requests: &[],
            cruxes: &[],
            measurements: runs,
            measure_settings: &BTreeMap::new(),
            sim_settings: &unchanged(),
            resolving: None,
        };
        assert_eq!(SimStale.run(&view, &Params::default()).len(), 1);
    }

    #[test]
    fn a_clause_no_sim_case_cites_is_found_only_where_there_is_a_sim_harness() {
        let mut p = project();
        // the Context `core` has Clauses 30 and 31 and a Harness of tests citing 30; `empty` has none
        assert!(found(&SimUncitedClause, &p, &[], &unchanged()).is_empty());
        let at = LineRange::new(1, 1).unwrap();
        p.put_harness_as(
            ctx(),
            hid(),
            Some(CodeKind::Sim),
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
        let all = found(&SimUncitedClause, &p, &[], &unchanged());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].subject(), Some(Subject::Clause(clause(31))));
        assert_eq!(all[0].level(), Level::Warning);
        assert!(all[0].message().contains("C2"));
        assert!(found(&SimUncitedClause, &sim_project(), &[], &unchanged()).is_empty());
    }

    #[test]
    fn a_sim_harness_is_unrun_until_a_simulation_exists_and_is_stale_when_the_model_moves() {
        let mut p = sim_project();
        let unrun = found(&SimUnrun, &p, &[], &unchanged());
        assert_eq!(unrun.len(), 1);
        assert_eq!(unrun[0].subject(), Some(Subject::Case(sweep_id())));
        assert_eq!(unrun[0].level(), Level::Warning);
        let run = simulated(&p, 1, SimOutcome::default());
        let runs = std::slice::from_ref(&run);
        assert!(found(&SimUnrun, &p, runs, &unchanged()).is_empty());
        assert!(found(&SimStale, &p, runs, &unchanged()).is_empty());

        // the script moves
        put_sim(&mut p, "script, changed");
        let stale = found(&SimStale, &p, runs, &unchanged());
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].subject(), Some(Subject::Case(sweep_id())));
        assert_eq!(stale[0].level(), Level::Warning);
        assert_eq!(stale[0].message(), "the latest simulation is stale: script");

        // a cited Contract moves
        let mut q = sim_project();
        put_contract(&mut q, "a half rounds to even");
        let stale = found(&SimStale, &q, runs, &unchanged());
        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0].message(),
            "the latest simulation is stale: contract"
        );
        // a cited Interface moves
        let mut r = sim_project();
        put_interface(&mut r, "fn a(x: u8)");
        let stale = found(&SimStale, &r, runs, &unchanged());
        assert_eq!(stale.len(), 1);
        assert_eq!(
            stale[0].message(),
            "the latest simulation is stale: built-revision"
        );
    }

    #[test]
    fn a_simulation_is_stale_when_the_settings_differ_or_the_pinned_code_is_not_the_one_it_ran() {
        let p = sim_project();
        let run = simulated(&p, 1, SimOutcome::default());
        let runs = std::slice::from_ref(&run);
        let same = unchanged();
        let other = settings_now(SettingsNow::Known(settings_of("settings-2")));
        assert!(found(&SimStale, &p, runs, &same).is_empty());
        let by_settings = found(&SimStale, &p, runs, &other);
        assert_eq!(by_settings.len(), 1);
        assert_eq!(
            by_settings[0].message(),
            "the latest simulation is stale: settings"
        );
        // the interface is pinned (to other code than the run saw, which saw none)
        let pins = [pin(&p)];
        let with_pin = SimStale.run(
            &CheckView {
                project: &p,
                pins: &pins,
                change_requests: &[],
                cruxes: &[],
                measurements: runs,
                measure_settings: &BTreeMap::new(),
                sim_settings: &same,
                resolving: None,
            },
            &Params::default(),
        );
        assert_eq!(with_pin.len(), 1);
        assert_eq!(
            with_pin[0].message(),
            "the latest simulation is stale: built-code"
        );
    }

    #[test]
    fn settings_that_cannot_be_read_make_a_simulation_stale_and_each_says_which_it_is() {
        let p = sim_project();
        let outcome = SimOutcome {
            cases: vec![broke(run_of(SimCase::Seed(0)), 2, 31, "x")],
            reruns: Vec::new(),
        };
        let run = simulated(&p, 1, outcome);
        let runs = std::slice::from_ref(&run);
        for (now, said) in [
            (SettingsNow::Unconfigured, "measure is not configured"),
            (
                SettingsNow::Unreadable("no layout for the script".to_string()),
                "the settings cannot be read: no layout for the script",
            ),
        ] {
            let now = settings_now(now);
            let stale = found(&SimStale, &p, runs, &now);
            assert_eq!(stale.len(), 1, "{said}");
            assert_eq!(
                stale[0].message(),
                format!("the latest simulation is stale: settings ({said})")
            );
            // a stale run is not judged: its finding is not reported
            assert!(found(&SimFinding, &p, runs, &now).is_empty());
        }
        // what else moved is said beside the settings: the script, and no `[measure]`
        let mut changed = p.clone();
        put_sim(&mut changed, "changed");
        let both = found(
            &SimStale,
            &changed,
            runs,
            &settings_now(SettingsNow::Unconfigured),
        );
        assert_eq!(both.len(), 1);
        assert_eq!(
            both[0].message(),
            "the latest simulation is stale: script, settings (measure is not configured)"
        );
        // settings that moved are a change, and say nothing of being unreadable
        let moved = found(
            &SimStale,
            &p,
            runs,
            &settings_now(SettingsNow::Known(settings_of("settings-2"))),
        );
        assert_eq!(moved.len(), 1);
        assert_eq!(
            moved[0].message(),
            "the latest simulation is stale: settings"
        );
        // a Harness the view has no settings for has settings that were not read
        let missing = found(&SimStale, &p, runs, &BTreeMap::new());
        assert_eq!(missing.len(), 1);
        assert!(
            missing[0]
                .message()
                .contains("the settings cannot be read: its settings were not read")
        );
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
        let all = found(&SimFinding, &p, runs, &unchanged());
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
        assert!(found(&SimFinding, &moved, runs, &unchanged()).is_empty());
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
        let all = found(&SimUnreached, &p, std::slice::from_ref(&run), &unchanged());
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
        assert!(found(&SimUnreached, &p, std::slice::from_ref(&run), &unchanged()).is_empty());
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
            &unchanged(),
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
                &unchanged()
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
        let all = found(&SimError, &p, runs, &unchanged());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].level(), Level::Warning);
        assert!(
            all[0]
                .message()
                .contains("no_build: the program does not build")
        );
        assert!(found(&SimFinding, &p, runs, &unchanged()).is_empty());
        assert!(found(&SimStale, &p, runs, &unchanged()).is_empty());
        assert!(found(&SimUnrun, &p, runs, &unchanged()).is_empty());

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
        let all = found(&SimError, &p, std::slice::from_ref(&crashed), &unchanged());
        assert_eq!(all.len(), 1);
        assert_eq!(
            all[0].message(),
            "the script failed in 2 case(s); first: seed-1 ended script:2"
        );
        let clean = simulated(&p, 4, SimOutcome::default());
        assert!(found(&SimError, &p, &[clean], &unchanged()).is_empty());
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
        let errors = found(&SimError, &p, runs, &unchanged());
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message(),
            "the script failed in 2 case(s); first: seed-0 ended \
             cannot_start:python3: No such file or directory; \
             the program could not be started in 2 of them"
        );
        // the one case that drove the program is the only finding; the two that did not add none
        let findings = found(&SimFinding, &p, runs, &unchanged());
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
        let errors = found(&SimError, &p, runs, &unchanged());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].level(), Level::Warning);
        assert!(
            errors[0]
                .message()
                .contains("no_start: the script could not start")
        );
        assert!(found(&SimFinding, &p, runs, &unchanged()).is_empty());
        assert!(found(&SimUnreached, &p, runs, &unchanged()).is_empty());
        assert!(found(&SimNondeterministic, &p, runs, &unchanged()).is_empty());
        assert!(found(&SimStale, &p, runs, &unchanged()).is_empty());
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
        assert!(found(&SimFinding, &p, &[old, newer], &unchanged()).is_empty());
    }
}
