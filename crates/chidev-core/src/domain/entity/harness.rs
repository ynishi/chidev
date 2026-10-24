//! The Harness: the cases that check a Context, as code, each citing what it checks.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::domain::vo::case::{CaseCites, CaseName};
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{CaseId, HarnessId, InterfaceId};
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::source::Source;

/// One case of a Harness (a test). It is known by its id, so a record of it survives a rename; its revision says
/// whether its code or what it cites changed. Where it is in the code (its lines) is shown but is not its content:
/// moving it changes no revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    id: CaseId,
    revision: Revision,
    name: CaseName,
    lines: LineRange,
    cites: CaseCites,
}

impl Case {
    /// Its id.
    pub fn id(&self) -> CaseId {
        self.id
    }

    /// How many times its code or its citations have changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Its name.
    pub fn name(&self) -> &CaseName {
        &self.name
    }

    /// The lines of the code it is on.
    pub fn lines(&self) -> LineRange {
        self.lines
    }

    /// What it cites.
    pub fn cites(&self) -> &CaseCites {
        &self.cites
    }
}

/// Why code and a list of Cases are not a Harness. The Harness is not made or changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HarnessError {
    /// Two Cases have one id.
    #[error("the case id {0} is given twice")]
    DuplicateId(CaseId),
    /// The lines of a Case are not inside the code.
    #[error("the case {id} is on lines {first}..={last}, past the {total} lines of the code")]
    RangeOutside {
        /// The Case.
        id: CaseId,
        /// The first line it was given.
        first: usize,
        /// The last line it was given.
        last: usize,
        /// How many lines the code has.
        total: usize,
    },
    /// A Case of a Harness of tests cites no Item; a test checks something the Interface shows.
    #[error("the case {0} of a harness of tests cites no item")]
    NoItem(CaseId),
    /// A Case of a sim Harness cites an Item. A simulation checks Clauses; the Interfaces whose pinned code it
    /// builds are the ones the Harness names in `builds`, not ones its Cases cite.
    #[error(
        "the case {0} of a sim harness cites an item; a sim case cites clauses only (name the interfaces to build in `builds`)"
    )]
    SimCitesItem(CaseId),
    /// A Harness of tests names Interfaces to build; only a sim Harness builds the program it drives.
    #[error(
        "a harness of tests has no `builds`; only a sim harness builds the interfaces it drives"
    )]
    BuildsOfTests,
}

/// The cases that check a Context. The code is the content (it is what runs); the Cases are the tests in it,
/// found by the caller, who knows the language, with what each cites. Whether what a Case cites exists is a rule
/// of the Context that owns the Harness, not of the Harness.
///
/// A Harness of tests cites Items (every Case at least one) and Clauses, and the Pin of an Interface stands on the
/// Harnesses whose Cases cite its Items. A sim Harness is a script that drives a program: its Cases cite Clauses
/// only (they are the names the protocol runs the script under, and the Clauses the script checks), and the
/// program is built from the pinned code of the Interfaces in `builds`. Naming an Interface to build is not citing
/// it: it puts the Harness on no Pin's basis and asks no Item of it. Only a sim Harness has `builds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Harness {
    id: HarnessId,
    revision: Revision,
    kind: HarnessKind,
    builds: BTreeSet<InterfaceId>,
    source: Source,
    cases: Vec<Case>,
}

/// A Case as the caller found it: its id, its name, its lines and what it cites.
pub type CaseFound = (CaseId, CaseName, LineRange, CaseCites);

/// The ids of `cases` are each given once, every Case's lines are inside `source`, and `kind` takes the Cases and
/// `builds` as given: a Case of tests cites an Item, a Case of a sim none, and only a sim builds.
fn check(
    kind: HarnessKind,
    builds: &BTreeSet<InterfaceId>,
    source: &Source,
    cases: &[CaseFound],
) -> Result<(), HarnessError> {
    if kind == HarnessKind::Tests && !builds.is_empty() {
        return Err(HarnessError::BuildsOfTests);
    }
    for (k, (id, _, lines, cites)) in cases.iter().enumerate() {
        if cases[..k].iter().any(|(i, ..)| i == id) {
            return Err(HarnessError::DuplicateId(*id));
        }
        if source.lines(*lines).is_none() {
            return Err(HarnessError::RangeOutside {
                id: *id,
                first: lines.first(),
                last: lines.last(),
                total: source.line_count(),
            });
        }
        match kind {
            HarnessKind::Tests if cites.items().is_empty() => {
                return Err(HarnessError::NoItem(*id));
            }
            HarnessKind::Sim if !cites.items().is_empty() => {
                return Err(HarnessError::SimCitesItem(*id));
            }
            HarnessKind::Tests | HarnessKind::Sim => {}
        }
    }
    Ok(())
}

impl Harness {
    /// A new Harness of tests at its first revision, with its Cases in the order given, each at its first revision.
    /// The ids are the caller's: the domain makes none.
    pub fn new(
        id: HarnessId,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<Harness, HarnessError> {
        Harness::of_kind(HarnessKind::Tests, id, BTreeSet::new(), source, cases)
    }

    /// A new Harness of `kind` that builds `builds`, as [`Harness::new`] makes a Harness of tests.
    pub fn of_kind(
        kind: HarnessKind,
        id: HarnessId,
        builds: BTreeSet<InterfaceId>,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<Harness, HarnessError> {
        check(kind, &builds, &source, &cases)?;
        Ok(Harness {
            id,
            revision: Revision::first(),
            kind,
            builds,
            source,
            cases: cases
                .into_iter()
                .map(|(id, name, lines, cites)| Case {
                    id,
                    revision: Revision::first(),
                    name,
                    lines,
                    cites,
                })
                .collect(),
        })
    }

    /// A Harness as a store kept it, revisions included. The rules of [`Harness::new`] hold on it.
    pub(crate) fn restore(
        id: HarnessId,
        kind: HarnessKind,
        revision: Revision,
        builds: BTreeSet<InterfaceId>,
        source: Source,
        cases: Vec<(CaseId, Revision, CaseName, LineRange, CaseCites)>,
    ) -> Result<Harness, HarnessError> {
        let found: Vec<CaseFound> = cases
            .iter()
            .map(|(i, _, n, l, c)| (*i, n.clone(), *l, c.clone()))
            .collect();
        check(kind, &builds, &source, &found)?;
        Ok(Harness {
            id,
            revision,
            kind,
            builds,
            source,
            cases: cases
                .into_iter()
                .map(|(id, revision, name, lines, cites)| Case {
                    id,
                    revision,
                    name,
                    lines,
                    cites,
                })
                .collect(),
        })
    }

    /// Its id.
    pub fn id(&self) -> HarnessId {
        self.id
    }

    /// How many times its content has changed, from 1.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// What it is for.
    pub fn kind(&self) -> HarnessKind {
        self.kind
    }

    /// The Interfaces whose pinned code a sim Harness builds into the program it drives, in the order of their
    /// ids. A Harness of tests builds none.
    pub fn builds(&self) -> &BTreeSet<InterfaceId> {
        &self.builds
    }

    /// Its code.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Its Cases, in order.
    pub fn cases(&self) -> &[Case] {
        &self.cases
    }

    /// The Case with this id.
    pub fn case(&self, id: CaseId) -> Option<&Case> {
        self.cases.iter().find(|c| c.id == id)
    }

    /// Gives the Harness new code and Cases. A Case whose id was there keeps its revision when its name, the text
    /// of its lines and its citations are the same (wherever it now is) and goes to the next one when any differs;
    /// a new id starts at the first; an id left out is gone. The Harness goes to its next revision when the code or
    /// its Cases (which, in what order, named what, citing what) differ, and stays when nothing does. Content that
    /// breaks a rule changes nothing. The kind and the Interfaces it builds stay.
    pub fn revise(&mut self, source: Source, cases: Vec<CaseFound>) -> Result<(), HarnessError> {
        self.revise_as(self.kind, self.builds.clone(), source, cases)
    }

    /// As [`Harness::revise`], and the Harness becomes one of `kind` that builds `builds`. A change of kind or of
    /// what it builds is a change of the Harness: its revision goes up.
    pub fn revise_as(
        &mut self,
        kind: HarnessKind,
        builds: BTreeSet<InterfaceId>,
        source: Source,
        cases: Vec<CaseFound>,
    ) -> Result<(), HarnessError> {
        check(kind, &builds, &source, &cases)?;
        let revised: Vec<Case> = cases
            .into_iter()
            .map(|(id, name, lines, cites)| {
                let revision = match self.case(id) {
                    None => Revision::first(),
                    Some(old)
                        if old.name == name
                            && old.cites == cites
                            && self.source.lines(old.lines) == source.lines(lines) =>
                    {
                        old.revision
                    }
                    Some(old) => old.revision.next(),
                };
                Case {
                    id,
                    revision,
                    name,
                    lines,
                    cites,
                }
            })
            .collect();
        let same = source == self.source
            && kind == self.kind
            && builds == self.builds
            && revised.len() == self.cases.len()
            && revised.iter().zip(&self.cases).all(|(new, old)| {
                new.id == old.id && new.name == old.name && new.cites == old.cites
            });
        if !same {
            self.revision = self.revision.next();
        }
        self.kind = kind;
        self.builds = builds;
        self.source = source;
        self.cases = revised;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::vo::id::{ClauseId, ItemId, test_uuid};

    fn cid(n: u128) -> CaseId {
        CaseId::from_uuid(test_uuid(n))
    }

    fn name(s: &str) -> CaseName {
        CaseName::try_from(s).unwrap()
    }

    fn at(first: usize, last: usize) -> LineRange {
        LineRange::new(first, last).unwrap()
    }

    fn src(s: &str) -> Source {
        Source::new(s.to_string())
    }

    /// Cites the Item `item` at revision `rev`, and no Clause.
    fn cites(item: u128, rev: Revision) -> CaseCites {
        CaseCites::new(
            BTreeMap::from([(ItemId::from_uuid(test_uuid(item)), rev)]),
            BTreeMap::new(),
        )
        .unwrap()
    }

    const CODE: &str = "fn push_then_pop() { a }\nfn empty_has_no_top() { b }\n";

    fn stack() -> Harness {
        Harness::new(
            HarnessId::from_uuid(test_uuid(100)),
            src(CODE),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap()
    }

    fn revisions(h: &Harness) -> Vec<u64> {
        h.cases().iter().map(|c| c.revision().number()).collect()
    }

    #[test]
    fn a_new_harness_and_its_cases_start_at_the_first_revision() {
        let h = stack();
        assert_eq!(h.revision(), Revision::first());
        assert_eq!(revisions(&h), [1, 1]);
        assert_eq!(h.case(cid(2)).unwrap().name().as_str(), "empty_has_no_top");
        assert!(h.case(cid(9)).is_none());
    }

    #[test]
    fn a_case_may_cite_several_items_and_clauses() {
        let items = BTreeMap::from([
            (ItemId::from_uuid(test_uuid(50)), Revision::first()),
            (ItemId::from_uuid(test_uuid(51)), Revision::first().next()),
        ]);
        let clauses = BTreeMap::from([(ClauseId::from_uuid(test_uuid(60)), Revision::first())]);
        let h = Harness::new(
            HarnessId::from_uuid(test_uuid(100)),
            src(CODE),
            vec![(
                cid(1),
                name("push_then_pop"),
                at(1, 1),
                CaseCites::new(items, clauses).unwrap(),
            )],
        )
        .unwrap();
        let c = h.case(cid(1)).unwrap().cites();
        assert_eq!((c.items().len(), c.clauses().len()), (2, 1));
    }

    #[test]
    fn a_duplicate_id_or_a_case_past_the_last_line_is_refused() {
        let id = HarnessId::from_uuid(test_uuid(100));
        assert_eq!(
            Harness::new(
                id,
                src(CODE),
                vec![
                    (cid(1), name("a"), at(1, 1), cites(50, Revision::first())),
                    (cid(1), name("b"), at(2, 2), cites(50, Revision::first())),
                ]
            ),
            Err(HarnessError::DuplicateId(cid(1)))
        );
        assert_eq!(
            Harness::new(
                id,
                src(CODE),
                vec![(cid(1), name("a"), at(2, 3), cites(50, Revision::first()))]
            ),
            Err(HarnessError::RangeOutside {
                id: cid(1),
                first: 2,
                last: 3,
                total: 2
            })
        );
    }

    #[test]
    fn the_same_code_and_cases_again_change_no_revision() {
        let mut h = stack();
        let before = h.clone();
        h.revise(
            src(CODE),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap();
        assert_eq!(h, before);
    }

    #[test]
    fn a_moved_case_keeps_its_revision_and_the_harness_goes_to_the_next() {
        let mut h = stack();
        h.revise(
            src(&format!("// tests\n{CODE}")),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(2, 2),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(3, 3),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap();
        assert_eq!(h.revision().number(), 2);
        assert_eq!(revisions(&h), [1, 1]);
    }

    #[test]
    fn a_changed_body_or_citation_goes_to_the_next_revision_and_the_others_stay() {
        let mut h = stack();
        h.revise(
            src("fn push_then_pop() { a2 }\nfn empty_has_no_top() { b }\n"),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first()),
                ),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (2, vec![2, 1]));
        h.revise(
            src("fn push_then_pop() { a2 }\nfn empty_has_no_top() { b }\n"),
            vec![
                (
                    cid(1),
                    name("push_then_pop"),
                    at(1, 1),
                    cites(50, Revision::first()),
                ),
                (
                    cid(2),
                    name("empty_has_no_top"),
                    at(2, 2),
                    cites(51, Revision::first().next()),
                ),
            ],
        )
        .unwrap();
        assert_eq!((h.revision().number(), revisions(&h)), (3, vec![2, 2]));
    }

    #[test]
    fn a_harness_is_of_tests_unless_made_a_sim_and_a_change_of_kind_is_a_revision() {
        let mut h = stack();
        assert_eq!(h.kind(), HarnessKind::Tests);
        let cases = |h: &Harness| -> Vec<CaseFound> {
            h.cases()
                .iter()
                .map(|c| (c.id(), c.name().clone(), c.lines(), c.cites().clone()))
                .collect()
        };
        // revising without a kind keeps the kind and, with nothing else changed, the revision
        h.revise(src(CODE), cases(&h)).unwrap();
        assert_eq!((h.kind(), h.revision().number()), (HarnessKind::Tests, 1));
        // a Case of tests cites an Item, so it is not a Case of a sim: the Cases are given again for the sim
        let clauses = |h: &Harness| -> Vec<CaseFound> {
            cases(h)
                .into_iter()
                .map(|(id, name, lines, _)| (id, name, lines, sim_cites()))
                .collect()
        };
        h.revise_as(HarnessKind::Sim, BTreeSet::new(), src(CODE), clauses(&h))
            .unwrap();
        assert_eq!((h.kind(), h.revision().number()), (HarnessKind::Sim, 2));
        assert_eq!(revisions(&h), [2, 2]);
        // and the kind stays through a revise that names none
        h.revise(src(CODE), cases(&h)).unwrap();
        assert_eq!((h.kind(), h.revision().number()), (HarnessKind::Sim, 2));
        // a new Harness can be made a sim
        let sim = Harness::of_kind(
            HarnessKind::Sim,
            HarnessId::from_uuid(test_uuid(101)),
            BTreeSet::new(),
            src(CODE),
            cases(&h),
        )
        .unwrap();
        assert_eq!(sim.kind(), HarnessKind::Sim);
    }

    /// Citations of a Case of a sim: one Clause, no Item.
    fn sim_cites() -> CaseCites {
        CaseCites::of(
            BTreeMap::new(),
            BTreeMap::from([(ClauseId::from_uuid(test_uuid(60)), Revision::first())]),
        )
    }

    fn iid(n: u128) -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(n))
    }

    fn sim_case(n: u128) -> CaseFound {
        (cid(n), name("sweep"), at(1, 1), sim_cites())
    }

    #[test]
    fn a_case_of_tests_cites_an_item_and_a_case_of_a_sim_none() {
        let id = HarnessId::from_uuid(test_uuid(100));
        let bare = (
            cid(1),
            name("a"),
            at(1, 1),
            CaseCites::of(BTreeMap::new(), BTreeMap::new()),
        );
        assert_eq!(
            Harness::new(id, src(CODE), vec![bare]),
            Err(HarnessError::NoItem(cid(1)))
        );
        let with_item = (cid(2), name("a"), at(1, 1), cites(50, Revision::first()));
        assert_eq!(
            Harness::of_kind(
                HarnessKind::Sim,
                id,
                BTreeSet::new(),
                src(CODE),
                vec![with_item]
            ),
            Err(HarnessError::SimCitesItem(cid(2)))
        );
        let sim = Harness::of_kind(
            HarnessKind::Sim,
            id,
            BTreeSet::new(),
            src(CODE),
            vec![sim_case(3)],
        )
        .unwrap();
        assert!(sim.case(cid(3)).unwrap().cites().items().is_empty());
    }

    #[test]
    fn only_a_sim_builds_and_what_it_builds_is_part_of_its_content() {
        let id = HarnessId::from_uuid(test_uuid(100));
        let one = BTreeSet::from([iid(7)]);
        assert_eq!(
            Harness::of_kind(
                HarnessKind::Tests,
                id,
                one.clone(),
                src(CODE),
                vec![(cid(1), name("a"), at(1, 1), cites(50, Revision::first()))]
            ),
            Err(HarnessError::BuildsOfTests)
        );
        let mut sim = Harness::of_kind(
            HarnessKind::Sim,
            id,
            one.clone(),
            src(CODE),
            vec![sim_case(3)],
        )
        .unwrap();
        assert_eq!(sim.builds(), &one);
        // the same content again changes nothing; another set of Interfaces is a new revision of the Harness
        // and of no Case
        sim.revise_as(HarnessKind::Sim, one.clone(), src(CODE), vec![sim_case(3)])
            .unwrap();
        assert_eq!(sim.revision().number(), 1);
        let two = BTreeSet::from([iid(7), iid(8)]);
        sim.revise_as(HarnessKind::Sim, two.clone(), src(CODE), vec![sim_case(3)])
            .unwrap();
        assert_eq!((sim.revision().number(), sim.builds()), (2, &two));
        assert_eq!(revisions(&sim), [1]);
        // a plain revise keeps what it builds
        sim.revise(src(CODE), vec![sim_case(3)]).unwrap();
        assert_eq!((sim.revision().number(), sim.builds()), (2, &two));
        // and turning it into tests with builds left is refused, changing nothing
        let before = sim.clone();
        assert_eq!(
            sim.revise_as(
                HarnessKind::Tests,
                two,
                src(CODE),
                vec![(cid(3), name("a"), at(1, 1), cites(50, Revision::first()))]
            ),
            Err(HarnessError::BuildsOfTests)
        );
        assert_eq!(sim, before);
    }

    #[test]
    fn content_that_breaks_a_rule_changes_nothing() {
        let mut h = stack();
        let before = h.clone();
        assert!(
            h.revise(
                src("x\n"),
                vec![(cid(1), name("a"), at(1, 2), cites(50, Revision::first()))]
            )
            .is_err()
        );
        assert_eq!(h, before);
    }
}
