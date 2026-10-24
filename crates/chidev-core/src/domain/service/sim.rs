//! The sim Harness read against the model: which Harnesses of a Context are simulations, whether each has the shape
//! the protocol needs, which Clauses its sweep names for the script, and what a simulation of it stands on.
//!
//! A sim Harness is one script. Its Cases are not tests in the script's code: they are the names the protocol
//! runs it under. The one `sweep` Case stands for the seeded runs (`seed-0`, `seed-1`, ...) and cites every Clause
//! the script checks, so a label in the script's report can be traced to a Clause; each `regress-<name>` Case
//! stands for a fixed schedule the script keeps.

use std::collections::{BTreeMap, BTreeSet};

use crate::domain::entity::context::Context;
use crate::domain::entity::harness::{Case, Harness};
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::clause::ClauseLabel;
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{CaseId, ClauseId, ContextId, HarnessId, InterfaceId};
use crate::domain::vo::sim::{CaseRole, RegressName, SimBasis, role_of};

/// What a problem of shape is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeAbout {
    /// The Context (a sim Harness as a whole).
    Context(ContextId),
    /// One Case of a sim Harness.
    Case(CaseId),
}

/// A way a sim Harness is not the shape the protocol runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeProblem {
    /// What it is about.
    pub about: ShapeAbout,
    /// What is wrong, and what to do.
    pub message: String,
}

/// The Harnesses of `context` that are simulations, in the order of their ids.
pub fn sim_harnesses(context: &Context) -> impl Iterator<Item = &Harness> {
    context.harnesses().filter(|h| h.kind() == HarnessKind::Sim)
}

/// The sweep Case of a sim Harness: the one named `sweep` (the first, if the shape is wrong).
pub fn sweep(harness: &Harness) -> Option<&Case> {
    harness
        .cases()
        .iter()
        .find(|c| role_of(c.name().as_str()) == Some(CaseRole::Sweep))
}

/// The regress cases of a sim Harness, in the order of its Cases, each name once.
pub fn regress_names(harness: &Harness) -> Vec<RegressName> {
    let mut names: Vec<RegressName> = Vec::new();
    for c in harness.cases() {
        if let Some(CaseRole::Regress(n)) = role_of(c.name().as_str())
            && !names.contains(&n)
        {
            names.push(n);
        }
    }
    names
}

/// The Clauses `case` cites, by the label the script writes them as. A Clause of another Context's Contract
/// counts as well as one of `context`'s; two Clauses with one label keep the first (the shape check refuses
/// that).
pub fn labels_of(project: &Project, case: &Case) -> BTreeMap<ClauseLabel, ClauseId> {
    let mut out = BTreeMap::new();
    for id in case.cites().clauses().keys() {
        if let Some(clause) = project
            .contexts()
            .find_map(|c| c.contract_of(*id))
            .and_then(|k| k.clause(*id))
        {
            out.entry(clause.content().label()).or_insert(*id);
        }
    }
    out
}

/// Every way the sim Harnesses of `context` are not the shape the protocol runs: more than one sim Harness, no
/// `sweep` Case or two, a Case that is neither `sweep` nor `regress-<name>` or whose name another has, a Case
/// that cites no Clause or a Clause of no Contract of the Context, and a sweep that cites two Clauses with one
/// label (the script could not say which). A Context with no sim Harness has none.
pub fn shape(context: &Context) -> Vec<ShapeProblem> {
    let mut out = Vec::new();
    let harnesses: Vec<&Harness> = sim_harnesses(context).collect();
    let about_context = ShapeAbout::Context(context.id());
    if harnesses.len() > 1 {
        out.push(ShapeProblem {
            about: about_context,
            message: format!(
                "the context has {} sim harnesses; it has at most one, so that a clause is checked by one script",
                harnesses.len()
            ),
        });
    }
    let own: BTreeSet<ClauseId> = context
        .contracts()
        .flat_map(|k| k.clauses().iter().map(|c| c.id()))
        .collect();
    for h in harnesses {
        let sweeps: Vec<&Case> = h
            .cases()
            .iter()
            .filter(|c| role_of(c.name().as_str()) == Some(CaseRole::Sweep))
            .collect();
        if sweeps.is_empty() {
            out.push(ShapeProblem {
                about: about_context,
                message: "a sim harness has no case named `sweep`; it has exactly one, citing every clause the script checks".to_string(),
            });
        }
        for extra in sweeps.iter().skip(1) {
            out.push(ShapeProblem {
                about: ShapeAbout::Case(extra.id()),
                message: "a sim harness has one case named `sweep`; this is another".to_string(),
            });
        }
        let mut seen: Vec<RegressName> = Vec::new();
        for c in h.cases() {
            let case = ShapeAbout::Case(c.id());
            let name = c.name().as_str();
            match role_of(name) {
                None => out.push(ShapeProblem {
                    about: case,
                    message: format!(
                        "`{name}` is neither `sweep` nor `regress-<name>` (a name of a-z, 0-9 and `-`)"
                    ),
                }),
                Some(CaseRole::Regress(n)) => {
                    if seen.contains(&n) {
                        out.push(ShapeProblem {
                            about: case,
                            message: format!("the regress case `{n}` is named twice"),
                        });
                    }
                    seen.push(n);
                }
                Some(CaseRole::Sweep) => {}
            }
            if role_of(name).is_some() {
                if c.cites().clauses().is_empty() {
                    out.push(ShapeProblem {
                        about: case,
                        message: format!(
                            "`{name}` cites no clause; a sim case cites the clauses it checks"
                        ),
                    });
                }
                if let Some(foreign) = c.cites().clauses().keys().find(|k| !own.contains(k)) {
                    out.push(ShapeProblem {
                        about: case,
                        message: format!(
                            "`{name}` cites the clause {foreign}, which is in no contract of this context"
                        ),
                    });
                }
            }
        }
        if let Some(s) = sweeps.first() {
            let mut labels: BTreeMap<ClauseLabel, ClauseId> = BTreeMap::new();
            for id in s.cites().clauses().keys() {
                let label = context
                    .contract_of(*id)
                    .and_then(|k| k.clause(*id))
                    .map(|c| c.content().label());
                if let Some(label) = label
                    && labels.insert(label, *id).is_some()
                {
                    out.push(ShapeProblem {
                        about: ShapeAbout::Case(s.id()),
                        message: format!(
                            "the sweep cites two clauses labelled {label}; the script names a clause by its label, so each label is cited once"
                        ),
                    });
                }
            }
        }
    }
    out
}

/// The Interfaces whose Items the Cases of `harness` cite, in the order of their ids.
pub fn cited_interfaces(project: &Project, harness: &Harness) -> BTreeSet<InterfaceId> {
    harness
        .cases()
        .iter()
        .flat_map(|c| c.cites().items().keys())
        .filter_map(|item| {
            project
                .contexts()
                .find_map(|c| c.interface_of(*item))
                .map(|i| i.id())
        })
        .collect()
}

/// What a simulation of the sim Harness `harness` stands on now: its revision, the Contracts its Cases cite, the
/// Interfaces whose Items they cite, each with the digest of the code pinned for it in `pins`. `None` when no sim
/// Harness has this id.
pub fn basis_of(project: &Project, harness: HarnessId, pins: &[Pin]) -> Option<SimBasis> {
    let h = project
        .contexts()
        .find_map(|c| c.harness(harness))
        .filter(|h| h.kind() == HarnessKind::Sim)?;
    let mut contracts = BTreeMap::new();
    for clause in h.cases().iter().flat_map(|c| c.cites().clauses().keys()) {
        if let Some(k) = project.contexts().find_map(|c| c.contract_of(*clause)) {
            contracts.insert(k.id(), k.revision());
        }
    }
    let mut interfaces = BTreeMap::new();
    for id in cited_interfaces(project, h) {
        if let Some(i) = project.contexts().find_map(|c| c.interface(id)) {
            let pin = pins.iter().find(|p| p.interface() == id).map(Pin::code);
            interfaces.insert(id, (i.revision(), pin));
        }
    }
    Some(SimBasis {
        harness: h.revision(),
        contracts,
        interfaces,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::service::pin::record;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseText};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::{ContractId, ItemId, PinId, ProjectId, test_uuid};
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::signature::Signature;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::title::Title;

    fn ctx() -> ContextId {
        ContextId::from_uuid(test_uuid(1))
    }

    fn clause(n: u128) -> ClauseId {
        ClauseId::from_uuid(test_uuid(n))
    }

    /// A Context with Contract 300 (Clauses 30 = C1, 31 = C2) and Interface 100 (Item 10).
    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                ctx(),
                ContextName::try_from("core").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
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
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![c(30, "C1", "one"), c(31, "C2", "two")],
        )
        .unwrap();
        p.put_interface(
            ctx(),
            InterfaceId::from_uuid(test_uuid(100)),
            Source::new("fn a()\n".to_string()),
            vec![(
                ItemId::from_uuid(test_uuid(10)),
                Signature::try_from("fn a()").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        p
    }

    /// A Case named `name` citing Item 10 and these Clauses.
    fn case(n: u128, name: &str, clauses: &[u128]) -> crate::domain::entity::harness::CaseFound {
        (
            CaseId::from_uuid(test_uuid(n)),
            CaseName::try_from(name).unwrap(),
            LineRange::new(1, 1).unwrap(),
            CaseCites::new(
                BTreeMap::from([(ItemId::from_uuid(test_uuid(10)), Revision::first())]),
                clauses
                    .iter()
                    .map(|k| (clause(*k), Revision::first()))
                    .collect(),
            )
            .unwrap(),
        )
    }

    fn put_sim(p: &mut Project, id: u128, cases: Vec<crate::domain::entity::harness::CaseFound>) {
        p.put_harness_as(
            ctx(),
            HarnessId::from_uuid(test_uuid(id)),
            Some(HarnessKind::Sim),
            Source::new("script\n".to_string()),
            cases,
        )
        .unwrap();
    }

    fn problems(p: &Project) -> Vec<String> {
        shape(p.context(ctx()).unwrap())
            .into_iter()
            .map(|s| s.message)
            .collect()
    }

    #[test]
    fn a_context_without_a_sim_harness_has_no_shape_to_get_wrong() {
        let mut p = project();
        assert!(problems(&p).is_empty());
        // a Harness of tests is not read as a simulation, whatever its Cases are called
        p.put_harness(
            ctx(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new("t\n".into()),
            vec![case(40, "anything", &[])],
        )
        .unwrap();
        assert!(problems(&p).is_empty());
        assert_eq!(sim_harnesses(p.context(ctx()).unwrap()).count(), 0);
    }

    #[test]
    fn a_sweep_and_regress_cases_that_cite_clauses_are_the_shape() {
        let mut p = project();
        put_sim(
            &mut p,
            500,
            vec![
                case(50, "sweep", &[30, 31]),
                case(51, "regress-lease-lost", &[31]),
            ],
        );
        assert!(problems(&p).is_empty());
        let h = p
            .context(ctx())
            .unwrap()
            .harness(HarnessId::from_uuid(test_uuid(500)))
            .unwrap();
        assert_eq!(sweep(h).unwrap().name().as_str(), "sweep");
        assert_eq!(
            regress_names(h)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["lease-lost"]
        );
        let labels = labels_of(&p, sweep(h).unwrap());
        assert_eq!(labels.len(), 2);
        assert_eq!(
            labels.get(&ClauseLabel::try_from("C2").unwrap()),
            Some(&clause(31))
        );
    }

    #[test]
    fn each_way_to_be_malformed_is_a_problem() {
        // no sweep
        let mut p = project();
        put_sim(&mut p, 500, vec![case(51, "regress-a", &[30])]);
        assert_eq!(problems(&p).len(), 1);
        assert!(problems(&p)[0].contains("no case named `sweep`"));

        // two sweeps, a stray name, a repeated regress name, a case that cites no clause
        let mut p = project();
        put_sim(
            &mut p,
            500,
            vec![
                case(50, "sweep", &[30]),
                case(51, "sweep", &[30]),
                case(52, "Regress-A", &[30]),
                case(53, "regress-a", &[30]),
                case(54, "regress-a", &[30]),
                case(55, "regress-b", &[]),
            ],
        );
        let all = shape(p.context(ctx()).unwrap());
        let about = |n: u128| {
            all.iter()
                .filter(|s| s.about == ShapeAbout::Case(CaseId::from_uuid(test_uuid(n))))
                .map(|s| s.message.as_str())
                .collect::<Vec<_>>()
        };
        assert!(about(51)[0].contains("another"));
        assert!(about(52)[0].contains("neither `sweep` nor"));
        assert!(about(54)[0].contains("named twice"));
        assert!(about(55)[0].contains("cites no clause"));
        assert!(about(50).is_empty() && about(53).is_empty());
        assert_eq!(all.len(), 4);

        // a Clause the Project does not have is refused before the shape is asked
        let mut p = project();
        assert!(
            p.put_harness_as(
                ctx(),
                HarnessId::from_uuid(test_uuid(500)),
                Some(HarnessKind::Sim),
                Source::new("s\n".into()),
                vec![case(50, "sweep", &[99])]
            )
            .is_err()
        );
    }

    #[test]
    fn a_clause_of_a_contract_of_another_context_is_not_one_the_script_checks() {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        let base = ContextId::from_uuid(test_uuid(2));
        p.add_context(
            Context::new(
                base,
                ContextName::try_from("base").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        p.put_contract(
            base,
            ContractId::from_uuid(test_uuid(310)),
            Title::try_from("B").unwrap(),
            vec![(
                clause(90),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("base promise").unwrap(),
                ),
            )],
        )
        .unwrap();
        p.add_context(
            Context::new(
                ctx(),
                ContextName::try_from("core").unwrap(),
                BTreeSet::from([base]),
            )
            .unwrap(),
        )
        .unwrap();
        p.put_interface(
            ctx(),
            InterfaceId::from_uuid(test_uuid(100)),
            Source::new("fn a()\n".to_string()),
            vec![(
                ItemId::from_uuid(test_uuid(10)),
                Signature::try_from("fn a()").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        put_sim(&mut p, 500, vec![case(50, "sweep", &[90])]);
        let found = problems(&p);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("in no contract of this context"));
    }

    #[test]
    fn two_sim_harnesses_and_two_clauses_with_one_label_in_the_sweep_are_problems() {
        let mut p = project();
        put_sim(&mut p, 500, vec![case(50, "sweep", &[30])]);
        put_sim(&mut p, 501, vec![case(60, "sweep", &[31])]);
        let found = problems(&p);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("2 sim harnesses"));

        // a second Contract whose clause is also C1
        let mut p = project();
        p.put_contract(
            ctx(),
            ContractId::from_uuid(test_uuid(301)),
            Title::try_from("U").unwrap(),
            vec![(
                clause(32),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("again").unwrap(),
                ),
            )],
        )
        .unwrap();
        put_sim(&mut p, 500, vec![case(50, "sweep", &[30, 32])]);
        let found = problems(&p);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("two clauses labelled C1"));
    }

    #[test]
    fn what_a_simulation_stands_on_follows_the_harness_the_contract_the_interface_and_its_pin() {
        let mut p = project();
        put_sim(&mut p, 500, vec![case(50, "sweep", &[30])]);
        let harness = HarnessId::from_uuid(test_uuid(500));
        let none = basis_of(&p, harness, &[]).unwrap();
        assert_eq!(none.contracts.len(), 1);
        assert_eq!(none.interfaces.len(), 1);
        assert!(none.interfaces.values().all(|(_, pin)| pin.is_none()));
        // a Harness of tests has no simulation to stand on
        let mut q = project();
        q.put_harness(
            ctx(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new("t\n".into()),
            vec![case(40, "t", &[])],
        )
        .unwrap();
        assert!(basis_of(&q, HarnessId::from_uuid(test_uuid(400)), &[]).is_none());

        // the Pin of the cited Interface is in it, by its code
        let pin = record(
            &p,
            InterfaceId::from_uuid(test_uuid(100)),
            PinId::from_uuid(test_uuid(50)),
            Digest::of(b"code"),
        );
        // pinning needs a Case of tests that cites the Interface, which the sim Harness is
        let pin = pin.unwrap();
        let with_pin = basis_of(&p, harness, std::slice::from_ref(&pin)).unwrap();
        assert_ne!(none, with_pin);
        assert_eq!(
            with_pin.interfaces.values().next().unwrap().1,
            Some(Digest::of(b"code"))
        );

        // revising the sim Harness, a cited Contract or the Interface moves it
        let mut r = p.clone();
        put_sim(&mut r, 500, vec![case(50, "sweep", &[30, 31])]);
        assert_ne!(basis_of(&r, harness, &[]), Some(none.clone()));
        let mut c = p.clone();
        c.put_contract(
            ctx(),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![
                (
                    clause(30),
                    ClauseContent::new(
                        ClauseLabel::try_from("C1").unwrap(),
                        ClauseText::try_from("one, changed").unwrap(),
                    ),
                ),
                (
                    clause(31),
                    ClauseContent::new(
                        ClauseLabel::try_from("C2").unwrap(),
                        ClauseText::try_from("two").unwrap(),
                    ),
                ),
            ],
        )
        .unwrap();
        assert_ne!(basis_of(&c, harness, &[]), Some(none));
    }

    #[test]
    fn the_interfaces_a_sim_harness_cites_are_found_by_its_items() {
        let mut p = project();
        put_sim(&mut p, 500, vec![case(50, "sweep", &[30])]);
        let h = p
            .context(ctx())
            .unwrap()
            .harness(HarnessId::from_uuid(test_uuid(500)))
            .unwrap();
        let cited: Vec<_> = cited_interfaces(&p, h).into_iter().collect();
        assert_eq!(cited, [InterfaceId::from_uuid(test_uuid(100))]);
    }
}
