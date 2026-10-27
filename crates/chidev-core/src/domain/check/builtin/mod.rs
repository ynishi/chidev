//! The checks that come with the code. Each is one struct with a `static` descriptor and reads the model through
//! the domain services that already decide it (`stale_cites`, `staleness`, `keep_moved`), so the rule is written
//! once. What must never be kept the Project already refuses (its invariants), and a check is for what may be kept
//! but should not be left. The one builtin that is mandatory is `sim.nondeterministic`: a simulation that cannot be
//! repeated finds nothing anyone can reproduce, so no policy turns it down.
//!
//! Each descriptor lists its moments, and the moments say what its check speaks about ([`Descriptor::kind`]). A check
//! that runs at an edit speaks about what an edit made stale or out of place: only what the edit introduced is
//! reported, and it fills the grounds of its findings, which are what tell one edit's finding from the last one's.
//! A check that does not speaks about the state of the model (something is not there, not settled or not measured)
//! and is judged where something is confirmed or asked for. `check_list` shows the kind and the moments of each rule.
//!
//! [`Descriptor::kind`]: super::descriptor::Descriptor::kind

mod change_requests;
mod citations;
mod contexts;
mod measure;
mod pins;
mod sim;

use super::Check;
use super::level::Moment;

pub use self::change_requests::{
    AwaitingDecision, IntentMismatch, KeepMoved, OutOfFrame, PolicyLineGone, ResolvedWithoutWork,
    TargetsUntouched,
};
pub use self::citations::{StaleCite, UncitedClause, UncitedItem};
pub use self::contexts::{NoHarness, NoInterface};
pub use self::measure::{Capped, MeasureFailed, Stale as StaleMutation, Survived, Unmeasured};
pub use self::pins::{StalePin, Unpinned};
pub use self::sim::{
    SimError, SimFinding, SimNondeterministic, SimShapeKept, SimStale, SimUncitedClause,
    SimUnreached, SimUnrun,
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
        &Capped,
        &Survived,
        &SimShapeKept,
        &SimUncitedClause,
        &SimUnrun,
        &SimStale,
        &SimFinding,
        &SimUnreached,
        &SimNondeterministic,
        &SimError,
    ]
}

/// The moments of a check that speaks about the state of the model (its kind is [`CheckKind::State`], which it is
/// because an edit is not among its moments): the ones that confirm something or ask for the whole model. It does
/// not run at an edit, where the model is not finished and would be told so every time.
///
/// [`CheckKind::State`]: super::descriptor::CheckKind::State
const CONFIRM_MOMENTS: &[Moment] = &[Moment::Pin, Moment::Resolve, Moment::Check];

#[cfg(test)]
mod fixture {
    //! A small Project to run the builtin checks on, and the ways to move it.

    use std::collections::{BTreeMap, BTreeSet};

    use crate::domain::check::policy::Params;
    use crate::domain::check::{Check, CheckView, Finding};
    use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft};
    use crate::domain::entity::context::Context;
    use crate::domain::entity::crux::{Crux, CruxDraft};
    use crate::domain::entity::pin::Pin;
    use crate::domain::entity::project::Project;
    use crate::domain::service::change_request::expectations_now;
    use crate::domain::service::pin::record;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::{
        CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, CruxId, CruxOptionId, HarnessId,
        InterfaceId, ItemId, PinId, ProjectId, test_uuid,
    };
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::revision::Revision;
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

    /// Revises the Interface: item 10 (key `a`) declared as `first`, item 11 (key `b`) as `fn b()`.
    pub(super) fn put_interface(p: &mut Project, first: &str) {
        p.put_interface(
            ctx(),
            iface(),
            Source::new(format!("{first}\nfn b()\n")),
            vec![
                (
                    item(10),
                    ItemKey::try_from("a").unwrap(),
                    LineRange::new(1, 1).unwrap(),
                ),
                (
                    item(11),
                    ItemKey::try_from("b").unwrap(),
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

    /// A change request, expecting `wanted` of targets at their revisions now.
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
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        )
    }

    /// An open Crux numbered `n`, raised in the change request numbered `raised_in`, with one option for each of
    /// `options` (the choice, and the targets that option expects). The option `k` (from 0) has the id
    /// `option_id(n, k)`.
    pub(super) fn crux(
        n: u128,
        raised_in: u128,
        kind: crate::domain::vo::crux::CruxKind,
        options: &[(&str, &[(Target, ExpectKind)])],
    ) -> Crux {
        use crate::domain::vo::choice::Choice;
        use crate::domain::vo::crux::CruxOption;
        use crate::domain::vo::prose::Prose;
        Crux::raise(
            CruxId::from_uuid(test_uuid(n)),
            CruxDraft {
                raised_in: ChangeRequestId::from_uuid(test_uuid(raised_in)),
                findings: Vec::new(),
                kind,
                question: Prose::try_from("which one?").unwrap(),
                options: options
                    .iter()
                    .enumerate()
                    .map(|(k, (choice, expects))| {
                        CruxOption::new(
                            option_id(n, k),
                            Choice::try_from(*choice).unwrap(),
                            None,
                            expects.iter().copied().collect(),
                            None,
                        )
                    })
                    .collect(),
                recommended: None,
                importance: None,
                supersedes: None,
            },
        )
        .unwrap()
    }

    /// The id of the option `k` of the Crux `n` made by [`crux`].
    pub(super) fn option_id(n: u128, k: usize) -> CruxOptionId {
        CruxOptionId::from_uuid(test_uuid(100_000 + n * 100 + k as u128))
    }

    /// `crux` answered with its option `k`, on `basis`.
    pub(super) fn answered(
        mut crux: Crux,
        k: usize,
        basis: crate::domain::vo::answer::Basis,
    ) -> Crux {
        use crate::domain::vo::answer::{Answer, DecisionMaker};
        use crate::domain::vo::crux::Decision;
        use crate::domain::vo::id::AnswerId;
        use crate::domain::vo::timestamp::Timestamp;
        let n = crux.id().as_uuid().as_u128();
        let option = crux.options()[k].id();
        crux.answer(Answer::new(
            AnswerId::from_uuid(test_uuid(900_000 + n)),
            Decision::Chosen {
                option,
                reason: None,
            },
            basis,
            DecisionMaker::role("orchestrator").unwrap(),
            Timestamp::from_millis(1),
        ))
        .unwrap();
        crux
    }

    /// What `check` finds in `p` with these Pins and change requests.
    pub(super) fn run(
        check: &dyn Check,
        p: &Project,
        pins: &[Pin],
        crs: &[ChangeRequest],
    ) -> Vec<Finding> {
        run_with_cruxes(check, p, pins, crs, &[])
    }

    /// What `check` finds in `p` with these Pins, change requests and Cruxes.
    pub(super) fn run_with_cruxes(
        check: &dyn Check,
        p: &Project,
        pins: &[Pin],
        crs: &[ChangeRequest],
        cruxes: &[Crux],
    ) -> Vec<Finding> {
        check.run(
            &CheckView {
                project: p,
                pins,
                change_requests: crs,
                cruxes,
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
    use crate::domain::entity::project::Project;

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
            assert!(
                d.moments.contains(&Moment::Check),
                "{} runs on demand",
                d.id
            );
            assert!(!d.summary.is_empty() && !d.help.is_empty());
            let note = matches!(
                d.id.as_str(),
                "pin.unpinned"
                    | "context.no-interface"
                    | "context.no-harness"
                    | "cr.intent-mismatch"
                    | "cr.targets-untouched"
                    | "crux.policy-line-gone"
                    | "mutation.unmeasured"
                    | "mutation.survived"
                    | "mutation.capped"
            );
            let error = matches!(
                d.id.as_str(),
                "sim.shape-kept" | "sim.finding" | "sim.unreached" | "sim.nondeterministic"
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
            &[],
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
    fn a_crux_not_decided_is_found_until_it_is_decided_even_in_a_request_that_is_resolved() {
        use crate::domain::vo::answer::Basis;
        use crate::domain::vo::crux::CruxKind;
        use crate::domain::vo::prose::Prose;
        let p = project();
        let plain = cr(&p, 1, Intent::ChangeBehavior, None, &[]);
        let asking = cr(&p, 2, Intent::ChangeBehavior, None, &[]);
        let mut gone = cr(&p, 3, Intent::ChangeBehavior, None, &[]);
        resolve_as(&mut gone, ResolutionKind::WontDo, &[]);
        let two = |n, raised_in| crux(n, raised_in, CruxKind::Conflict, &[("a", &[]), ("b", &[])]);
        let crs = [plain, asking.clone(), gone.clone()];
        let cruxes = [two(10, 2), two(11, 3), two(12, 2)];
        let found = run_with_cruxes(&AwaitingDecision, &p, &[], &crs, &cruxes);
        // One finding for each request, however many of its Cruxes are open, and one for the resolved request.
        assert_eq!(
            subjects(&found),
            [
                Some(Subject::ChangeRequest(asking.id())),
                Some(Subject::ChangeRequest(gone.id()))
            ]
        );
        assert_eq!(found[0].level(), crate::domain::check::Level::Warning);
        assert!(found[0].message().contains(&cruxes[0].id().to_string()));
        assert!(found[0].message().contains(&cruxes[2].id().to_string()));
        // Decided, a Crux is not found; a Crux of the same request still open is.
        let basis = || Basis::Arbitrary {
            reason: Prose::try_from("nothing derives it").unwrap(),
        };
        let cruxes = [
            answered(two(10, 2), 0, basis()),
            answered(two(11, 3), 1, basis()),
            two(12, 2),
        ];
        let found = run_with_cruxes(&AwaitingDecision, &p, &[], &crs, &cruxes);
        assert_eq!(
            subjects(&found),
            [Some(Subject::ChangeRequest(asking.id()))]
        );
        let cruxes = [
            answered(two(10, 2), 0, basis()),
            answered(two(11, 3), 1, basis()),
            answered(two(12, 2), 0, basis()),
        ];
        assert!(run_with_cruxes(&AwaitingDecision, &p, &[], &crs, &cruxes).is_empty());
    }

    #[test]
    fn a_decision_on_a_policy_line_that_is_gone_or_reworded_is_noted() {
        use crate::domain::vo::answer::Basis;
        use crate::domain::vo::crux::CruxKind;
        use crate::domain::vo::policy::PolicyLineId;
        use crate::domain::vo::prose::Prose;
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
        let on_p1 = cr(&p, 1, Intent::ChangeBehavior, None, &[]);
        let on_p2 = cr(&p, 2, Intent::ChangeBehavior, None, &[]);
        let arbitrary = cr(&p, 3, Intent::ChangeBehavior, None, &[]);
        let decided = |n: u128, raised_in: u128, basis: Basis| {
            answered(
                crux(n, raised_in, CruxKind::Ambiguity, &[("a", &[])]),
                0,
                basis,
            )
        };
        let on = |n: &str| {
            p.policy()
                .basis_on(PolicyLineId::try_from(n).unwrap())
                .unwrap()
        };
        let crs = [on_p1.clone(), on_p2.clone(), arbitrary];
        let cruxes = [
            decided(10, 1, on("P1")),
            decided(11, 2, on("P2")),
            decided(
                12,
                3,
                Basis::Arbitrary {
                    reason: Prose::try_from("the product leans this way").unwrap(),
                },
            ),
        ];
        // Nothing moved.
        assert!(run_with_cruxes(&PolicyLineGone, &p, &[], &crs, &cruxes).is_empty());
        // P1 is reworded, P2 is dropped: both are found, the arbitrary one never is.
        p.put_policy(vec![line("P1", "take it from 90")]).unwrap();
        let found = run_with_cruxes(&PolicyLineGone, &p, &[], &crs, &cruxes);
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
        let basis = q
            .policy()
            .basis_on(PolicyLineId::try_from("P1").unwrap())
            .unwrap();
        let kept_crs = [on_p1];
        let kept = [decided(10, 1, basis)];
        q.put_policy(vec![line("P1", "take it from 80"), line("P3", "new")])
            .unwrap();
        assert!(run_with_cruxes(&PolicyLineGone, &q, &[], &kept_crs, &kept).is_empty());
        // Written again after it was dropped, it is a new text.
        q.put_policy(vec![line("P3", "new")]).unwrap();
        q.put_policy(vec![line("P1", "take it from 80")]).unwrap();
        assert_eq!(
            run_with_cruxes(&PolicyLineGone, &q, &[], &kept_crs, &kept).len(),
            1
        );
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
        use crate::domain::vo::crux::CruxKind;
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
        // Nothing is named in itself, but an option of a Crux raised in it names a target.
        let option_removes = cr(&p, 6, Intent::ChangeBehavior, None, &[]);
        let asks = crux(
            60,
            6,
            CruxKind::Conflict,
            &[("drop", &[(Target::Clause(clause(31)), ExpectKind::Target)])],
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
        let found = run_with_cruxes(&IntentMismatch, &p, &[], &crs, std::slice::from_ref(&asks));
        assert_eq!(
            subjects(&found),
            [
                Some(Subject::ChangeRequest(keeps_but_changes_a_clause.id())),
                Some(Subject::ChangeRequest(keeps_but_removes_a_clause.id())),
                Some(Subject::ChangeRequest(changes_nothing.id())),
            ]
        );
        assert_eq!(found[0].level(), crate::domain::check::Level::Note);
        // Without the Crux, the request that only the option names a target for names none.
        let alone = run(&IntentMismatch, &p, &[], &crs);
        assert_eq!(alone.len(), 4);
    }

    use crate::domain::check::{Ground, LineState, introduced};
    use crate::domain::service::footprint::moves_between;
    use crate::domain::vo::footprint::Footprint;

    #[test]
    fn a_citation_that_moves_on_is_introduced_by_the_put_that_moved_it_and_not_again_by_one_that_did_not()
     {
        let mut p = project();
        put_contract(&mut p, "a half rounds to even");
        let was = run(&StaleCite, &p, &[], &[]);
        assert_eq!(was.len(), 1);
        assert_eq!(was[0].grounds().len(), 1);
        // A put that leaves the Project as it is introduces nothing.
        assert!(introduced(&was, &run(&StaleCite, &p, &[], &[])).is_empty());
        // The Case now also cites an Item that moved: a ground the finding did not have.
        let mut q = p.clone();
        put_interface(&mut q, "fn a(x: u8)");
        let found = introduced(&was, &run(&StaleCite, &q, &[], &[]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].grounds().len(), 2);
        // The same Clause moves on again: the finding is the same and its mark is another.
        let mut r = p.clone();
        put_contract(&mut r, "a half rounds to odd");
        assert_eq!(introduced(&was, &run(&StaleCite, &r, &[], &[])).len(), 1);
    }

    #[test]
    fn a_pin_that_goes_stale_for_one_more_reason_or_the_same_reason_again_is_introduced() {
        let mut p = project();
        let pins = [pin(&p)];
        put_contract(&mut p, "a half rounds to even");
        let was = run(&StalePin, &p, &pins, &[]);
        assert_eq!(was.len(), 1);
        // Nothing moved since.
        assert!(introduced(&was, &run(&StalePin, &p, &pins, &[])).is_empty());
        // The Harness the Pin stands on moves too: one more reason.
        let mut q = p.clone();
        revise_harness(&mut q);
        let found = introduced(&was, &run(&StalePin, &q, &pins, &[]));
        assert_eq!(found.len(), 1);
        assert!(matches!(
            found[0].grounds(),
            [Ground::Pin { .. }, Ground::Pin { .. }]
        ));
        // The same Contract moves again: the reason is the same and the revision it has is not.
        let mut r = p.clone();
        put_contract(&mut r, "a half rounds to odd");
        assert_eq!(introduced(&was, &run(&StalePin, &r, &pins, &[])).len(), 1);
    }

    #[test]
    fn a_target_a_request_keeps_that_moves_again_is_introduced() {
        let mut p = project();
        let keeping = cr(
            &p,
            1,
            Intent::KeepBehavior,
            None,
            &[(Target::Clause(clause(30)), ExpectKind::Keep)],
        );
        let crs = [keeping];
        put_contract(&mut p, "a half rounds to even");
        let was = run(&KeepMoved, &p, &[], &crs);
        assert_eq!(was.len(), 1);
        assert!(introduced(&was, &run(&KeepMoved, &p, &[], &crs)).is_empty());
        let mut q = p.clone();
        put_contract(&mut q, "a half rounds to odd");
        assert_eq!(introduced(&was, &run(&KeepMoved, &q, &[], &crs)).len(), 1);
    }

    #[test]
    fn work_outside_the_frame_is_introduced_by_the_work_that_did_it_and_not_by_a_put_of_no_request()
    {
        let mut p = project();
        let mut c = cr(
            &p,
            1,
            Intent::ChangeBehavior,
            None,
            &[(Target::Clause(clause(30)), ExpectKind::Target)],
        );
        applied_for(&mut c, &mut p, |p| put_interface(p, "fn a(x: u8)"));
        let was = run(&OutOfFrame, &p, &[], std::slice::from_ref(&c));
        assert_eq!(was.len(), 1);
        // Another request, or none, moves the same target again: the work of this one is as it was.
        let mut q = p.clone();
        put_interface(&mut q, "fn a(x: u16)");
        assert!(introduced(&was, &run(&OutOfFrame, &q, &[], std::slice::from_ref(&c))).is_empty());
        // Its own new work moves the same target again: that is another change set of its own.
        let mut d = c.clone();
        let mut r = p.clone();
        put_interface(&mut r, "fn a(x: u16)");
        d.record_work(Footprint::new(3, moves_between(&p, &r)))
            .unwrap();
        let found = introduced(&was, &run(&OutOfFrame, &r, &[], std::slice::from_ref(&d)));
        assert_eq!(found.len(), 1);
        assert!(
            found[0]
                .grounds()
                .iter()
                .any(|g| matches!(g, Ground::Work { at: 3, .. }))
        );
    }

    /// A change request, and a Crux raised in it answered on the Policy line `line` as the Policy of `p` has it.
    fn decided_on(
        p: &Project,
        line: &str,
    ) -> (
        crate::domain::entity::change_request::ChangeRequest,
        crate::domain::entity::crux::Crux,
    ) {
        use crate::domain::vo::crux::CruxKind;
        use crate::domain::vo::policy::PolicyLineId;
        let basis = p
            .policy()
            .basis_on(PolicyLineId::try_from(line).unwrap())
            .unwrap();
        (
            cr(p, 1, Intent::ChangeBehavior, None, &[]),
            answered(crux(10, 1, CruxKind::Ambiguity, &[("a", &[])]), 0, basis),
        )
    }

    #[test]
    fn a_policy_line_that_goes_from_reworded_to_gone_is_introduced_and_another_line_reworded_is_not()
     {
        use crate::domain::vo::policy::PolicyLineId;
        use crate::domain::vo::prose::Prose;
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
        let (request, decided) = decided_on(&p, "P1");
        let (crs, cruxes) = ([request], [decided]);
        // Nothing found while the line is as it was; the line reworded is found.
        assert!(run_with_cruxes(&PolicyLineGone, &p, &[], &crs, &cruxes).is_empty());
        let mut reworded = p.clone();
        reworded
            .put_policy(vec![
                line("P1", "take it from 90"),
                line("P2", "low: either"),
            ])
            .unwrap();
        let was = run_with_cruxes(&PolicyLineGone, &reworded, &[], &crs, &cruxes);
        assert_eq!(was.len(), 1);
        assert!(matches!(
            was[0].grounds(),
            [Ground::PolicyLine {
                state: LineState::Reworded(_),
                ..
            }]
        ));
        assert_eq!(
            introduced(
                &[],
                &run_with_cruxes(&PolicyLineGone, &reworded, &[], &crs, &cruxes)
            )
            .len(),
            1
        );
        // The line is dropped from the reworded text: it is gone now, which the finding did not say.
        let mut gone = reworded.clone();
        gone.put_policy(vec![line("P2", "low: either")]).unwrap();
        let found = introduced(
            &was,
            &run_with_cruxes(&PolicyLineGone, &gone, &[], &crs, &cruxes),
        );
        assert_eq!(found.len(), 1);
        assert!(matches!(
            found[0].grounds(),
            [Ground::PolicyLine {
                state: LineState::Gone,
                ..
            }]
        ));
        // Another line reworded moves the Policy, and not what this decision rests on.
        let mut other = reworded.clone();
        other
            .put_policy(vec![
                line("P1", "take it from 90"),
                line("P2", "low: the other"),
            ])
            .unwrap();
        assert_eq!(
            run_with_cruxes(&PolicyLineGone, &other, &[], &crs, &cruxes).len(),
            1
        );
        assert!(
            introduced(
                &was,
                &run_with_cruxes(&PolicyLineGone, &other, &[], &crs, &cruxes)
            )
            .is_empty()
        );
    }
}
