//! The Harness of examples read against the model: which Harnesses of a Context hold examples, and what a run of
//! one stands on. Whether a Harness of examples has the shape a run needs (a Case cites a Clause, builds at least one
//! Interface) is for the Harness to say, and a put of one that has not is refused.

use crate::domain::entity::context::Context;
use crate::domain::entity::harness::Harness;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::examples_run::ExamplesBasis;
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::HarnessId;

/// The Harnesses of `context` that hold examples, in the order of their ids.
pub fn examples_harnesses(context: &Context) -> impl Iterator<Item = &Harness> {
    context
        .harnesses()
        .filter(|h| h.kind() == HarnessKind::Examples)
}

/// What a run of the examples of the Harness `harness` stands on now: its revision (the examples are in it), and the
/// digest of the code pinned in `pins` for each Interface it builds. `None` when no Harness of examples has this id.
/// An Interface the model has not any more is left out: the Harness no longer builds a thing that is there.
pub fn basis_of(project: &Project, harness: HarnessId, pins: &[Pin]) -> Option<ExamplesBasis> {
    let h = project
        .contexts()
        .find_map(|c| c.harness(harness))
        .filter(|h| h.kind() == HarnessKind::Examples)?;
    let mut builds = std::collections::BTreeMap::new();
    for id in h.builds() {
        let id = *id;
        if project.contexts().any(|c| c.interface(id).is_some()) {
            let pin = pins.iter().find(|p| p.interface() == id).map(Pin::code);
            builds.insert(id, pin);
        }
    }
    Some(ExamplesBasis {
        harness: h.revision(),
        builds,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::domain::entity::harness::ExampleFound;
    use crate::domain::service::pin::record;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::example::Example;
    use crate::domain::vo::id::{
        CaseId, ClauseId, ContextId, ContractId, InterfaceId, ItemId, PinId, ProjectId, test_uuid,
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

    fn iface() -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(100))
    }

    fn hid() -> HarnessId {
        HarnessId::from_uuid(test_uuid(500))
    }

    /// A Context with Contract 300 (Clause 30 = C1) and Interface 100 (Item 10).
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
        p.put_contract(
            ctx(),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![(
                ClauseId::from_uuid(test_uuid(30)),
                ClauseContent::new(
                    ClauseLabel::try_from("C1").unwrap(),
                    ClauseText::try_from("one").unwrap(),
                ),
            )],
        )
        .unwrap();
        p.put_interface(
            ctx(),
            iface(),
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

    fn example(n: u128, name: &str, expected: &str) -> ExampleFound {
        (
            CaseId::from_uuid(test_uuid(n)),
            CaseName::try_from(name).unwrap(),
            Example::new(String::new(), String::new(), expected.to_string(), None).unwrap(),
            CaseCites::of(
                BTreeMap::new(),
                BTreeMap::from([(ClauseId::from_uuid(test_uuid(30)), Revision::first())]),
            ),
        )
    }

    fn put_examples(p: &mut Project, expected: &str) {
        p.put_examples_harness(
            ctx(),
            hid(),
            BTreeSet::from([iface()]),
            vec![example(50, "a1", expected)],
        )
        .unwrap();
    }

    #[test]
    fn a_run_stands_on_the_harness_and_the_code_pinned_for_what_it_builds() {
        let mut p = project();
        put_examples(&mut p, "#exit 0\n");
        let none = basis_of(&p, hid(), &[]).unwrap();
        assert_eq!(
            none.harness,
            p.context(ctx()).unwrap().harness(hid()).unwrap().revision()
        );
        assert_eq!(none.builds, BTreeMap::from([(iface(), None)]));
        // the Interface is pinned: its code is on the basis, and no Contract or Interface revision is
        p.put_harness(
            ctx(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new("fn t() {}\n".to_string()),
            vec![(
                CaseId::from_uuid(test_uuid(40)),
                CaseName::try_from("t").unwrap(),
                LineRange::new(1, 1).unwrap(),
                CaseCites::new(
                    BTreeMap::from([(ItemId::from_uuid(test_uuid(10)), Revision::first())]),
                    BTreeMap::new(),
                )
                .unwrap(),
            )],
        )
        .unwrap();
        let pin = record(
            &p,
            iface(),
            PinId::from_uuid(test_uuid(60)),
            Digest::of(b"code"),
        )
        .unwrap();
        let pinned = basis_of(&p, hid(), &[pin]).unwrap();
        assert_eq!(
            pinned.builds,
            BTreeMap::from([(iface(), Some(Digest::of(b"code")))])
        );
        assert_eq!(pinned.harness, none.harness);
    }

    #[test]
    fn a_changed_example_is_another_revision_of_the_harness() {
        let mut p = project();
        put_examples(&mut p, "#exit 0\n");
        let before = basis_of(&p, hid(), &[]).unwrap();
        put_examples(&mut p, "1\n#exit 0\n");
        let after = basis_of(&p, hid(), &[]).unwrap();
        assert_ne!(before.harness, after.harness);
    }

    #[test]
    fn a_harness_that_does_not_hold_examples_has_no_basis() {
        let mut p = project();
        assert_eq!(basis_of(&p, hid(), &[]), None);
        p.put_harness(
            ctx(),
            hid(),
            Source::new("fn t() {}\n".to_string()),
            vec![(
                CaseId::from_uuid(test_uuid(40)),
                CaseName::try_from("t").unwrap(),
                LineRange::new(1, 1).unwrap(),
                CaseCites::new(
                    BTreeMap::from([(ItemId::from_uuid(test_uuid(10)), Revision::first())]),
                    BTreeMap::new(),
                )
                .unwrap(),
            )],
        )
        .unwrap();
        assert_eq!(basis_of(&p, hid(), &[]), None);
        assert_eq!(examples_harnesses(p.context(ctx()).unwrap()).count(), 0);
        put_examples(&mut p, "#exit 0\n");
        assert_eq!(examples_harnesses(p.context(ctx()).unwrap()).count(), 1);
    }
}
