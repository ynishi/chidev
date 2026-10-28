//! The Frame: the targets a change request names to revise or retire, and what is inside them.
//!
//! "Inside the frame" is defined here once. It is read for the work a request did (what a change set moved that the
//! request is not about), for what an answer adds to a request (a target outside the frame is not asked of it), and
//! for which citations a Harness put for a request brings up (a Case is claimed current only for what the request
//! is about). Naming a container frames everything in it.

use std::collections::BTreeSet;

use super::footprint::Citations;
use super::target::Target;

/// The targets a change request names to revise or retire. A container in it frames everything inside.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame(BTreeSet<Target>);

impl Frame {
    /// The frame that names `targets`.
    pub fn new(targets: BTreeSet<Target>) -> Frame {
        Frame(targets)
    }

    /// The targets it names.
    pub fn targets(&self) -> &BTreeSet<Target> {
        &self.0
    }

    /// Whether it names nothing.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `target` is inside the frame: the frame names it, or names one of the containers it is in (`within`:
    /// the Project, the Context, the Contract, Interface or Harness, as they hold it).
    pub fn contains(&self, target: Target, within: &[Target]) -> bool {
        self.0
            .iter()
            .any(|named| target == *named || within.contains(named))
    }

    /// Whether `target` is inside the frame as [`contains`](Frame::contains) says, or is a Case (`cites` is what it
    /// cites) with a citation that is a target the frame names or inside one: the Cases that follow a framed Clause
    /// are the work of revising it.
    pub fn holds(&self, target: Target, within: &[Target], cites: Option<&Citations>) -> bool {
        self.contains(target, within)
            || cites.is_some_and(|c| self.0.iter().any(|named| c.is_inside(*named)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::id::{
        CaseId, ClauseId, ContextId, ContractId, HarnessId, ItemId, test_uuid,
    };

    fn context() -> Target {
        Target::Context(ContextId::from_uuid(test_uuid(1)))
    }

    fn contract() -> Target {
        Target::Contract(ContractId::from_uuid(test_uuid(2)))
    }

    fn clause(n: u128) -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(n)))
    }

    fn harness() -> Target {
        Target::Harness(HarnessId::from_uuid(test_uuid(3)))
    }

    fn case() -> Target {
        Target::Case(CaseId::from_uuid(test_uuid(4)))
    }

    fn item() -> Target {
        Target::Item(ItemId::from_uuid(test_uuid(5)))
    }

    #[test]
    fn a_frame_that_names_nothing_holds_nothing() {
        let none = Frame::default();
        assert!(none.is_empty());
        assert!(!none.contains(clause(10), &[Target::Project, context(), contract()]));
        assert!(!none.holds(
            case(),
            &[Target::Project, context(), harness()],
            Some(&Citations::new(
                vec![clause(10)],
                vec![Target::Project, context(), contract()]
            ))
        ));
    }

    #[test]
    fn a_target_is_inside_when_named_or_when_a_container_of_it_is() {
        let frame = Frame::new(BTreeSet::from([clause(10), contract()]));
        assert!(!frame.is_empty());
        assert_eq!(frame.targets().len(), 2);
        let within = [Target::Project, context(), contract()];
        assert!(frame.contains(clause(10), &within));
        assert!(frame.contains(clause(11), &within));
        assert!(frame.contains(contract(), &within[..2]));
        assert!(!frame.contains(clause(11), &[Target::Project, context()]));
        assert!(!frame.contains(context(), &[Target::Project]));
    }

    #[test]
    fn a_container_in_the_frame_frames_everything_in_it() {
        let frame = Frame::new(BTreeSet::from([context()]));
        assert!(frame.contains(clause(10), &[Target::Project, context(), contract()]));
        assert!(frame.contains(contract(), &[Target::Project, context()]));
        assert!(!frame.contains(Target::Project, &[]));
    }

    #[test]
    fn a_case_is_held_through_what_it_cites_and_contains_does_not_see_it() {
        let frame = Frame::new(BTreeSet::from([clause(10)]));
        let within = [Target::Project, context(), harness()];
        let cites = Citations::new(
            vec![item(), clause(10)],
            vec![Target::Project, context(), contract()],
        );
        assert!(!frame.contains(case(), &within));
        assert!(frame.holds(case(), &within, Some(&cites)));
        // a Case citing another Clause is not held, nor is one whose citations are not known
        let other = Citations::new(
            vec![clause(11)],
            vec![Target::Project, context(), contract()],
        );
        assert!(!frame.holds(case(), &within, Some(&other)));
        assert!(!frame.holds(case(), &within, None));
    }

    #[test]
    fn a_case_is_held_through_a_container_of_what_it_cites() {
        let frame = Frame::new(BTreeSet::from([contract()]));
        let within = [Target::Project, context(), harness()];
        let cites = Citations::new(
            vec![clause(10)],
            vec![Target::Project, context(), contract()],
        );
        assert!(frame.holds(case(), &within, Some(&cites)));
        // and through the Harness it is in, without citing anything
        let by_harness = Frame::new(BTreeSet::from([harness()]));
        assert!(by_harness.holds(case(), &within, None));
    }
}
