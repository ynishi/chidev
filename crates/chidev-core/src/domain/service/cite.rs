//! Citations a Case has not been brought up to: the cited Item or Clause changed after the Case was written.

use crate::domain::entity::context::Context;
use crate::domain::entity::project::Project;
use crate::domain::vo::id::{CaseId, ClauseId, ItemId};
use crate::domain::vo::revision::Revision;

/// What a citation points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    /// An Item of an Interface.
    Item(ItemId),
    /// A Clause of a Contract.
    Clause(ClauseId),
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
    for context in project.contexts() {
        let reach: Vec<&Context> = std::iter::once(context)
            .chain(context.uses().iter().filter_map(|u| project.context(*u)))
            .collect();
        for case in context.harnesses().flat_map(|h| h.cases()) {
            for (item, cited) in case.cites().items() {
                let now = reach
                    .iter()
                    .find_map(|c| c.interface_of(*item))
                    .and_then(|i| i.item(*item))
                    .map(|i| i.revision());
                if let Some(now) = now.filter(|now| now > cited) {
                    out.push(StaleCite {
                        case: case.id(),
                        target: Target::Item(*item),
                        cited: *cited,
                        now,
                    });
                }
            }
            for (clause, cited) in case.cites().clauses() {
                let now = reach
                    .iter()
                    .find_map(|c| c.contract_of(*clause))
                    .and_then(|k| k.clause(*clause))
                    .map(|k| k.revision());
                if let Some(now) = now.filter(|now| now > cited) {
                    out.push(StaleCite {
                        case: case.id(),
                        target: Target::Clause(*clause),
                        cited: *cited,
                        now,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::id::{
        ContextId, ContractId, HarnessId, InterfaceId, ProjectId, test_uuid,
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
