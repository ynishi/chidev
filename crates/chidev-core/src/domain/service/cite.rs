//! The revision of anything a Case or a change request can point at, and the citations a Case has not been
//! brought up to: the cited Item or Clause changed after the Case was written.

use crate::domain::entity::project::Project;
use crate::domain::vo::id::CaseId;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

/// The revision `target` has in `project` now; `None` when the Project does not have it. The Project always has
/// itself. Ids are given once in the whole Project, so the first one found is the one.
pub fn revision_of(project: &Project, target: Target) -> Option<Revision> {
    match target {
        Target::Project => Some(project.revision()),
        Target::Context(id) => project.context(id).map(|c| c.revision()),
        Target::Contract(id) => project
            .contexts()
            .find_map(|c| c.contract(id))
            .map(|k| k.revision()),
        Target::Interface(id) => project
            .contexts()
            .find_map(|c| c.interface(id))
            .map(|i| i.revision()),
        Target::Harness(id) => project
            .contexts()
            .find_map(|c| c.harness(id))
            .map(|h| h.revision()),
        Target::Clause(id) => project.contexts().find_map(|c| {
            c.contract_of(id)
                .and_then(|k| k.clause(id))
                .map(|k| k.revision())
        }),
        Target::Item(id) => project.contexts().find_map(|c| {
            c.interface_of(id)
                .and_then(|i| i.item(id))
                .map(|i| i.revision())
        }),
        Target::Case(id) => project.contexts().find_map(|c| {
            c.harness_of(id)
                .and_then(|h| h.case(id))
                .map(|k| k.revision())
        }),
    }
}

/// A citation whose target moved past the revision the Case recorded. It is not refused (the Project holds
/// either way); it says that the Case was written against an older Item or Clause and may need to change with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleCite {
    /// The Case that cites it.
    pub case: CaseId,
    /// What it cites.
    pub target: Target,
    /// The revision the Case recorded.
    pub cited: Revision,
    /// The revision the target has now.
    pub now: Revision,
}

/// Every citation in `project` whose target has a later revision than the one its Case recorded, in the order of
/// the Contexts, their Harnesses and Cases, Items before Clauses. A target that is not there is not listed: the
/// Project refuses that state.
pub fn stale_cites(project: &Project) -> Vec<StaleCite> {
    let mut out = Vec::new();
    for case in project
        .contexts()
        .flat_map(|c| c.harnesses())
        .flat_map(|h| h.cases())
    {
        let cites = case.cites();
        let targets = cites
            .items()
            .iter()
            .map(|(id, rev)| (Target::Item(*id), *rev))
            .chain(
                cites
                    .clauses()
                    .iter()
                    .map(|(id, rev)| (Target::Clause(*id), *rev)),
            );
        for (target, cited) in targets {
            if let Some(now) = revision_of(project, target).filter(|now| *now > cited) {
                out.push(StaleCite {
                    case: case.id(),
                    target,
                    cited,
                    now,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::domain::entity::context::Context;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::id::{
        ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId, ProjectId, test_uuid,
    };
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::signature::Signature;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::title::Title;

    fn put_interface(p: &mut Project, sig: &str) {
        p.put_interface(
            ContextId::from_uuid(test_uuid(1)),
            InterfaceId::from_uuid(test_uuid(100)),
            Source::new(format!("{sig}\n")),
            vec![(
                ItemId::from_uuid(test_uuid(10)),
                Signature::try_from(sig).unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
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
        put_interface(&mut p, "fn a()");
        put_contract(&mut p, "a promise");
        let cites = CaseCites::new(
            BTreeMap::from([(ItemId::from_uuid(test_uuid(10)), Revision::first())]),
            BTreeMap::from([(ClauseId::from_uuid(test_uuid(30)), Revision::first())]),
        )
        .unwrap();
        p.put_harness(
            ContextId::from_uuid(test_uuid(1)),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new("fn t() {}\n".to_string()),
            vec![(
                CaseId::from_uuid(test_uuid(40)),
                CaseName::try_from("t").unwrap(),
                LineRange::new(1, 1).unwrap(),
                cites,
            )],
        )
        .unwrap();
        p
    }

    #[test]
    fn citations_at_the_current_revisions_are_not_stale() {
        assert!(stale_cites(&project()).is_empty());
    }

    #[test]
    fn a_target_that_moved_past_the_recorded_revision_is_listed() {
        let mut p = project();
        put_interface(&mut p, "fn a(x: u8)");
        put_contract(&mut p, "another promise");
        let case = CaseId::from_uuid(test_uuid(40));
        let second = Revision::first().next();
        assert_eq!(
            stale_cites(&p),
            [
                StaleCite {
                    case,
                    target: Target::Item(ItemId::from_uuid(test_uuid(10))),
                    cited: Revision::first(),
                    now: second,
                },
                StaleCite {
                    case,
                    target: Target::Clause(ClauseId::from_uuid(test_uuid(30))),
                    cited: Revision::first(),
                    now: second,
                },
            ]
        );
    }
}
