//! Deciding and resolving a change request against the other change requests, and reading what its work did.
//!
//! A change request ends by an explicit resolution. Only explicit states refuse one: a Crux not answered (to
//! resolve as done), a part not resolved, a resolution pointing at a request that is not there, the request
//! resolved already. The revisions of the Project and the work recorded are never consulted by a resolution; they
//! are read afterwards, for `status` and the checks, which say where the record and the work disagree.
//!
//! The work of a request is its footprints and those of the requests that are a part of it, directly or not
//! (`moved_under`). The targets a request names to revise or retire are its frame: naming a container frames
//! everything in it. The frame of one request is its own; a parent frames nothing for its parts.

use std::collections::{BTreeMap, BTreeSet};

use super::cite::revision_of;
use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestError, ChangeRequestState, Crux,
};
use crate::domain::entity::project::Project;
use crate::domain::vo::answer::{Answer, Basis};
use crate::domain::vo::footprint::Move;
use crate::domain::vo::id::ChangeRequestId;
use crate::domain::vo::resolution::Resolution;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::{ExpectKind, Expectation, Target};

/// The revision each of `targets` has in `project` now; a target the Project does not have is left out.
pub fn revisions_now(
    project: &Project,
    targets: impl IntoIterator<Item = Target>,
) -> BTreeMap<Target, Revision> {
    targets
        .into_iter()
        .filter_map(|t| revision_of(project, t).map(|r| (t, r)))
        .collect()
}

/// The expectations `wanted` as a change request records them: each target with what is asked of it and the
/// revision it has in `project` now. A target the Project does not have is left out; the caller reports it.
pub fn expectations_now(
    project: &Project,
    wanted: impl IntoIterator<Item = (Target, ExpectKind)>,
) -> BTreeMap<Target, Expectation> {
    wanted
        .into_iter()
        .filter_map(|(t, kind)| {
            revision_of(project, t).map(|recorded| (t, Expectation { kind, recorded }))
        })
        .collect()
}

/// Gives the Crux of `cr` its answer, once, with each target the option chosen names at the revision it has in
/// `project` now. An answer on the basis of a Policy line must cite a line the Policy of `project` has, at the
/// revision it has now (the line applied is the line as it reads at the decision). A Crux that is answered already
/// takes no other answer.
pub fn decide(
    cr: &mut ChangeRequest,
    answer: Answer,
    project: &Project,
) -> Result<(), ChangeRequestError> {
    if let Basis::Policy {
        line,
        policy_revision,
    } = answer.basis()
    {
        let policy = project.policy();
        if policy.line(*line).is_none() {
            return Err(ChangeRequestError::UnknownPolicyLine(*line));
        }
        if *policy_revision != policy.revision() {
            return Err(ChangeRequestError::PolicyRevisionMismatch {
                recorded: *policy_revision,
                now: policy.revision(),
            });
        }
    }
    let targets = cr
        .crux()
        .into_iter()
        .flat_map(|q| q.options())
        .flat_map(|o| o.expects().keys().copied());
    let now = revisions_now(project, targets);
    cr.decide(answer, &now)
}

/// Whether a Crux may supersede the answered Crux of the change request `replaced`: `all` must have it, and its
/// Crux must be answered. What was decided is not rewritten; a different decision is a new Crux that points at the
/// one it replaces.
pub fn check_supersedes(
    replaced: ChangeRequestId,
    all: &[ChangeRequest],
) -> Result<(), ChangeRequestError> {
    let cr = all
        .iter()
        .find(|c| c.id() == replaced)
        .ok_or(ChangeRequestError::SupersedesUnknown(replaced))?;
    match cr.crux().and_then(Crux::answer) {
        Some(_) => Ok(()),
        None => Err(ChangeRequestError::SupersedesUnanswered(replaced)),
    }
}

/// `cr` and every change request in `all` that is a part of it, directly or not. `cr` is the one given (the copy in
/// `all`, if there is one, is left out), and a cycle of parents is followed once.
fn lineage<'a>(cr: &'a ChangeRequest, all: &'a [ChangeRequest]) -> Vec<&'a ChangeRequest> {
    let mut out = vec![cr];
    let mut seen = BTreeSet::from([cr.id()]);
    let mut next = 0;
    while next < out.len() {
        let parent = out[next].id();
        next += 1;
        for c in all.iter().filter(|c| c.parent() == Some(parent)) {
            if seen.insert(c.id()) {
                out.push(c);
            }
        }
    }
    out
}

/// The moves in the work of `cr` or of a part of it, in the order of the footprints.
fn moves_under<'a>(cr: &'a ChangeRequest, all: &'a [ChangeRequest]) -> Vec<&'a Move> {
    lineage(cr, all)
        .into_iter()
        .flat_map(|c| c.work())
        .flat_map(|f| f.moves())
        .collect()
}

/// Whether the work of `cr` or of a part of it moved something (a change set that moved nothing is no work).
pub fn has_work(cr: &ChangeRequest, all: &[ChangeRequest]) -> bool {
    !moves_under(cr, all).is_empty()
}

/// The change requests, in `crs`, that are a part of `parent` (their parent is it) and are not resolved.
pub fn open_parts(parent: ChangeRequestId, crs: &[ChangeRequest]) -> Vec<ChangeRequestId> {
    crs.iter()
        .filter(|c| c.parent() == Some(parent) && c.state() != ChangeRequestState::Resolved)
        .map(|c| c.id())
        .collect()
}

/// Closes the record of `cr` with `resolution`. The explicit states that refuse it, in this order: `cr` resolved
/// already; the Crux not answered, when it is resolved as done; a resolution that points at `cr` itself or at a
/// change request `all` does not have; a change request in `all` that is a part of `cr` and not resolved. Nothing
/// else is read: not the revisions of the targets, not the work. A change request is a record of its own, so `all`
/// is every change request there is (`cr` itself may be among them).
pub fn resolve(
    cr: &mut ChangeRequest,
    resolution: Resolution,
    all: &[ChangeRequest],
) -> Result<(), ChangeRequestError> {
    if cr.is_resolved() {
        return Err(ChangeRequestError::AlreadyResolved);
    }
    if resolution.kind().is_done() && cr.awaits_decision() {
        return Err(ChangeRequestError::Undecided);
    }
    if let Some(reference) = resolution.kind().reference() {
        if reference == cr.id() {
            return Err(ChangeRequestError::ReferencesItself);
        }
        if !all.iter().any(|c| c.id() == reference) {
            return Err(ChangeRequestError::ReferenceUnknown(reference));
        }
    }
    let children = open_parts(cr.id(), all);
    if !children.is_empty() {
        return Err(ChangeRequestError::ChildrenNotResolved { children });
    }
    cr.resolve(resolution)
}

/// What the work did to a target a change request names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The work moved it (or something in it) and it is still there.
    Revised,
    /// The work moved it and it is gone.
    Retired,
    /// Nothing in the work of the request, or of a part of it, moved it.
    NotTouched,
}

impl Progress {
    /// The word for it.
    pub fn word(self) -> &'static str {
        match self {
            Progress::Revised => "revised",
            Progress::Retired => "retired",
            Progress::NotTouched => "not touched yet",
        }
    }
}

/// A target a change request names, and what the work did to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetProgress {
    /// The target.
    pub target: Target,
    /// What the work did to it.
    pub progress: Progress,
}

/// A target a change request keeps, and whether it moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepProgress {
    /// The target.
    pub target: Target,
    /// Whether it is no longer at the revision recorded (or is gone), whoever moved it.
    pub moved: bool,
}

/// Each target `cr` names to revise or retire (its own, then those the decision named), and what the footprints of
/// its work and of its parts' work did to it: moved and still there is `Revised`, moved and gone `Retired`, never
/// moved `NotTouched`. A container is moved when something inside it is. A target in the lists once. The revision of
/// the target is not read, only whether the Project still has it.
pub fn target_progress(
    cr: &ChangeRequest,
    project: &Project,
    all: &[ChangeRequest],
) -> Vec<TargetProgress> {
    let moves = moves_under(cr, all);
    let mut out: Vec<TargetProgress> = Vec::new();
    for (target, e) in cr.expectations() {
        if e.kind != ExpectKind::Target || out.iter().any(|p| p.target == target) {
            continue;
        }
        let progress = if !moves.iter().any(|m| m.is_inside(target)) {
            Progress::NotTouched
        } else if revision_of(project, target).is_none() {
            Progress::Retired
        } else {
            Progress::Revised
        };
        out.push(TargetProgress { target, progress });
    }
    out
}

/// Each target `cr` keeps (its own, then those the decision named) and whether it moved. A target in the list once.
pub fn keep_progress(cr: &ChangeRequest, project: &Project) -> Vec<KeepProgress> {
    let mut out: Vec<KeepProgress> = Vec::new();
    for (target, e) in cr.expectations() {
        if e.kind != ExpectKind::Keep || out.iter().any(|p| p.target == target) {
            continue;
        }
        out.push(KeepProgress {
            target,
            moved: e.keep_broken_by(revision_of(project, target)),
        });
    }
    out
}

/// A change request resolved as done with no work on it or on its parts.
pub fn resolved_without_work(crs: &[ChangeRequest]) -> Vec<ChangeRequestId> {
    crs.iter()
        .filter(|c| c.resolution().is_some_and(|r| r.kind().is_done()) && !has_work(c, crs))
        .map(|c| c.id())
        .collect()
}

/// A change request resolved as done that named targets its work never touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetsUntouched {
    /// The change request.
    pub cr: ChangeRequestId,
    /// The targets no work of it, or of a part of it, moved, in order.
    pub untouched: Vec<Target>,
}

/// Every change request in `crs` resolved as done that has work, and named a target to revise or retire that no
/// move in its work or its parts' work touches. One with no work at all is [`resolved_without_work`], not here.
pub fn targets_untouched(crs: &[ChangeRequest], project: &Project) -> Vec<TargetsUntouched> {
    crs.iter()
        .filter(|c| c.resolution().is_some_and(|r| r.kind().is_done()) && has_work(c, crs))
        .filter_map(|cr| {
            let untouched: Vec<Target> = target_progress(cr, project, crs)
                .into_iter()
                .filter(|p| p.progress == Progress::NotTouched)
                .map(|p| p.target)
                .collect();
            (!untouched.is_empty()).then(|| TargetsUntouched {
                cr: cr.id(),
                untouched,
            })
        })
        .collect()
}

/// An unresolved change request one of whose keeps was broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepMoved {
    /// The change request.
    pub cr: ChangeRequestId,
    /// The targets it keeps that are not at the revision it recorded (or are gone), in order.
    pub moved: Vec<Target>,
}

/// Every unresolved change request in `crs` that keeps a target and the target has moved on in `project`, whoever
/// moved it: a keep is judged by the revision alone. A resolved one is a record: what changes after it is another
/// change request's business.
pub fn keep_moved(crs: &[ChangeRequest], project: &Project) -> Vec<KeepMoved> {
    crs.iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved)
        .filter_map(|cr| {
            let moved: Vec<Target> = keep_progress(cr, project)
                .into_iter()
                .filter(|k| k.moved)
                .map(|k| k.target)
                .collect();
            (!moved.is_empty()).then(|| KeepMoved { cr: cr.id(), moved })
        })
        .collect()
}

/// The targets `cr` names to revise or retire, its own and those its decision named: its frame. A container in it
/// frames everything inside.
pub fn frame(cr: &ChangeRequest) -> BTreeSet<Target> {
    cr.expectations()
        .filter(|(_, e)| e.kind == ExpectKind::Target)
        .map(|(t, _)| t)
        .collect()
}

/// An unresolved change request whose work moved targets outside its frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutOfFrame {
    /// The change request the work was applied for.
    pub cr: ChangeRequestId,
    /// The targets it moved outside its frame, in order.
    pub moved: Vec<Target>,
}

/// Every unresolved change request in `crs` with work that moved a target outside its own frame (the frames of the
/// requests it is a part of do not count: a frame is per request). A target is inside the frame when it is, or is
/// inside, a target the frame names. A container that moved only because something inside it moved within the
/// frame (a Contract that took a new Clause, the Context that holds a new asset) is not outside. The work of a part
/// is judged for the part, not again for its parent. A request with no frame has all its work outside.
pub fn out_of_frame(crs: &[ChangeRequest]) -> Vec<OutOfFrame> {
    crs.iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved && !c.work().is_empty())
        .filter_map(|cr| {
            let frame = frame(cr);
            let inside = |m: &Move| frame.iter().any(|container| m.is_inside(*container));
            // Each change set on its own: a container is covered by what moved inside it in the same set.
            let mut moved = BTreeSet::new();
            for footprint in cr.work() {
                let mut covered = BTreeSet::new();
                for m in footprint.moves().iter().filter(|m| inside(m)) {
                    covered.insert(m.target());
                    covered.extend(m.within().iter().copied());
                }
                moved.extend(footprint.targets().filter(|t| !covered.contains(t)));
            }
            (!moved.is_empty()).then(|| OutOfFrame {
                cr: cr.id(),
                moved: moved.into_iter().collect(),
            })
        })
        .collect()
}

/// Every unresolved change request in `crs` whose Crux is not decided yet: a person is awaited.
pub fn awaiting_decision<'a>(
    crs: impl IntoIterator<Item = &'a ChangeRequest>,
) -> Vec<&'a ChangeRequest> {
    crs.into_iter().filter(|c| c.awaits_decision()).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::entity::change_request::{ChangeRequestDraft, Crux};
    use crate::domain::entity::context::Context;
    use crate::domain::service::footprint::moves_between;
    use crate::domain::vo::answer::DecisionMaker;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned, Recommended};
    use crate::domain::vo::footprint::Footprint;
    use crate::domain::vo::id::{
        CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId, ProjectId,
        test_uuid,
    };
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::policy::PolicyLineId;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::resolution::{Reopening, ResolutionKind, Transition};
    use crate::domain::vo::source::Source;
    use crate::domain::vo::timestamp::Timestamp;
    use crate::domain::vo::title::Title;

    /// [`super::keep_moved`] for the change requests given.
    fn keep_moved<const N: usize>(crs: [&ChangeRequest; N], p: &Project) -> Vec<KeepMoved> {
        let all: Vec<ChangeRequest> = crs.into_iter().cloned().collect();
        super::keep_moved(&all, p)
    }

    /// What a change set `for` `c` does: `change` is made to `p`, and what it moved is recorded as work of `c`.
    fn work(p: &mut Project, c: &mut ChangeRequest, at: u64, change: impl FnOnce(&mut Project)) {
        let before = p.clone();
        change(p);
        c.record_work(Footprint::new(at, moves_between(&before, p)))
            .unwrap();
    }

    fn clause() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(30)))
    }

    /// The clause the question is about: which error a refusal is.
    fn detail() -> Target {
        Target::Clause(ClauseId::from_uuid(test_uuid(31)))
    }

    fn core() -> ContextId {
        ContextId::from_uuid(test_uuid(1))
    }

    fn billing() -> ContextId {
        ContextId::from_uuid(test_uuid(2))
    }

    fn contract() -> Target {
        Target::Contract(ContractId::from_uuid(test_uuid(300)))
    }

    fn interface() -> Target {
        Target::Interface(InterfaceId::from_uuid(test_uuid(100)))
    }

    fn item() -> Target {
        Target::Item(ItemId::from_uuid(test_uuid(10)))
    }

    fn case() -> Target {
        Target::Case(CaseId::from_uuid(test_uuid(40)))
    }

    /// The contract with its two clauses: the settled change (`text`) and the detail (`error`).
    fn put_both(p: &mut Project, text: &str, error: &str) {
        let one = |n: u128, label: &str, text: &str| {
            (
                ClauseId::from_uuid(test_uuid(n)),
                ClauseContent::new(
                    ClauseLabel::try_from(label).unwrap(),
                    ClauseText::try_from(text).unwrap(),
                ),
            )
        };
        p.put_contract(
            core(),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![one(30, "C1", text), one(31, "C2", error)],
        )
        .unwrap();
    }

    fn put_contract(p: &mut Project, text: &str) {
        put_both(p, text, "a refusal is InvalidDiscount");
    }

    /// The contract with its first clause alone.
    fn put_first_only(p: &mut Project, text: &str) {
        p.put_contract(
            core(),
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

    fn put_interface(p: &mut Project, id: u128, item_id: u128, sig: &str) {
        p.put_interface(
            core(),
            InterfaceId::from_uuid(test_uuid(id)),
            Source::new(format!("{sig}\n")),
            vec![(
                ItemId::from_uuid(test_uuid(item_id)),
                ItemKey::try_from(sig.split('(').next().unwrap()).unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
    }

    fn put_harness(p: &mut Project, body: &str) {
        p.put_harness(
            core(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new(format!("{body}\n")),
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
    }

    fn context(id: ContextId, name: &str) -> Context {
        Context::new(id, ContextName::try_from(name).unwrap(), BTreeSet::new()).unwrap()
    }

    /// A Project with the Context `core`: an Interface with one Item, the Contract with two Clauses, and a Harness
    /// with one Case.
    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(context(core(), "core")).unwrap();
        put_interface(&mut p, 100, 10, "fn a()");
        put_contract(&mut p, "a half rounds up");
        put_harness(&mut p, "fn t() {}");
        p
    }

    fn choice(s: &str) -> Choice {
        Choice::try_from(s).unwrap()
    }

    fn draft(p: &Project, intent: Intent, wanted: &[(Target, ExpectKind)]) -> ChangeRequestDraft {
        ChangeRequestDraft {
            title: Title::try_from("How is a half rounded?").unwrap(),
            situation: None,
            intent,
            parent: None,
            issuer: None,
            expects: expectations_now(p, wanted.iter().copied()),
            crux: None,
            assumptions: Vec::new(),
            addresses: Vec::new(),
        }
    }

    fn open(
        p: &Project,
        n: u128,
        intent: Intent,
        wanted: &[(Target, ExpectKind)],
    ) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(n)),
            draft(p, intent, wanted),
        )
    }

    fn cr(p: &Project, crux: Option<Crux>) -> ChangeRequest {
        let mut d = draft(p, Intent::ChangeBehavior, &[(clause(), ExpectKind::Target)]);
        d.crux = crux;
        ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(1)), d)
    }

    /// A settled change (C1, its own target) and a question about a detail (which error type; C2 for a new one).
    fn person(p: &Project) -> ChangeRequest {
        let crux = Crux::new(
            CruxKind::Ambiguity,
            vec![
                CruxOption::new(choice("up"), None, BTreeMap::new(), None),
                CruxOption::new(
                    choice("to even"),
                    Some(Prose::try_from("C2 names a new error").unwrap()),
                    BTreeMap::from([(detail(), ExpectKind::Target)]),
                    None,
                ),
            ],
            Some(Recommended::new(choice("up"), None, None)),
            None,
            None,
        )
        .unwrap();
        cr(p, Some(crux))
    }

    /// An answer on an arbitrary basis, by a person.
    fn chosen(c: &str) -> Answer {
        answer_on(
            c,
            Basis::Arbitrary {
                reason: Prose::try_from("nothing derives it").unwrap(),
            },
        )
    }

    fn answer_on(c: &str, basis: Basis) -> Answer {
        Answer::new(
            Decision::Chosen(Reasoned::new(choice(c), None)),
            basis,
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(5),
        )
    }

    fn line_id(text: &str) -> PolicyLineId {
        PolicyLineId::try_from(text).unwrap()
    }

    fn resolution(kind: ResolutionKind) -> Resolution {
        Resolution::new(
            kind,
            Prose::try_from("settled").unwrap(),
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(7),
        )
    }

    fn done() -> Resolution {
        resolution(ResolutionKind::Done)
    }

    /// Resolves `c` as done against the change requests `all`.
    fn finish(c: &mut ChangeRequest, all: &[ChangeRequest]) -> Result<(), ChangeRequestError> {
        resolve(c, done(), all)
    }

    // The Crux.

    #[test]
    fn a_decision_on_a_policy_basis_must_cite_a_line_the_policy_has() {
        let mut p = project();
        let mut c = person(&p);
        let on = |line: &str, revision| {
            answer_on(
                "up",
                Basis::Policy {
                    line: line_id(line),
                    policy_revision: revision,
                },
            )
        };
        // The policy has no lines yet.
        assert_eq!(
            decide(&mut c, on("P1", p.policy().revision()), &p),
            Err(ChangeRequestError::UnknownPolicyLine(line_id("P1")))
        );
        assert!(c.awaits_decision());
        p.put_policy(vec![(
            line_id("P1"),
            Prose::try_from("take the recommendation from 80").unwrap(),
        )])
        .unwrap();
        // A line it does not have, and a revision it is not at, are refused.
        assert_eq!(
            decide(&mut c, on("P2", p.policy().revision()), &p),
            Err(ChangeRequestError::UnknownPolicyLine(line_id("P2")))
        );
        assert_eq!(
            decide(&mut c, on("P1", Revision::first()), &p),
            Err(ChangeRequestError::PolicyRevisionMismatch {
                recorded: Revision::first(),
                now: p.policy().revision()
            })
        );
        assert!(c.awaits_decision());
        // The line it has, at the revision it has: taken, and the revision is recorded.
        let basis = p.policy().basis_on(line_id("P1")).unwrap();
        decide(&mut c, answer_on("up", basis.clone()), &p).unwrap();
        assert_eq!(
            c.crux().and_then(Crux::answer).map(Answer::basis),
            Some(&basis)
        );
    }

    #[test]
    fn a_second_decision_is_refused_and_what_the_first_named_still_stands() {
        let p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            decide(&mut c, chosen("up"), &p),
            Err(ChangeRequestError::AlreadyAnswered)
        );
        assert_eq!(
            target_progress(&c, &p, &[c.clone()])
                .iter()
                .map(|t| t.target)
                .collect::<Vec<_>>(),
            [clause(), detail()]
        );
    }

    #[test]
    fn a_crux_may_supersede_only_an_answered_crux_of_a_change_request_that_exists() {
        let p = project();
        let mut answered = person(&p);
        let unanswered = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(2)),
            draft(&p, Intent::ChangeBehavior, &[]),
        );
        let asking = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(3)),
            ChangeRequestDraft {
                crux: person(&p).crux().cloned(),
                ..draft(&p, Intent::ChangeBehavior, &[])
            },
        );
        let all = [answered.clone(), unanswered.clone(), asking.clone()];
        // It does not exist.
        let ghost = ChangeRequestId::from_uuid(test_uuid(99));
        assert_eq!(
            check_supersedes(ghost, &all),
            Err(ChangeRequestError::SupersedesUnknown(ghost))
        );
        // It has no Crux, or its Crux is not answered.
        assert_eq!(
            check_supersedes(unanswered.id(), &all),
            Err(ChangeRequestError::SupersedesUnanswered(unanswered.id()))
        );
        assert_eq!(
            check_supersedes(asking.id(), &all),
            Err(ChangeRequestError::SupersedesUnanswered(asking.id()))
        );
        // It is answered.
        decide(&mut answered, chosen("up"), &p).unwrap();
        let all = [answered.clone(), unanswered, asking];
        assert_eq!(check_supersedes(answered.id(), &all), Ok(()));
    }

    #[test]
    fn an_undecided_crux_is_awaited_and_refuses_done_alone() {
        let p = project();
        let mut c = person(&p);
        assert_eq!(awaiting_decision([&c]), [&c]);
        assert_eq!(finish(&mut c, &[]), Err(ChangeRequestError::Undecided));
        assert_eq!(c.state(), ChangeRequestState::Open);
        // Not done: the question may stay open.
        let mut dropped = c.clone();
        resolve(&mut dropped, resolution(ResolutionKind::WontDo), &[]).unwrap();
        assert!(awaiting_decision([&dropped]).is_empty());
        decide(&mut c, chosen("up"), &p).unwrap();
        assert!(awaiting_decision([&c]).is_empty());
        finish(&mut c, &[]).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    // Resolving looks at explicit states only.

    #[test]
    fn a_request_whose_targets_were_never_touched_resolves_as_done() {
        let p = project();
        let mut c = cr(&p, None);
        assert_eq!(
            target_progress(&c, &p, &[]),
            [TargetProgress {
                target: clause(),
                progress: Progress::NotTouched
            }]
        );
        // The revision of the target is not consulted, and neither is the work.
        finish(&mut c, &[]).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
        assert!(!has_work(&c, &[]));
        assert_eq!(resolved_without_work(&[c.clone()]), [c.id()]);
    }

    #[test]
    fn a_request_whose_target_moved_for_someone_else_resolves_as_done_all_the_same() {
        let mut p = project();
        let mut mine = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        put_contract(&mut p, "a half rounds to even");
        finish(&mut mine, &[]).unwrap();
        assert_eq!(mine.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn a_request_with_an_unresolved_part_is_not_resolved_in_any_way() {
        let p = project();
        let mut parent = open(&p, 1, Intent::ChangeBehavior, &[]);
        let mut d = draft(&p, Intent::ChangeBehavior, &[]);
        d.title = Title::try_from("part").unwrap();
        d.parent = Some(parent.id());
        let mut part = ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(2)), d);
        let all = [parent.clone(), part.clone()];
        assert_eq!(open_parts(parent.id(), &all), [part.id()]);
        for kind in [
            ResolutionKind::Done,
            ResolutionKind::WontDo,
            ResolutionKind::Invalid,
        ] {
            assert_eq!(
                resolve(&mut parent, resolution(kind), &all),
                Err(ChangeRequestError::ChildrenNotResolved {
                    children: vec![part.id()]
                })
            );
        }
        assert_eq!(parent.state(), ChangeRequestState::Open);
        // A part ends any way it likes, and then its parent may close.
        resolve(&mut part, resolution(ResolutionKind::WontDo), &all).unwrap();
        let all = [parent.clone(), part];
        assert!(open_parts(parent.id(), &all).is_empty());
        finish(&mut parent, &all).unwrap();
        assert_eq!(parent.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn a_parent_with_no_targets_closes_after_its_parts_whose_work_it_counts() {
        let mut p = project();
        let parent = open(&p, 1, Intent::ChangeBehavior, &[]);
        let mut d = draft(
            &p,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        d.title = Title::try_from("part").unwrap();
        d.parent = Some(parent.id());
        let mut part = ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(2)), d);
        work(&mut p, &mut part, 2, |p| {
            put_contract(p, "a half rounds to even");
        });
        let mut parent = parent;
        let all = [parent.clone(), part.clone()];
        finish(&mut part, &all).unwrap();
        let all = [parent.clone(), part];
        // The parent names nothing; the work of its part is its work.
        assert!(target_progress(&parent, &p, &all).is_empty());
        assert!(has_work(&parent, &all));
        finish(&mut parent, &all).unwrap();
        let all = [parent, all[1].clone()];
        assert!(all.iter().all(ChangeRequest::is_resolved));
        assert!(resolved_without_work(&all).is_empty());
        assert!(targets_untouched(&all, &p).is_empty());
    }

    #[test]
    fn a_resolution_that_points_at_another_request_needs_it_to_exist() {
        let p = project();
        let other = open(&p, 2, Intent::ChangeBehavior, &[]);
        let ghost = ChangeRequestId::from_uuid(test_uuid(99));
        let all = [open(&p, 1, Intent::ChangeBehavior, &[]), other.clone()];
        let kinds = |id| {
            [
                ResolutionKind::Duplicate { of: id },
                ResolutionKind::Superseded { by: id },
            ]
        };
        for kind in kinds(ghost) {
            let mut c = all[0].clone();
            assert_eq!(
                resolve(&mut c, resolution(kind), &all),
                Err(ChangeRequestError::ReferenceUnknown(ghost))
            );
            assert_eq!(c.state(), ChangeRequestState::Open);
        }
        for kind in kinds(all[0].id()) {
            let mut c = all[0].clone();
            assert_eq!(
                resolve(&mut c, resolution(kind), &all),
                Err(ChangeRequestError::ReferencesItself)
            );
        }
        for kind in kinds(other.id()) {
            let mut c = all[0].clone();
            resolve(&mut c, resolution(kind), &all).unwrap();
            assert_eq!(c.resolution().map(|r| r.kind()), Some(kind));
        }
    }

    #[test]
    fn a_resolved_request_is_resolved_once_until_it_is_reopened() {
        let p = project();
        let mut c = cr(&p, None);
        finish(&mut c, &[]).unwrap();
        assert_eq!(
            finish(&mut c, &[]),
            Err(ChangeRequestError::AlreadyResolved)
        );
        let reopening = || {
            Reopening::new(
                Prose::try_from("the report was wrong").unwrap(),
                DecisionMaker::person("bob").unwrap(),
                Timestamp::from_millis(8),
            )
        };
        c.reopen(reopening()).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Open);
        resolve(&mut c, resolution(ResolutionKind::WontDo), &[]).unwrap();
        assert_eq!(c.history().len(), 3);
        assert!(matches!(&c.history()[0], Transition::Resolved(r) if r.kind().is_done()));
        assert!(matches!(&c.history()[1], Transition::Reopened(_)));
        assert!(
            matches!(&c.history()[2], Transition::Resolved(r) if r.kind() == ResolutionKind::WontDo)
        );
    }

    #[test]
    fn a_resolved_request_takes_no_more_work() {
        let p = project();
        let mut c = open(&p, 1, Intent::KeepBehavior, &[]);
        finish(&mut c, &[]).unwrap();
        assert_eq!(
            c.record_work(Footprint::new(2, Vec::new())),
            Err(ChangeRequestError::AlreadyResolved)
        );
    }

    // What the work did to the targets.

    #[test]
    fn each_target_reads_revised_retired_or_not_touched_from_the_work() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[
                (clause(), ExpectKind::Target),
                (detail(), ExpectKind::Target),
                (item(), ExpectKind::Target),
            ],
        );
        let progress = |c: &ChangeRequest, p: &Project| -> Vec<Progress> {
            target_progress(c, p, std::slice::from_ref(c))
                .iter()
                .map(|t| t.progress)
                .collect()
        };
        assert_eq!(progress(&c, &p), [Progress::NotTouched; 3]);
        // Moved for no request: the work of the request is what counts.
        put_contract(&mut p, "a half rounds to odd");
        assert_eq!(progress(&c, &p), [Progress::NotTouched; 3]);
        work(&mut p, &mut c, 3, |p| {
            put_first_only(p, "a half rounds to even")
        });
        assert_eq!(
            progress(&c, &p),
            [Progress::Revised, Progress::Retired, Progress::NotTouched]
        );
        assert_eq!(Progress::NotTouched.word(), "not touched yet");
    }

    #[test]
    fn adding_a_thing_reads_as_a_revision_of_what_holds_it() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[
                (contract(), ExpectKind::Target),
                (Target::Project, ExpectKind::Target),
            ],
        );
        let progress = |c: &ChangeRequest, p: &Project| -> Vec<Progress> {
            target_progress(c, p, std::slice::from_ref(c))
                .iter()
                .map(|t| t.progress)
                .collect()
        };
        // A new Clause is a move of its Contract; every move is inside the Project.
        work(&mut p, &mut c, 2, |p| {
            p.put_contract(
                core(),
                ContractId::from_uuid(test_uuid(300)),
                Title::try_from("T").unwrap(),
                vec![
                    (
                        ClauseId::from_uuid(test_uuid(30)),
                        ClauseContent::new(
                            ClauseLabel::try_from("C1").unwrap(),
                            ClauseText::try_from("a half rounds up").unwrap(),
                        ),
                    ),
                    (
                        ClauseId::from_uuid(test_uuid(31)),
                        ClauseContent::new(
                            ClauseLabel::try_from("C2").unwrap(),
                            ClauseText::try_from("a refusal is InvalidDiscount").unwrap(),
                        ),
                    ),
                    (
                        ClauseId::from_uuid(test_uuid(32)),
                        ClauseContent::new(
                            ClauseLabel::try_from("C3").unwrap(),
                            ClauseText::try_from("a total of no lines is zero").unwrap(),
                        ),
                    ),
                ],
            )
            .unwrap();
        });
        assert_eq!(progress(&c, &p)[0], Progress::Revised);
        // The Project is a container of everything the work moved.
        assert_eq!(progress(&c, &p)[1], Progress::Revised);
    }

    #[test]
    fn a_context_retired_in_the_work_reads_retired() {
        let mut p = project();
        p.add_context(context(billing(), "billing")).unwrap();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(Target::Context(billing()), ExpectKind::Target)],
        );
        work(&mut p, &mut c, 2, |p| {
            p.retire_context(billing()).unwrap();
        });
        assert_eq!(
            target_progress(&c, &p, std::slice::from_ref(&c))[0].progress,
            Progress::Retired
        );
    }

    #[test]
    fn the_work_of_a_part_counts_for_its_parent_and_a_sibling_of_the_parent_does_not() {
        let mut p = project();
        let titled = |n: u128, title: &str, parent: Option<&ChangeRequest>| {
            let mut d = draft(
                &p,
                Intent::ChangeBehavior,
                &[(contract(), ExpectKind::Target)],
            );
            d.title = Title::try_from(title).unwrap();
            d.parent = parent.map(ChangeRequest::id);
            ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(n)), d)
        };
        let parent = titled(1, "parent", None);
        let mut part = titled(2, "part", Some(&parent));
        let mut stranger = titled(3, "stranger", None);
        work(&mut p, &mut stranger, 2, |p| {
            put_both(p, "a half rounds up", "a refusal is InvalidRate");
        });
        let all = [parent.clone(), part.clone(), stranger.clone()];
        assert_eq!(
            target_progress(&parent, &p, &all)[0].progress,
            Progress::NotTouched
        );
        work(&mut p, &mut part, 3, |p| {
            put_contract(p, "a half rounds to even")
        });
        let all = [parent.clone(), part, stranger];
        assert_eq!(
            target_progress(&parent, &p, &all)[0].progress,
            Progress::Revised
        );
    }

    #[test]
    fn a_request_resolved_as_done_that_never_touched_a_target_is_found_when_it_did_other_work() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target), (item(), ExpectKind::Target)],
        );
        // No work at all: resolved-without-work covers it alone.
        let mut none = c.clone();
        finish(&mut none, &[]).unwrap();
        assert!(targets_untouched(&[none.clone()], &p).is_empty());
        assert_eq!(resolved_without_work(&[none]).len(), 1);
        work(&mut p, &mut c, 2, |p| {
            put_contract(p, "a half rounds to even")
        });
        finish(&mut c, &[]).unwrap();
        assert!(resolved_without_work(&[c.clone()]).is_empty());
        assert_eq!(
            targets_untouched(&[c.clone()], &p),
            [TargetsUntouched {
                cr: c.id(),
                untouched: vec![item()]
            }]
        );
        // Not done: nothing to find.
        let mut dropped = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(item(), ExpectKind::Target)],
        );
        resolve(&mut dropped, resolution(ResolutionKind::WontDo), &[]).unwrap();
        assert!(targets_untouched(&[dropped.clone()], &p).is_empty());
        assert!(resolved_without_work(&[dropped]).is_empty());
    }

    #[test]
    fn a_change_set_that_moved_nothing_is_no_work() {
        let p = project();
        let mut c = cr(&p, None);
        c.record_work(Footprint::new(2, Vec::new())).unwrap();
        assert!(!has_work(&c, &[]));
        finish(&mut c, &[]).unwrap();
        assert_eq!(resolved_without_work(&[c.clone()]), [c.id()]);
    }

    #[test]
    fn what_the_chosen_option_names_is_a_target_too() {
        let p = project();
        let mut c = person(&p);
        assert_eq!(target_progress(&c, &p, &[]).len(), 1);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            target_progress(&c, &p, &[])
                .iter()
                .map(|t| t.target)
                .collect::<Vec<_>>(),
            [clause(), detail()]
        );
        assert_eq!(frame(&c), BTreeSet::from([clause(), detail()]));
    }

    // Keep.

    #[test]
    fn a_keep_that_moved_is_reported_whoever_moved_it_and_does_not_refuse_the_resolve() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(clause(), ExpectKind::Keep), (item(), ExpectKind::Target)],
        );
        assert!(keep_moved([&c], &p).is_empty());
        assert_eq!(
            keep_progress(&c, &p),
            [KeepProgress {
                target: clause(),
                moved: false
            }]
        );
        work(&mut p, &mut c, 2, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)")
        });
        // Someone else moves the clause, in a change set for no request.
        put_contract(&mut p, "a half rounds to even");
        assert_eq!(
            keep_moved([&c], &p),
            [KeepMoved {
                cr: c.id(),
                moved: vec![clause()]
            }]
        );
        assert!(keep_progress(&c, &p)[0].moved);
        // Gone is moved too.
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(keep_moved([&c], &empty)[0].moved, [clause()]);
        // The record is explicit: the keep is reported, not a reason to refuse.
        finish(&mut c, &[]).unwrap();
        assert!(keep_moved([&c], &p).is_empty());
    }

    #[test]
    fn a_keep_is_judged_by_the_revision_whoever_moved_it() {
        let mut p = project();
        let mine = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Keep)],
        );
        let mut other = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        work(&mut p, &mut other, 2, |p| {
            put_contract(p, "a half rounds to even")
        });
        let all = [mine.clone(), other];
        assert_eq!(
            super::keep_moved(&all, &p),
            [KeepMoved {
                cr: mine.id(),
                moved: vec![clause()]
            }]
        );
    }

    #[test]
    fn an_option_that_keeps_a_target_is_a_keep_once_chosen() {
        let p = project();
        let crux = Crux::new(
            CruxKind::Conflict,
            vec![
                CruxOption::new(
                    choice("drop C2"),
                    None,
                    BTreeMap::from([(detail(), ExpectKind::Target)]),
                    None,
                ),
                CruxOption::new(
                    choice("keep both"),
                    None,
                    BTreeMap::from([(detail(), ExpectKind::Keep)]),
                    None,
                ),
            ],
            None,
            None,
            None,
        )
        .unwrap();
        let mut c = cr(&p, Some(crux));
        decide(&mut c, chosen("keep both"), &p).unwrap();
        assert_eq!(c.decided_expects()[&detail()].kind, ExpectKind::Keep);
        assert_eq!(keep_progress(&c, &p).len(), 1);
        assert_eq!(frame(&c), BTreeSet::from([clause()]));
    }

    // Frame: the work stays inside what the request named.

    #[test]
    fn work_on_a_target_the_request_did_not_name_is_out_of_frame() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        // The clause (and its contract, which only moved because of it) is inside.
        work(&mut p, &mut c, 2, |p| {
            put_contract(p, "a half rounds to even")
        });
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
        // The interface is not.
        work(&mut p, &mut c, 3, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)")
        });
        let found = out_of_frame(std::slice::from_ref(&c));
        assert_eq!(
            found,
            [OutOfFrame {
                cr: c.id(),
                moved: vec![interface(), item()]
            }]
        );
    }

    #[test]
    fn a_container_in_the_frame_frames_everything_in_it() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(contract(), ExpectKind::Target)],
        );
        work(&mut p, &mut c, 2, |p| {
            put_both(p, "a half rounds to even", "a refusal is InvalidRate");
        });
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
        // A Context in the frame covers the assets in it, and the assets that are new.
        let mut q = project();
        let mut whole = open(
            &q,
            2,
            Intent::ChangeBehavior,
            &[(Target::Context(core()), ExpectKind::Target)],
        );
        work(&mut q, &mut whole, 2, |p| {
            put_contract(p, "a half rounds to even");
            put_interface(p, 101, 11, "fn b()");
        });
        assert!(out_of_frame(std::slice::from_ref(&whole)).is_empty());
        // The Project is outside a frame that names a Context.
        work(&mut q, &mut whole, 3, |p| {
            p.add_context(context(billing(), "billing")).unwrap();
        });
        assert_eq!(
            out_of_frame(std::slice::from_ref(&whole))[0].moved,
            [Target::Project, Target::Context(billing())]
        );
    }

    #[test]
    fn a_keep_does_not_frame_and_a_request_with_no_frame_has_all_its_work_outside() {
        let mut p = project();
        let mut c = open(&p, 1, Intent::KeepBehavior, &[(clause(), ExpectKind::Keep)]);
        work(&mut p, &mut c, 2, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)")
        });
        assert_eq!(
            out_of_frame(std::slice::from_ref(&c))[0].moved,
            [interface(), item()]
        );
    }

    #[test]
    fn the_frame_is_the_request_s_own_and_a_parent_does_not_bound_its_parts() {
        let mut p = project();
        let titled = |n: u128, title: &str, parent: Option<&ChangeRequest>, t: Target| {
            let mut d = draft(&p, Intent::ChangeBehavior, &[(t, ExpectKind::Target)]);
            d.title = Title::try_from(title).unwrap();
            d.parent = parent.map(ChangeRequest::id);
            ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(n)), d)
        };
        // The parent names the contract; the part names the interface, which is outside the parent's frame.
        let parent = titled(1, "parent", None, contract());
        let mut part = titled(2, "part", Some(&parent), interface());
        work(&mut p, &mut part, 2, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)");
        });
        assert!(out_of_frame(&[parent, part]).is_empty());
    }

    #[test]
    fn a_resolved_request_is_not_reported_for_its_work() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        work(&mut p, &mut c, 2, |p| {
            put_contract(p, "a half rounds to even");
            put_interface(p, 100, 10, "fn a(x: u8)");
        });
        assert_eq!(out_of_frame(std::slice::from_ref(&c)).len(), 1);
        finish(&mut c, &[]).unwrap();
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
    }

    #[test]
    fn a_case_is_touched_by_its_harness_being_put_again() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(case(), ExpectKind::Target)],
        );
        work(&mut p, &mut c, 2, |p| {
            put_harness(p, "fn t() { assert!(true); }");
        });
        assert_eq!(
            target_progress(&c, &p, std::slice::from_ref(&c))[0].progress,
            Progress::Revised
        );
    }
}
