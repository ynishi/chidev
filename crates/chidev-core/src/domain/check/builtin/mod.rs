//! The checks that come with the code. Each is one struct with a `static` descriptor and reads the model through
//! the domain services that already decide it (`stale_cites`, `staleness`, `keep_moved`), so the rule is written
//! once. What must never be kept the Project already refuses (its invariants), and a check is for what may be kept
//! but should not be left. The one builtin that is mandatory is `sim.nondeterministic`: a simulation that cannot be
//! repeated finds nothing anyone can reproduce, so no policy turns it down.

mod change_requests;
mod citations;
mod contexts;
mod measure;
mod pins;
mod sim;

use super::Check;
use super::level::Hook;

pub use self::change_requests::{
    AwaitingDecision, IntentMismatch, KeepMoved, OutOfFrame, PolicyLineGone, ResolvedWithoutWork,
    TargetsUntouched,
};
pub use self::citations::{StaleCite, UncitedClause, UncitedItem};
pub use self::contexts::{NoHarness, NoInterface};
pub use self::measure::{MeasureFailed, Stale as StaleMutation, Survived, Unmeasured};
pub use self::pins::{StalePin, Unpinned};
pub use self::sim::{
    SimError, SimFinding, SimNondeterministic, SimShape, SimStale, SimUncitedClause, SimUnreached,
    SimUnrun,
};

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
        &AwaitingDecision,
        &KeepMoved,
        &OutOfFrame,
        &ResolvedWithoutWork,
        &TargetsUntouched,
        &IntentMismatch,
        &PolicyLineGone,
        &MeasureFailed,
        &Unmeasured,
        &StaleMutation,
        &Survived,
        &SimShape,
        &SimUncitedClause,
        &SimUnrun,
        &SimStale,
        &SimFinding,
        &SimUnreached,
        &SimNondeterministic,
        &SimError,
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
    use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft};
    use crate::domain::entity::context::Context;
    use crate::domain::entity::pin::Pin;
    use crate::domain::entity::project::Project;
    use crate::domain::service::change_request::expectations_now;
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
    use crate::domain::vo::target::{ExpectKind, Target};
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

    /// A change request without a Crux, expecting `wanted` of targets at their revisions now.
    pub(super) fn cr(
        p: &Project,
        n: u128,
        intent: Intent,
        parent: Option<u128>,
        wanted: &[(Target, ExpectKind)],
    ) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(n)),
            ChangeRequestDraft {
                title: Title::try_from(format!("cr {n}").as_str()).unwrap(),
                situation: None,
                intent,
                parent: parent.map(|k| ChangeRequestId::from_uuid(test_uuid(k))),
                issuer: None,
                expects: expectations_now(p, wanted.iter().copied()),
                crux: None,
                assumptions: Vec::new(),
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
                measurements: &[],
                measure_settings: &BTreeMap::new(),
                sim_settings: &BTreeMap::new(),
            },
            &Params::default(),
        )
    }

    /// Revises the Harness (the one the project has): its code changes, so its revision does.
    pub(super) fn revise_harness(p: &mut Project) {
        p.put_harness(
            ctx(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new("fn t() { /* changed */ }\n".to_string()),
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
            assert_eq!(
                d.mandatory,
                d.id.as_str() == "sim.nondeterministic",
                "{} is mandatory only if it is a repeatability rule",
                d.id
            );
            assert_eq!(d.executor, Executor::Builtin);
            assert!(d.hooks.contains(&Hook::Check), "{} runs on demand", d.id);
            assert!(!d.summary.is_empty() && !d.help.is_empty());
            let note = matches!(
                d.id.as_str(),
                "pin.unpinned"
                    | "context.no-interface"
                    | "context.no-harness"
                    | "cr.awaiting-decision"
                    | "cr.intent-mismatch"
                    | "cr.targets-untouched"
                    | "crux.policy-line-gone"
                    | "mutation.unmeasured"
                    | "mutation.survived"
            );
            let error = matches!(
                d.id.as_str(),
                "sim.shape" | "sim.finding" | "sim.unreached" | "sim.nondeterministic"
            );
            assert_eq!(
                d.default_level,
                if error {
                    Level::Error
                } else if note {
                    Level::Note
                } else {
                    Level::Warning
                },
                "{}",
                d.id
            );
        }
    }

    use crate::domain::vo::finding::Subject;
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::resolution::{Resolution, ResolutionKind};
    use crate::domain::vo::target::{ExpectKind, Target};

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
        // the message does not count what moved; it gives the things for the use case to name
        assert_eq!(
            found[0].mentions(),
            [crate::domain::check::finding::Mention::Interface(iface())]
        );
    }

    #[test]
    fn a_pin_of_a_retired_interface_is_not_stale_but_one_of_a_vanished_interface_is() {
        use crate::domain::entity::project::Project;
        use crate::domain::vo::id::{HarnessId, ProjectId, test_uuid};

        let mut p = project();
        let pins = [pin(&p)];
        // the Case that cites the Interface goes first, then the Interface
        p.retire_harness(ctx(), HarnessId::from_uuid(test_uuid(400)))
            .unwrap();
        p.retire_interface(ctx(), iface()).unwrap();
        assert!(run(&StalePin, &p, &pins, &[]).is_empty());
        // gone without being retired: reported as before
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(
            subjects(&run(&StalePin, &empty, &pins, &[])),
            [Some(Subject::Interface(iface()))]
        );
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

    /// Resolves `c` as done by a person, against the change requests `all`.
    fn resolve_as(
        c: &mut crate::domain::entity::change_request::ChangeRequest,
        kind: ResolutionKind,
        all: &[crate::domain::entity::change_request::ChangeRequest],
    ) {
        use crate::domain::service::change_request::resolve;
        use crate::domain::vo::answer::DecisionMaker;
        use crate::domain::vo::prose::Prose;
        use crate::domain::vo::timestamp::Timestamp;
        resolve(
            c,
            Resolution::new(
                kind,
                Prose::try_from("settled").unwrap(),
                DecisionMaker::person("alice").unwrap(),
                Timestamp::from_millis(3),
            ),
            all,
        )
        .unwrap();
    }

    #[test]
    fn a_request_resolved_as_done_with_no_work_is_found_and_one_with_work_is_not() {
        let mut p = project();
        let target = [(Target::Clause(clause(30)), ExpectKind::Target)];
        let mut bare = cr(&p, 1, Intent::ChangeBehavior, None, &target);
        let mut worked = cr(&p, 2, Intent::ChangeBehavior, None, &target);
        let mut dropped = cr(&p, 3, Intent::ChangeBehavior, None, &target);
        let open = cr(&p, 4, Intent::ChangeBehavior, None, &target);
        applied_for(&mut worked, &mut p, |p| {
            put_contract(p, "a half rounds to even")
        });
        resolve_as(&mut bare, ResolutionKind::Done, &[]);
        resolve_as(&mut worked, ResolutionKind::Done, &[]);
        resolve_as(&mut dropped, ResolutionKind::WontDo, &[]);
        let crs = [bare.clone(), worked, dropped, open];
        let found = run(&ResolvedWithoutWork, &p, &[], &crs);
        // The one resolved as done with no work; not the one with work, the one not done, or the one open.
        assert_eq!(subjects(&found), [Some(Subject::ChangeRequest(bare.id()))]);
        assert_eq!(found[0].level(), crate::domain::check::Level::Warning);
    }

    #[test]
    fn a_request_resolved_as_done_with_a_target_its_work_never_touched_is_noted() {
        let mut p = project();
        let both = [
            (Target::Clause(clause(30)), ExpectKind::Target),
            (Target::Item(item(10)), ExpectKind::Target),
        ];
        let mut c = cr(&p, 1, Intent::ChangeBehavior, None, &both);
        applied_for(&mut c, &mut p, |p| put_contract(p, "a half rounds to even"));
        // Open: nothing yet, and the item is still untouched; resolve does not ask.
        assert!(run(&TargetsUntouched, &p, &[], &[c.clone()]).is_empty());
        resolve_as(&mut c, ResolutionKind::Done, &[]);
        let found = run(&TargetsUntouched, &p, &[], &[c.clone()]);
        assert_eq!(subjects(&found), [Some(Subject::ChangeRequest(c.id()))]);
        assert_eq!(found[0].level(), crate::domain::check::Level::Note);
        assert!(run(&ResolvedWithoutWork, &p, &[], &[c]).is_empty());
    }

    #[test]
    fn a_parent_resolved_as_done_counts_the_work_of_its_parts() {
        let mut p = project();
        let mut parent = cr(&p, 1, Intent::ChangeBehavior, None, &[]);
        let mut part = cr(
            &p,
            2,
            Intent::ChangeBehavior,
            Some(1),
            &[(Target::Clause(clause(30)), ExpectKind::Target)],
        );
        applied_for(&mut part, &mut p, |p| {
            put_contract(p, "a half rounds to even")
        });
        // A parent that names no target is not a mismatch of intent while its part does.
        assert!(run(&IntentMismatch, &p, &[], &[parent.clone(), part.clone()]).is_empty());
        let all = [parent.clone(), part.clone()];
        resolve_as(&mut part, ResolutionKind::Done, &all);
        let all = [parent.clone(), part.clone()];
        resolve_as(&mut parent, ResolutionKind::Done, &all);
        let crs = [parent, part];
        assert!(run(&ResolvedWithoutWork, &p, &[], &crs).is_empty());
        assert!(run(&TargetsUntouched, &p, &[], &crs).is_empty());
    }

    /// `change` is made to `p` as a change set applied for `c`.
    fn applied_for(
        c: &mut crate::domain::entity::change_request::ChangeRequest,
        p: &mut crate::domain::entity::project::Project,
        change: impl FnOnce(&mut crate::domain::entity::project::Project),
    ) {
        use crate::domain::service::footprint::moves_between;
        use crate::domain::vo::footprint::Footprint;

        let before = p.clone();
        change(p);
        c.record_work(Footprint::new(2, moves_between(&before, p)))
            .unwrap();
    }

    #[test]
    fn work_outside_the_frame_of_a_change_request_is_found() {
        let mut p = project();
        let mut c = cr(
            &p,
            1,
            Intent::ChangeBehavior,
            None,
            &[(Target::Clause(clause(30)), ExpectKind::Target)],
        );
        applied_for(&mut c, &mut p, |p| put_contract(p, "a half rounds to even"));
        let mut crs = [c];
        assert!(run(&OutOfFrame, &p, &[], &crs).is_empty());
        applied_for(&mut crs[0], &mut p, |p| put_interface(p, "fn a(x: u8)"));
        let found = run(&OutOfFrame, &p, &[], &crs);
        assert_eq!(
            subjects(&found),
            [Some(Subject::ChangeRequest(crs[0].id()))]
        );
        assert_eq!(found[0].level(), crate::domain::check::Level::Warning);
    }

    #[test]
    fn a_change_request_whose_question_is_not_decided_is_found_until_it_is_decided() {
        use std::collections::BTreeMap;

        use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft, Crux};
        use crate::domain::vo::answer::{Answer, Basis, DecisionMaker};
        use crate::domain::vo::choice::Choice;
        use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned};
        use crate::domain::vo::id::{ChangeRequestId, test_uuid};
        use crate::domain::vo::prose::Prose;
        use crate::domain::vo::timestamp::Timestamp;
        use crate::domain::vo::title::Title;
        let option =
            |c: &str| CruxOption::new(Choice::try_from(c).unwrap(), None, BTreeMap::new(), None);
        let crux = Crux::new(
            CruxKind::Conflict,
            vec![option("a"), option("b")],
            None,
            None,
            None,
        )
        .unwrap();
        let p = project();
        let plain = cr(&p, 1, Intent::ChangeBehavior, None, &[]);
        let mut asking = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(2)),
            ChangeRequestDraft {
                title: Title::try_from("asks").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: BTreeMap::new(),
                crux: Some(crux),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let found = run(&AwaitingDecision, &p, &[], &[plain.clone(), asking.clone()]);
        assert_eq!(
            subjects(&found),
            [Some(Subject::ChangeRequest(asking.id()))]
        );
        assert_eq!(found[0].level(), crate::domain::check::Level::Note);
        let choice = Decision::Chosen(Reasoned::new(Choice::try_from("a").unwrap(), None));
        let answer = Answer::new(
            choice,
            Basis::Arbitrary {
                reason: Prose::try_from("nothing derives it").unwrap(),
            },
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(1),
        );
        asking.decide(answer, &BTreeMap::new()).unwrap();
        assert!(run(&AwaitingDecision, &p, &[], &[plain, asking]).is_empty());
    }

    #[test]
    fn a_decision_on_a_policy_line_that_is_gone_or_reworded_is_noted() {
        use std::collections::BTreeMap;

        use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft, Crux};
        use crate::domain::vo::answer::{Answer, Basis, DecisionMaker};
        use crate::domain::vo::choice::Choice;
        use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned};
        use crate::domain::vo::id::{ChangeRequestId, test_uuid};
        use crate::domain::vo::policy::PolicyLineId;
        use crate::domain::vo::prose::Prose;
        use crate::domain::vo::timestamp::Timestamp;
        use crate::domain::vo::title::Title;
        let line = |n: &str, t: &str| {
            (
                PolicyLineId::try_from(n).unwrap(),
                Prose::try_from(t).unwrap(),
            )
        };
        let mut p = project();
        p.put_policy(vec![
            line("P1", "take it from 80"),
            line("P2", "low: either"),
        ])
        .unwrap();
        let answered = |n: u128, title: &str, on: Basis| {
            let crux = Crux::new(
                CruxKind::Ambiguity,
                vec![CruxOption::new(
                    Choice::try_from("a").unwrap(),
                    None,
                    BTreeMap::new(),
                    None,
                )],
                None,
                None,
                None,
            )
            .unwrap();
            let mut c = ChangeRequest::open(
                ChangeRequestId::from_uuid(test_uuid(n)),
                ChangeRequestDraft {
                    title: Title::try_from(title).unwrap(),
                    situation: None,
                    intent: Intent::ChangeBehavior,
                    parent: None,
                    issuer: None,
                    expects: BTreeMap::new(),
                    crux: Some(crux),
                    assumptions: Vec::new(),
                    addresses: Vec::new(),
                },
            );
            let answer = Answer::new(
                Decision::Chosen(Reasoned::new(Choice::try_from("a").unwrap(), None)),
                on,
                DecisionMaker::role("orchestrator").unwrap(),
                Timestamp::from_millis(1),
            );
            c.decide(answer, &BTreeMap::new()).unwrap();
            c
        };
        let on = |n: &str| {
            p.policy()
                .basis_on(PolicyLineId::try_from(n).unwrap())
                .unwrap()
        };
        let on_p1 = answered(1, "on p1", on("P1"));
        let on_p2 = answered(2, "on p2", on("P2"));
        let arbitrary = answered(
            3,
            "arbitrary",
            Basis::Arbitrary {
                reason: Prose::try_from("the product leans this way").unwrap(),
            },
        );
        let crs = [on_p1.clone(), on_p2.clone(), arbitrary];
        // Nothing moved.
        assert!(run(&PolicyLineGone, &p, &[], &crs).is_empty());
        // P1 is reworded, P2 is dropped: both are found, the arbitrary one never is.
        p.put_policy(vec![line("P1", "take it from 90")]).unwrap();
        let found = run(&PolicyLineGone, &p, &[], &crs);
        assert_eq!(
            subjects(&found),
            [
                Some(Subject::ChangeRequest(on_p1.id())),
                Some(Subject::ChangeRequest(on_p2.id()))
            ]
        );
        assert_eq!(found[0].level(), crate::domain::check::Level::Note);
        // A line left as it was, while others change, is not found.
        let mut q = project();
        q.put_policy(vec![
            line("P1", "take it from 80"),
            line("P2", "low: either"),
        ])
        .unwrap();
        let kept = [answered(
            1,
            "on p1",
            q.policy()
                .basis_on(PolicyLineId::try_from("P1").unwrap())
                .unwrap(),
        )];
        q.put_policy(vec![line("P1", "take it from 80"), line("P3", "new")])
            .unwrap();
        assert!(run(&PolicyLineGone, &q, &[], &kept).is_empty());
        // Written again after it was dropped, it is a new text.
        q.put_policy(vec![line("P3", "new")]).unwrap();
        q.put_policy(vec![line("P1", "take it from 80")]).unwrap();
        assert_eq!(run(&PolicyLineGone, &q, &[], &kept).len(), 1);
    }

    #[test]
    fn a_change_request_is_found_when_a_target_it_keeps_moves() {
        let mut p = project();
        let keep = |p: &crate::domain::entity::project::Project, n| {
            cr(
                p,
                n,
                Intent::KeepBehavior,
                None,
                &[(Target::Clause(clause(30)), ExpectKind::Keep)],
            )
        };
        let waiting = keep(&p, 1);
        let change = cr(
            &p,
            2,
            Intent::ChangeBehavior,
            None,
            &[(Target::Clause(clause(30)), ExpectKind::Target)],
        );
        let mut done = keep(&p, 3);
        resolve_as(&mut done, ResolutionKind::Done, &[]);
        let crs = [waiting.clone(), change, done];
        assert!(run(&KeepMoved, &p, &[], &crs).is_empty());
        put_contract(&mut p, "a half rounds to even");
        let found = run(&KeepMoved, &p, &[], &crs);
        // Only the unresolved one that keeps: not the one that names the clause as a target, not the record.
        assert_eq!(
            subjects(&found),
            [Some(Subject::ChangeRequest(waiting.id()))]
        );
        assert_eq!(found[0].level(), crate::domain::check::Level::Warning);
    }

    #[test]
    fn a_keep_behavior_request_that_keeps_a_clause_and_changes_an_item_raises_nothing() {
        // The model problem this replaces: such a request resolves, and neither check has anything to say.
        let mut p = project();
        let c = cr(
            &p,
            1,
            Intent::KeepBehavior,
            None,
            &[
                (Target::Clause(clause(30)), ExpectKind::Keep),
                (Target::Item(item(10)), ExpectKind::Target),
            ],
        );
        let mut c = c;
        applied_for(&mut c, &mut p, |p| put_interface(p, "fn a(x: u8)"));
        resolve_as(&mut c, ResolutionKind::Done, &[]);
        let crs = [c];
        assert!(run(&KeepMoved, &p, &[], &crs).is_empty());
        assert!(run(&IntentMismatch, &p, &[], &crs).is_empty());
        assert!(run(&ResolvedWithoutWork, &p, &[], &crs).is_empty());
        assert!(run(&TargetsUntouched, &p, &[], &crs).is_empty());
    }

    #[test]
    fn intent_and_targets_that_disagree_are_noted() {
        use std::collections::BTreeMap;

        use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft, Crux};
        use crate::domain::vo::choice::Choice;
        use crate::domain::vo::crux::{CruxKind, CruxOption};
        use crate::domain::vo::id::{ChangeRequestId, test_uuid};
        use crate::domain::vo::title::Title;
        let p = project();
        let clause_change = [(Target::Clause(clause(30)), ExpectKind::Target)];
        let keeps_but_changes_a_clause = cr(&p, 1, Intent::KeepBehavior, None, &clause_change);
        let keeps_but_removes_a_clause = cr(
            &p,
            2,
            Intent::KeepBehavior,
            None,
            &[(Target::Clause(clause(31)), ExpectKind::Target)],
        );
        let keeps_and_changes_an_item = cr(
            &p,
            3,
            Intent::KeepBehavior,
            None,
            &[(Target::Item(item(10)), ExpectKind::Target)],
        );
        let changes_nothing = cr(
            &p,
            4,
            Intent::ChangeBehavior,
            None,
            &[(Target::Clause(clause(30)), ExpectKind::Keep)],
        );
        let changes_a_clause = cr(&p, 5, Intent::ChangeBehavior, None, &clause_change);
        // Nothing is named in itself, but an option names a target.
        let crux = Crux::new(
            CruxKind::Conflict,
            vec![CruxOption::new(
                Choice::try_from("drop").unwrap(),
                None,
                BTreeMap::from([(Target::Clause(clause(31)), ExpectKind::Target)]),
                None,
            )],
            None,
            None,
            None,
        )
        .unwrap();
        let option_removes = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(6)),
            ChangeRequestDraft {
                title: Title::try_from("asks").unwrap(),
                situation: None,
                intent: Intent::ChangeBehavior,
                parent: None,
                issuer: None,
                expects: BTreeMap::new(),
                crux: Some(crux),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let mut resolved = cr(&p, 7, Intent::KeepBehavior, None, &clause_change);
        resolve_as(&mut resolved, ResolutionKind::Done, &[]);
        let crs = [
            keeps_but_changes_a_clause.clone(),
            keeps_but_removes_a_clause.clone(),
            keeps_and_changes_an_item,
            changes_nothing.clone(),
            changes_a_clause,
            option_removes,
            resolved,
        ];
        let found = run(&IntentMismatch, &p, &[], &crs);
        assert_eq!(
            subjects(&found),
            [
                Some(Subject::ChangeRequest(keeps_but_changes_a_clause.id())),
                Some(Subject::ChangeRequest(keeps_but_removes_a_clause.id())),
                Some(Subject::ChangeRequest(changes_nothing.id())),
            ]
        );
        assert_eq!(found[0].level(), crate::domain::check::Level::Note);
    }
}
