//! The sim Harness read against the model: which Harnesses of a Context are simulations, which Clauses its sweep
//! names for the script, and what a simulation of it stands on. Whether a sim Harness has the shape the protocol
//! runs is for the Harness and the Context to say (`Harness::sim_problems`, `Context::sim_problems`), and the
//! Project refuses a put of one that has not.
//!
//! A sim Harness is one script. Its Cases are not tests in the script's code: they are the names the protocol
//! runs it under. The one `sweep` Case stands for the seeded runs (`seed-0`, `seed-1`, ...) and cites every Clause
//! the script checks, of any number of the Contracts of the Context, so a Clause named in the script's report
//! (`<contract>:<label>`) can be traced to a Clause; each `regress-<name>` Case stands for a fixed schedule the
//! script keeps. The Harness cites Clauses only; the program the script drives is built from the pinned code of
//! the Interfaces it `builds`.

use std::collections::BTreeMap;

use crate::domain::entity::context::Context;
use crate::domain::entity::harness::{Case, Harness};
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::clause::ClauseLabel;
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{ClauseId, ContractId, HarnessId};
use crate::domain::vo::sim::{CaseRole, RegressName, SimBasis, role_of};

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

/// A Clause a Case cites, with the Contract it is in and its label there. A label is unique only within its
/// Contract, so a script names the Clause by the Contract too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CitedIn {
    /// The Contract the Clause is in.
    pub contract: ContractId,
    /// Its label in that Contract.
    pub label: ClauseLabel,
    /// The Clause.
    pub clause: ClauseId,
}

/// The Clauses `case` cites that the Project has, with their Contracts, in the order of their ids. Clauses of
/// several Contracts may share a label; they are different Clauses.
pub fn cited_in(project: &Project, case: &Case) -> Vec<CitedIn> {
    case.cites()
        .clauses()
        .keys()
        .filter_map(|id| {
            let contract = project.contexts().find_map(|c| c.contract_of(*id))?;
            let clause = contract.clause(*id)?;
            Some(CitedIn {
                contract: contract.id(),
                label: clause.content().label(),
                clause: *id,
            })
        })
        .collect()
}

/// What a simulation of the sim Harness `harness` stands on now: its revision, the Contracts its Cases cite a
/// Clause of, the Interfaces it builds, each with the digest of the code pinned for it in `pins`. `None` when no
/// sim Harness has this id. The Interfaces are the ones it names in `builds`, not ones its Cases cite (a sim Case
/// cites no Item), so the simulation stands on the Pin of each without the Pin standing on the simulation.
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
    for id in h.builds() {
        let id = *id;
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
    use crate::domain::entity::harness::SimShapeProblem;
    use crate::domain::service::pin::record;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseText};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::{
        CaseId, ContextId, ContractId, InterfaceId, ItemId, PinId, ProjectId, test_uuid,
    };
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::revision::Revision;
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
                ItemKey::try_from("a").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        p
    }

    fn iface() -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(100))
    }

    /// A Case named `name` of a sim: it cites these Clauses and no Item.
    fn case(n: u128, name: &str, clauses: &[u128]) -> crate::domain::entity::harness::CaseFound {
        (
            CaseId::from_uuid(test_uuid(n)),
            CaseName::try_from(name).unwrap(),
            LineRange::new(1, 1).unwrap(),
            CaseCites::of(
                BTreeMap::new(),
                clauses
                    .iter()
                    .map(|k| (clause(*k), Revision::first()))
                    .collect(),
            ),
        )
    }

    /// A Case of tests citing Item 10 and no Clause.
    fn test_case(n: u128, name: &str) -> crate::domain::entity::harness::CaseFound {
        (
            CaseId::from_uuid(test_uuid(n)),
            CaseName::try_from(name).unwrap(),
            LineRange::new(1, 1).unwrap(),
            CaseCites::new(
                BTreeMap::from([(ItemId::from_uuid(test_uuid(10)), Revision::first())]),
                BTreeMap::new(),
            )
            .unwrap(),
        )
    }

    /// A sim Harness that builds Interface 100.
    fn put_sim(p: &mut Project, id: u128, cases: Vec<crate::domain::entity::harness::CaseFound>) {
        p.put_harness_as(
            ctx(),
            HarnessId::from_uuid(test_uuid(id)),
            Some(HarnessKind::Sim),
            BTreeSet::from([iface()]),
            Source::new("script\n".to_string()),
            cases,
        )
        .unwrap();
    }

    /// What is wrong with the sim Harnesses of the Context, as the Context says it.
    fn problems(p: &Project) -> Vec<SimShapeProblem> {
        let c = p.context(ctx()).unwrap();
        c.harnesses().flat_map(|h| c.sim_problems(h)).collect()
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
            vec![test_case(40, "anything")],
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
        let cited = cited_in(&p, sweep(h).unwrap());
        assert_eq!(cited.len(), 2);
        assert_eq!(
            cited
                .iter()
                .find(|c| c.label == ClauseLabel::try_from("C2").unwrap())
                .map(|c| (c.clause, c.contract)),
            Some((clause(31), ContractId::from_uuid(test_uuid(300))))
        );
    }

    #[test]
    fn one_label_in_two_contracts_is_no_problem_and_each_clause_is_named_by_its_contract() {
        // a second Contract whose clause is also C1: one sweep cites both, and each is named by its Contract
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
        assert!(problems(&p).is_empty());
        let h = p
            .context(ctx())
            .unwrap()
            .harness(HarnessId::from_uuid(test_uuid(500)))
            .unwrap();
        let cited = cited_in(&p, sweep(h).unwrap());
        let c1 = ClauseLabel::try_from("C1").unwrap();
        let in_contract = |c: &CitedIn| c.contract.as_uuid();
        assert_eq!(cited.len(), 2);
        assert!(cited.iter().all(|c| c.label == c1));
        assert_ne!(in_contract(&cited[0]), in_contract(&cited[1]));
        assert_eq!(
            cited.iter().map(|c| c.clause).collect::<Vec<_>>(),
            [clause(30), clause(32)]
        );
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
            vec![test_case(40, "t")],
        )
        .unwrap();
        assert!(basis_of(&q, HarnessId::from_uuid(test_uuid(400)), &[]).is_none());

        // pinning needs a Case of tests that cites the Interface; the sim Harness cites no Item, so it is none
        assert!(
            record(
                &p,
                iface(),
                PinId::from_uuid(test_uuid(50)),
                Digest::of(b"code")
            )
            .is_err()
        );
        p.put_harness(
            ctx(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new("t\n".into()),
            vec![test_case(40, "t")],
        )
        .unwrap();
        let pin = record(
            &p,
            iface(),
            PinId::from_uuid(test_uuid(50)),
            Digest::of(b"code"),
        )
        .unwrap();
        // the Pin of the built Interface is in the basis, by its code; the Pin does not stand on the sim Harness
        assert!(
            !pin.basis()
                .harnesses()
                .contains_key(&HarnessId::from_uuid(test_uuid(500)))
        );
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
    fn what_a_sim_builds_is_in_its_basis_and_what_it_stops_building_is_not() {
        let mut p = project();
        // a second Interface, which the sim comes to build
        p.put_interface(
            ctx(),
            InterfaceId::from_uuid(test_uuid(101)),
            Source::new("fn b()\n".to_string()),
            vec![(
                ItemId::from_uuid(test_uuid(11)),
                ItemKey::try_from("b").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        put_sim(&mut p, 500, vec![case(50, "sweep", &[30])]);
        let harness = HarnessId::from_uuid(test_uuid(500));
        let one = basis_of(&p, harness, &[]).unwrap();
        assert_eq!(
            one.interfaces.keys().copied().collect::<Vec<_>>(),
            [iface()]
        );
        p.put_harness_as(
            ctx(),
            harness,
            None,
            BTreeSet::from([iface(), InterfaceId::from_uuid(test_uuid(101))]),
            Source::new("script\n".to_string()),
            vec![case(50, "sweep", &[30])],
        )
        .unwrap();
        let two = basis_of(&p, harness, &[]).unwrap();
        assert_eq!(two.interfaces.len(), 2);
        // naming another Interface to build is a revision of the Harness, so the simulation is stale
        assert_ne!(one.harness, two.harness);
        assert_ne!(one, two);
    }

    #[test]
    fn a_revision_of_an_interface_the_sim_builds_moves_its_basis_though_no_case_cites_it() {
        let mut p = project();
        put_sim(&mut p, 500, vec![case(50, "sweep", &[30])]);
        let harness = HarnessId::from_uuid(test_uuid(500));
        let before = basis_of(&p, harness, &[]).unwrap();
        p.put_interface(
            ctx(),
            iface(),
            Source::new("fn a(x: u8)\n".to_string()),
            vec![(
                ItemId::from_uuid(test_uuid(10)),
                ItemKey::try_from("a").unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
        assert_ne!(basis_of(&p, harness, &[]).unwrap(), before);
    }
}
