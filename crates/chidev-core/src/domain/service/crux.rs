//! Answering a Crux against the Project, and finding answers the definitions have not caught up with.

use std::collections::BTreeMap;

use super::cite::revision_of;
use crate::domain::entity::crux::{Crux, CruxError};
use crate::domain::entity::project::Project;
use crate::domain::vo::choice::Choice;
use crate::domain::vo::id::CruxId;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

/// The revision each of `targets` has in `project` now; a target no asset has is left out.
pub fn revisions_now(
    project: &Project,
    targets: impl IntoIterator<Item = Target>,
) -> BTreeMap<Target, Revision> {
    targets
        .into_iter()
        .filter_map(|t| revision_of(project, t).map(|r| (t, r)))
        .collect()
}

/// Gives `crux` the person's answer, with each line at the revision its definition has in `project` now.
pub fn answer(crux: &mut Crux, choice: Choice, project: &Project) -> Result<(), CruxError> {
    let now = revisions_now(project, crux.lines().keys().copied());
    crux.take_answer(choice, &now)
}

/// A Crux answered other than its default whose definitions written on the default have not changed since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapplied {
    /// The Crux.
    pub crux: CruxId,
    /// The definitions still at the revision they had when the answer was given, in order.
    pub unchanged: Vec<Target>,
    /// The definitions no asset of the Project has any more, in order.
    pub gone: Vec<Target>,
}

/// Every Crux in `cruxes` answered other than its default that still has a line its answer has not reached: a
/// definition at the revision it had when the answer was given (it was written on the default and has not been
/// changed since), or one that is gone. An answer equal to the default, or no answer, asks for no change.
pub fn unapplied<'a>(
    cruxes: impl IntoIterator<Item = &'a Crux>,
    project: &Project,
) -> Vec<Unapplied> {
    let mut out = Vec::new();
    for crux in cruxes {
        if !crux.differs_from_default() {
            continue;
        }
        let mut unchanged = Vec::new();
        let mut gone = Vec::new();
        for (target, at_answer) in crux.lines() {
            match revision_of(project, *target) {
                None => gone.push(*target),
                Some(now) if now == *at_answer => unchanged.push(*target),
                Some(_) => {}
            }
        }
        if !unchanged.is_empty() || !gone.is_empty() {
            out.push(Unapplied {
                crux: crux.id(),
                unchanged,
                gone,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::entity::context::Context;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::id::{ClauseId, ContextId, ContractId, ProjectId, test_uuid};
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::title::Title;

    fn clause() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(30)))
    }

    fn put_contract(p: &mut Project, text: &str) {
        p.put_contract(
            ContextId::from_uuid(test_uuid(1)),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![(
                ClauseId::from_uuid(test_uuid(30)),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from(text).unwrap(),
                ),
            )],
        )
        .unwrap();
    }

    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                ContextId::from_uuid(test_uuid(1)),
                ContextName::try_from("core").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        put_contract(&mut p, "a half rounds up");
        p
    }

    fn choice(s: &str) -> Choice {
        Choice::try_from(s).unwrap()
    }

    fn crux(p: &Project) -> Crux {
        Crux::new(
            CruxId::from_uuid(test_uuid(1)),
            Title::try_from("How is a half rounded?").unwrap(),
            vec![choice("up"), choice("to even")],
            choice("up"),
            revisions_now(p, [clause()]),
        )
        .unwrap()
    }

    #[test]
    fn an_answer_equal_to_the_default_or_none_asks_for_nothing() {
        let p = project();
        let mut c = crux(&p);
        assert!(unapplied([&c], &p).is_empty());
        answer(&mut c, choice("up"), &p).unwrap();
        assert!(unapplied([&c], &p).is_empty());
    }

    #[test]
    fn another_answer_is_unapplied_until_the_definitions_change() {
        let mut p = project();
        let mut c = crux(&p);
        answer(&mut c, choice("to even"), &p).unwrap();
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                crux: c.id(),
                unchanged: vec![clause()],
                gone: Vec::new()
            }]
        );
        put_contract(&mut p, "a half rounds to even");
        assert!(unapplied([&c], &p).is_empty());
    }

    #[test]
    fn a_definition_that_is_gone_is_reported() {
        let p = project();
        let mut c = crux(&p);
        answer(&mut c, choice("to even"), &p).unwrap();
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(
            unapplied([&c], &empty),
            [Unapplied {
                crux: c.id(),
                unchanged: Vec::new(),
                gone: vec![clause()]
            }]
        );
    }
}
