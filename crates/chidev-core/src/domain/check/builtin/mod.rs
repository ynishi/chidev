//! The checks that come with the code. Each is one struct with a `static` descriptor and reads the model through
//! the domain services that already decide it (`stale_cites`, `staleness`, `unapplied`), so the rule is written
//! once. None is mandatory: what must never be kept the Project already refuses (its invariants), and a check is
//! for what may be kept but should not be left.

mod change_requests;
mod citations;
mod contexts;
mod pins;

use super::Check;
use super::level::Hook;

pub use self::change_requests::{KeepBehaviorChanged, ParentResolvedEarly, Unapplied};
pub use self::citations::{StaleCite, UncitedClause, UncitedItem};
pub use self::contexts::{NoHarness, NoInterface};
pub use self::pins::{StalePin, Unpinned};

/// Every builtin check.
pub fn all() -> &'static [&'static dyn Check] {
    &[
        &StaleCite,
        &StalePin,
        &Unpinned,
        &UncitedClause,
        &UncitedItem,
        &NoInterface,
        &NoHarness,
        &Unapplied,
        &KeepBehaviorChanged,
        &ParentResolvedEarly,
    ]
}

/// The hooks of the checks that read the whole model, not only what a change touches.
const ALL_HOOKS: &[Hook] = &[Hook::Put, Hook::Pin, Hook::Resolve, Hook::Check];

#[cfg(test)]
mod fixture {
    //! A small Project to run the builtin checks on, and the ways to move it.

    use std::collections::{BTreeMap, BTreeSet};

    use crate::domain::check::policy::Params;
    use crate::domain::check::{Check, CheckView, Finding};
    use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft, Route};
    use crate::domain::entity::context::Context;
    use crate::domain::entity::pin::Pin;
    use crate::domain::entity::project::Project;
    use crate::domain::service::change_request::revisions_now;
    use crate::domain::service::pin::record;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::{
        CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
        PinId, ProjectId, test_uuid,
    };
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::signature::Signature;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::target::Target;
    use crate::domain::vo::title::Title;

    pub(super) fn ctx() -> ContextId {
        ContextId::from_uuid(test_uuid(1))
    }

    pub(super) fn empty_ctx() -> ContextId {
        ContextId::from_uuid(test_uuid(2))
    }

    pub(super) fn iface() -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(100))
    }

    pub(super) fn item(n: u128) -> ItemId {
        ItemId::from_uuid(test_uuid(n))
    }

    pub(super) fn clause(n: u128) -> ClauseId {
        ClauseId::from_uuid(test_uuid(n))
    }

    /// Context `core` with Contract (C1 = 30, C2 = 31), Interface 100 (items 10 and 11) and a Harness whose one Case
    /// cites item 10 and C1; and Context `empty`, which has nothing.
    pub(super) fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        for (id, name) in [(ctx(), "core"), (empty_ctx(), "empty")] {
            p.add_context(
                Context::new(id, ContextName::try_from(name).unwrap(), BTreeSet::new()).unwrap(),
            )
            .unwrap();
        }
        put_contract(&mut p, "a half rounds up");
        put_interface(&mut p, "fn a()");
        p.put_harness(
            ctx(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new("fn t() {}\n".to_string()),
            vec![(
                CaseId::from_uuid(test_uuid(40)),
                CaseName::try_from("t").unwrap(),
                LineRange::new(1, 1).unwrap(),
                CaseCites::new(
                    BTreeMap::from([(item(10), Revision::first())]),
                    BTreeMap::from([(clause(30), Revision::first())]),
                )
                .unwrap(),
            )],
        )
        .unwrap();
        p
    }

    /// Revises Clause 30's text (and keeps Clause 31).
    pub(super) fn put_contract(p: &mut Project, text: &str) {
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
            vec![c(30, "C1", text), c(31, "C2", "an empty total is zero")],
        )
        .unwrap();
    }

    /// Revises the Interface: item 10 with signature `first`, item 11 `fn b()`.
    pub(super) fn put_interface(p: &mut Project, first: &str) {
        p.put_interface(
            ctx(),
            iface(),
            Source::new(format!("{first}\nfn b()\n")),
            vec![
                (
                    item(10),
                    Signature::try_from(first).unwrap(),
                    LineRange::new(1, 1).unwrap(),
                ),
                (
                    item(11),
                    Signature::try_from("fn b()").unwrap(),
                    LineRange::new(2, 2).unwrap(),
                ),
            ],
        )
        .unwrap();
    }

    pub(super) fn pin(p: &Project) -> Pin {
        record(
            p,
            iface(),
            PinId::from_uuid(test_uuid(50)),
            Digest::of(b"code"),
        )
        .unwrap()
    }

    /// A change request resolved by the machine, about `targets` at their revisions now.
    pub(super) fn cr(
        p: &Project,
        n: u128,
        intent: Intent,
        parent: Option<u128>,
        targets: &[Target],
    ) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(n)),
            ChangeRequestDraft {
                title: Title::try_from(format!("cr {n}").as_str()).unwrap(),
                situation: None,
                intent,
                parent: parent.map(|k| ChangeRequestId::from_uuid(test_uuid(k))),
                issuer: None,
                lines: revisions_now(p, targets.iter().copied()),
                route: Route::Machine,
                addresses: Vec::new(),
            },
        )
    }

    /// What `check` finds in `p` with these Pins and change requests.
    pub(super) fn run(
        check: &dyn Check,
        p: &Project,
        pins: &[Pin],
        crs: &[ChangeRequest],
    ) -> Vec<Finding> {
        check.run(
            &CheckView {
                before: None,
                after: p,
                pins,
                change_requests: crs,
            },
            &Params::default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::fixture::*;
    use super::*;
    use crate::domain::check::descriptor::Executor;
    use crate::domain::check::level::Level;

    #[test]
    fn the_builtins_have_distinct_ids_and_the_levels_that_were_decided() {
        let all = all();
        let ids: BTreeSet<&str> = all.iter().map(|c| c.descriptor().id.as_str()).collect();
        assert_eq!(ids.len(), all.len());
        for c in all {
            let d = c.descriptor();
            assert!(!d.mandatory, "{} is not mandatory yet", d.id);
            assert_eq!(d.executor, Executor::Builtin);
            assert!(d.hooks.contains(&Hook::Check), "{} runs on demand", d.id);
            assert!(!d.summary.is_empty() && !d.help.is_empty());
            let note = matches!(
                d.id.as_str(),
                "pin.unpinned" | "context.no-interface" | "context.no-harness"
            );
            assert_eq!(
                d.default_level,
                if note { Level::Note } else { Level::Warning },
                "{}",
                d.id
            );
        }
    }

    use crate::domain::vo::finding::Subject;
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::target::Target;

    fn subjects(found: &[crate::domain::check::Finding]) -> Vec<Option<Subject>> {
        found.iter().map(|f| f.subject()).collect()
    }

    #[test]
    fn a_clause_or_item_no_case_cites_is_found() {
        let p = project();
        assert_eq!(
            subjects(&run(&UncitedClause, &p, &[], &[])),
            [Some(Subject::Clause(clause(31)))]
        );
        assert_eq!(
            subjects(&run(&UncitedItem, &p, &[], &[])),
            [Some(Subject::Item(item(11)))]
        );
    }

    #[test]
    fn a_context_without_an_interface_or_a_harness_is_found() {
        let p = project();
        let empty = [Some(Subject::Context(empty_ctx()))];
        assert_eq!(subjects(&run(&NoInterface, &p, &[], &[])), empty);
        assert_eq!(subjects(&run(&NoHarness, &p, &[], &[])), empty);
    }

    #[test]
    fn an_interface_with_no_pin_is_found_until_it_is_pinned() {
        let p = project();
        assert_eq!(
            subjects(&run(&Unpinned, &p, &[], &[])),
            [Some(Subject::Interface(iface()))]
        );
        assert!(run(&Unpinned, &p, &[pin(&p)], &[]).is_empty());
    }

    #[test]
    fn a_pin_is_stale_once_what_it_stands_on_moves() {
        let mut p = project();
        let pins = [pin(&p)];
        assert!(run(&StalePin, &p, &pins, &[]).is_empty());
        put_interface(&mut p, "fn a(x: u8)");
        let found = run(&StalePin, &p, &pins, &[]);
        assert_eq!(subjects(&found), [Some(Subject::Interface(iface()))]);
        assert!(found[0].message().contains("1 interface change"));
    }

    #[test]
    fn a_case_is_found_once_when_what_it_cites_moves() {
        let mut p = project();
        assert!(run(&StaleCite, &p, &[], &[]).is_empty());
        put_contract(&mut p, "a half rounds to even");
        put_interface(&mut p, "fn a(x: u8)");
        let found = run(&StaleCite, &p, &[], &[]);
        assert_eq!(found.len(), 1);
        assert!(matches!(found[0].subject(), Some(Subject::Case(_))));
    }

    #[test]
    fn a_change_request_the_definitions_have_not_followed_is_found() {
        let mut p = project();
        let c = cr(
            &p,
            1,
            Intent::ChangeBehavior,
            None,
            &[Target::Clause(clause(30))],
        );
        let crs = [c];
        assert_eq!(run(&Unapplied, &p, &[], &crs).len(), 1);
        put_contract(&mut p, "a half rounds to even");
        assert!(run(&Unapplied, &p, &[], &crs).is_empty());
    }

    #[test]
    fn a_change_request_that_keeps_the_behaviour_is_found_when_its_line_moves() {
        let mut p = project();
        let keep = cr(
            &p,
            1,
            Intent::KeepBehavior,
            None,
            &[Target::Clause(clause(30))],
        );
        let change = cr(
            &p,
            2,
            Intent::ChangeBehavior,
            None,
            &[Target::Clause(clause(30))],
        );
        let mut done = cr(
            &p,
            3,
            Intent::KeepBehavior,
            None,
            &[Target::Clause(clause(30))],
        );
        done.resolve().unwrap();
        let crs = [keep.clone(), change, done];
        assert!(run(&KeepBehaviorChanged, &p, &[], &crs).is_empty());
        put_contract(&mut p, "a half rounds to even");
        let found = run(&KeepBehaviorChanged, &p, &[], &crs);
        // Only the unresolved one that promised to keep: not the one that asked for a change, not the record.
        assert_eq!(subjects(&found), [Some(Subject::ChangeRequest(keep.id()))]);
    }

    #[test]
    fn a_parent_resolved_before_its_part_is_found() {
        let p = project();
        let mut parent = cr(&p, 1, Intent::ChangeBehavior, None, &[]);
        parent.resolve().unwrap();
        let part = cr(&p, 2, Intent::ChangeBehavior, Some(1), &[]);
        let crs = [parent.clone(), part.clone()];
        assert_eq!(
            subjects(&run(&ParentResolvedEarly, &p, &[], &crs)),
            [Some(Subject::ChangeRequest(parent.id()))]
        );
        let mut part = part;
        part.resolve().unwrap();
        assert!(run(&ParentResolvedEarly, &p, &[], &[parent, part]).is_empty());
    }
}
