//! Resolving a change request against the other change requests and the Cruxes raised in it, and reading what its
//! work did.
//!
//! A change request ends by an explicit resolution. Only explicit states refuse one: a Crux raised in it that has no
//! decision (whatever the resolution), a part not resolved, a resolution pointing at a request that is not there,
//! the request resolved already. The revisions of the Project and the work recorded are never consulted by a
//! resolution; they are read afterwards, for `status` and the checks, which say where the record and the work
//! disagree.
//!
//! The work of a request is its footprints and those of the requests that are a part of it, directly or not
//! (`moved_under`). The targets a request names to revise or retire are its frame: naming a container frames
//! everything in it. The frame of one request is its own; a parent frames nothing for its parts.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use super::cite::revision_of;
use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestError, ChangeRequestState,
};
use crate::domain::entity::crux::Crux;
use crate::domain::entity::project::Project;
use crate::domain::vo::footprint::Move;
use crate::domain::vo::id::{ChangeRequestId, ContextId};
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

/// The Cruxes in `cruxes` that were raised in `cr`.
pub fn raised_in<'a>(
    cr: &'a ChangeRequest,
    cruxes: &'a [Crux],
) -> impl Iterator<Item = &'a Crux> + 'a {
    cruxes.iter().filter(move |q| q.raised_in() == cr.id())
}

/// Where `cr` stands, given the Cruxes in `cruxes` (those raised in other change requests are not read): resolved
/// when it is; decided when it has at least one Crux and every one has a decision (a withdrawal is one); open
/// otherwise.
pub fn state_of(cr: &ChangeRequest, cruxes: &[Crux]) -> ChangeRequestState {
    if cr.is_resolved() {
        return ChangeRequestState::Resolved;
    }
    let mut own = raised_in(cr, cruxes).peekable();
    if own.peek().is_some() && own.all(|q| !q.is_open()) {
        ChangeRequestState::Decided
    } else {
        ChangeRequestState::Open
    }
}

/// Every expectation `cr` could come to hold: its own, and those of every option of every Crux raised in it,
/// whichever is chosen. The options' are the kind alone: they are given a revision when one is chosen.
pub fn possible_kinds<'a>(
    cr: &'a ChangeRequest,
    cruxes: &'a [Crux],
) -> impl Iterator<Item = (Target, ExpectKind)> + 'a {
    cr.expects().iter().map(|(t, e)| (*t, e.kind)).chain(
        raised_in(cr, cruxes)
            .flat_map(|q| q.options())
            .flat_map(|o| o.expects().iter().map(|(t, k)| (*t, *k))),
    )
}

/// Why the change requests and the Cruxes read from a store do not fit together: the change request the problem
/// is about, and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the change request {cr}: {error}")]
pub struct RecordsInconsistent {
    /// The change request. For a Crux raised in one that does not exist, that one's id.
    pub cr: ChangeRequestId,
    /// What is wrong.
    pub error: ChangeRequestError,
}

/// Checks the rules that hold between the change requests `crs` and the Cruxes `cruxes` read from a store, which no
/// single record can check:
///
/// - A Crux was raised in a change request there is.
/// - A change request that is resolved as done has no open Crux. One resolved otherwise may have: it is a record of
///   what was, and `cr.awaiting-decision` goes on reporting the question.
/// - Every expectation recorded for a decision names the answer of a Crux raised in the change request, and the decision
///   of that answer asks for it.
///
/// It is the last word on a pair read together; the first refusal is returned.
pub fn check_requests_and_cruxes(
    crs: &[ChangeRequest],
    cruxes: &[Crux],
) -> Result<(), RecordsInconsistent> {
    // The Cruxes of each change request, gathered once, so that each pair is not looked for in every list.
    let mut by_request: BTreeMap<ChangeRequestId, Vec<&Crux>> =
        crs.iter().map(|c| (c.id(), Vec::new())).collect();
    for q in cruxes {
        match by_request.get_mut(&q.raised_in()) {
            Some(raised) => raised.push(q),
            None => {
                return Err(RecordsInconsistent {
                    cr: q.raised_in(),
                    error: ChangeRequestError::RaisedInUnknown(q.raised_in()),
                });
            }
        }
    }
    for cr in crs {
        let broken = |error| RecordsInconsistent { cr: cr.id(), error };
        let raised = by_request.get(&cr.id()).map_or(&[][..], Vec::as_slice);
        let done = cr.resolution().is_some_and(|r| r.kind().is_done());
        if done && let Some(open) = raised.iter().find(|q| q.is_open()) {
            return Err(broken(ChangeRequestError::Undecided(open.id())));
        }
        // Each decided expectation names the answer of one of those Cruxes, and that decision asked for it.
        if let Some(t) = cr
            .decided_expects()
            .iter()
            .find(|e| {
                !raised.iter().any(|q| {
                    q.answer_given().is_some_and(|a| a.id() == e.answer)
                        && q.decided_expects().get(&e.target) == Some(&e.kind)
                })
            })
            .map(|e| e.target)
        {
            return Err(broken(ChangeRequestError::UnaskedExpectation(t)));
        }
    }
    Ok(())
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
        .filter(|c| c.parent() == Some(parent) && !c.is_resolved())
        .map(|c| c.id())
        .collect()
}

/// Closes the record of `cr` with `resolution`. The explicit states that refuse it, in this order: `cr` resolved
/// already; a Crux of `cruxes` raised in `cr` with no decision, whatever the resolution; a resolution that points at
/// `cr` itself or at a change request `all` does not have; a change request in `all` that is a part of `cr` and not
/// resolved. Nothing else is read: not the revisions of the targets, not the work. A change request is a record of
/// its own, so `all` is every change request there is (`cr` itself may be among them), and `cruxes` is every Crux.
pub fn resolve(
    cr: &mut ChangeRequest,
    resolution: Resolution,
    all: &[ChangeRequest],
    cruxes: &[Crux],
) -> Result<(), ChangeRequestError> {
    if cr.is_resolved() {
        return Err(ChangeRequestError::AlreadyResolved);
    }
    if let Some(open) = raised_in(cr, cruxes).find(|q| q.is_open()) {
        return Err(ChangeRequestError::Undecided(open.id()));
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

/// Whether `cr` is judged as a request still open: it is not resolved, or it is the one `resolving` names, the one a
/// resolution is being made for, which the state read has resolved already. Every check of change requests that runs
/// at the `resolve` moment selects its requests with this, so none of them misses the one being resolved.
pub fn judged_open(cr: &ChangeRequest, resolving: Option<ChangeRequestId>) -> bool {
    !cr.is_resolved() || Some(cr.id()) == resolving
}

/// Every unresolved change request in `crs` that keeps a target and the target has moved on in `project`, whoever
/// moved it: a keep is judged by the revision alone. A resolved one is a record: what changes after it is another
/// change request's business. The one named by `resolving` is the one a resolution is being made for: it is judged
/// although the state read already has it resolved.
pub fn keep_moved(
    crs: &[ChangeRequest],
    project: &Project,
    resolving: Option<ChangeRequestId>,
) -> Vec<KeepMoved> {
    crs.iter()
        .filter(|c| judged_open(c, resolving))
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

/// A keep of one open change request that a target of an open change request breaks for sure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conflict {
    /// The target kept.
    pub kept: Target,
    /// The target named to be revised or retired: the kept one, or a part of it.
    pub target: Target,
    /// The change request that names it.
    pub by: ChangeRequestId,
}

/// An open change request one of whose keeps an open change request names as a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepTargeted {
    /// The change request that keeps.
    pub cr: ChangeRequestId,
    /// The keeps broken, in order.
    pub conflicts: Vec<Conflict>,
}

/// Every change request of `crs` judged open (see [`judged_open`]) that keeps a target which a change request of
/// `crs` judged open, itself included, names to revise or retire: the same target, or a part of the Contract,
/// Interface or Harness it keeps, whose revision then moves. Conflicts in the order of the keeps, then of the
/// requests, then of their targets. A target the Project does not have now conflicts by being the same alone. The
/// options of a Crux not decided yet name nothing, so they are not compared. A Context or the Project kept moves only
/// when an asset or a Context is added or removed, which revising something inside does not do, so only the same
/// target conflicts with it.
pub fn keep_targeted(
    crs: &[ChangeRequest],
    project: &Project,
    resolving: Option<ChangeRequestId>,
) -> Vec<KeepTargeted> {
    let open: Vec<&ChangeRequest> = crs.iter().filter(|c| judged_open(c, resolving)).collect();
    // Each target named by an open request, with the Contract, Interface or Harness that holds it now.
    let named: Vec<(ChangeRequestId, Target, Option<Target>)> = open
        .iter()
        .flat_map(|c| {
            frame(c)
                .into_iter()
                .map(|t| (c.id(), t, project.holder_of(t)))
        })
        .collect();
    open.iter()
        .filter_map(|keeper| {
            let conflicts: Vec<Conflict> = keeper
                .expectations()
                .filter(|(_, e)| e.kind == ExpectKind::Keep)
                .flat_map(|(kept, _)| {
                    named
                        .iter()
                        .filter(move |(_, target, holder)| *target == kept || *holder == Some(kept))
                        .map(move |&(by, target, _)| Conflict { kept, target, by })
                })
                .collect();
            (!conflicts.is_empty()).then(|| KeepTargeted {
                cr: keeper.id(),
                conflicts,
            })
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

/// The Contexts `cr` is about in `project`: the ones that hold a target it names to revise or retire (its frame) or
/// one it keeps. The options of a question not yet decided name nothing: until one is chosen it is not settled which
/// targets the request is about. A target the Project does not have now belongs to no Context.
pub fn contexts_in(cr: &ChangeRequest, project: &Project) -> BTreeSet<ContextId> {
    cr.expectations()
        .filter_map(|(target, _)| project.context_of_target(target))
        .collect()
}

/// An unresolved change request whose work moved targets outside its frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutOfFrame {
    /// The change request the work was applied for.
    pub cr: ChangeRequestId,
    /// The targets it moved outside its frame, in order.
    pub moved: Vec<Target>,
    /// Each target it moved outside its frame with the store revision of the change set that moved it, in order.
    /// A target that several change sets moved is in it once for each. A footprint never changes, so the pair says
    /// which work did it: the same target moved by a later change set is another pair.
    pub by_work: Vec<(Target, u64)>,
}

/// Every unresolved change request in `crs` with work that moved a target outside its own frame (the frames of the
/// requests it is a part of do not count: a frame is per request). A target is inside the frame when it is, or is
/// inside, a target the frame names. A container that moved only because something inside it moved within the
/// frame (a Contract that took a new Clause, the Context that holds a new asset) is not outside. The work of a part
/// is judged for the part, not again for its parent. A request with no frame has all its work outside.
///
/// A moved Case is also inside when any one of the Items and Clauses it cites, as the footprint recorded them, is
/// inside the frame (a citation is inside when it is, or is inside, a target the frame names): the Cases that follow
/// a Clause are the work of revising it. A footprint is judged from itself alone, so the Project is not read; a
/// footprint recorded without citations is judged by containment only. The request named by `resolving` is judged
/// although it is resolved: it is the one a resolution is being made for.
pub fn out_of_frame(crs: &[ChangeRequest], resolving: Option<ChangeRequestId>) -> Vec<OutOfFrame> {
    crs.iter()
        .filter(|c| judged_open(c, resolving) && !c.work().is_empty())
        .filter_map(|cr| {
            let frame = frame(cr);
            // Each change set on its own: a container is covered by what moved inside it in the same set.
            let mut moved = BTreeSet::new();
            let mut by_work = BTreeSet::new();
            for footprint in cr.work() {
                let inside = |m: &Move| {
                    frame.iter().any(|container| {
                        m.is_inside(*container) || footprint.cites_inside(m.target(), *container)
                    })
                };
                let mut covered = BTreeSet::new();
                for m in footprint.moves().iter().filter(|m| inside(m)) {
                    covered.insert(m.target());
                    covered.extend(m.within().iter().copied());
                }
                for t in footprint.targets().filter(|t| !covered.contains(t)) {
                    moved.insert(t);
                    by_work.insert((t, footprint.store_revision()));
                }
            }
            (!moved.is_empty()).then(|| OutOfFrame {
                cr: cr.id(),
                moved: moved.into_iter().collect(),
                by_work: by_work.into_iter().collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::entity::change_request::{ChangeRequestDraft, DecidedExpectation};
    use crate::domain::entity::context::Context;
    use crate::domain::entity::crux::CruxDraft;
    use crate::domain::service::crux::decide;
    use crate::domain::service::footprint::footprint_between;
    use crate::domain::vo::answer::{Answer, Basis, DecisionMaker};
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::crux::{CruxKind, CruxOption, Decision};
    use crate::domain::vo::footprint::{Citations, Footprint};
    use crate::domain::vo::id::{
        AnswerId, CaseId, ClauseId, ContextId, ContractId, CruxId, CruxOptionId, HarnessId,
        InterfaceId, ItemId, ProjectId, test_uuid,
    };
    use crate::domain::vo::intent::Intent;
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::resolution::{Reopening, ResolutionKind, Transition};
    use crate::domain::vo::source::Source;
    use crate::domain::vo::timestamp::Timestamp;
    use crate::domain::vo::title::Title;

    /// [`super::keep_moved`] for the change requests given.
    fn keep_moved<const N: usize>(crs: [&ChangeRequest; N], p: &Project) -> Vec<KeepMoved> {
        let all: Vec<ChangeRequest> = crs.into_iter().cloned().collect();
        super::keep_moved(&all, p, None)
    }

    /// [`super::out_of_frame`] with no request being resolved.
    fn out_of_frame(crs: &[ChangeRequest]) -> Vec<OutOfFrame> {
        super::out_of_frame(crs, None)
    }

    /// What a change set `for` `c` does: `change` is made to `p`, and what it moved is recorded as work of `c`.
    fn work(p: &mut Project, c: &mut ChangeRequest, at: u64, change: impl FnOnce(&mut Project)) {
        let before = p.clone();
        change(p);
        c.record_work(footprint_between(at, &before, p)).unwrap();
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
        put_cases(p, body, &[(40, "t", &[])]);
    }

    /// The Harness with the Cases given, each its number, its name, and the numbers of the Clauses it cites besides
    /// the Item `a`.
    fn put_cases(p: &mut Project, body: &str, cases: &[(u128, &str, &[u128])]) {
        p.put_harness(
            core(),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new(format!("{body}\n")),
            cases
                .iter()
                .map(|(n, name, clauses)| {
                    (
                        CaseId::from_uuid(test_uuid(*n)),
                        CaseName::try_from(*name).unwrap(),
                        LineRange::new(1, 1).unwrap(),
                        CaseCites::new(
                            BTreeMap::from([(ItemId::from_uuid(test_uuid(10)), Revision::first())]),
                            clauses
                                .iter()
                                .map(|c| (ClauseId::from_uuid(test_uuid(*c)), Revision::first()))
                                .collect(),
                        )
                        .unwrap(),
                    )
                })
                .collect(),
        )
        .unwrap();
    }

    /// The contract with its second clause alone.
    fn put_second_only(p: &mut Project) {
        p.put_contract(
            core(),
            ContractId::from_uuid(test_uuid(300)),
            Title::try_from("T").unwrap(),
            vec![(
                ClauseId::from_uuid(test_uuid(31)),
                ClauseContent::new(
                    ClauseLabel::try_from("C2").unwrap(),
                    ClauseText::try_from("a refusal is InvalidDiscount").unwrap(),
                ),
            )],
        )
        .unwrap();
    }

    fn harness() -> Target {
        Target::Harness(HarnessId::from_uuid(test_uuid(400)))
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

    fn draft(p: &Project, intent: Intent, wanted: &[(Target, ExpectKind)]) -> ChangeRequestDraft {
        ChangeRequestDraft {
            title: Title::try_from("How is a half rounded?").unwrap(),
            situation: None,
            intent,
            parent: None,
            issuer: None,
            expects: expectations_now(p, wanted.iter().copied()),
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

    /// A settled change (C1, its own target).
    fn cr(p: &Project) -> ChangeRequest {
        ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(1)),
            draft(p, Intent::ChangeBehavior, &[(clause(), ExpectKind::Target)]),
        )
    }

    fn option_id(n: u128) -> CruxOptionId {
        CruxOptionId::from_uuid(test_uuid(1000 + n))
    }

    /// A Crux numbered `n` raised in `c`, with the options `options` (the choice, and what it expects of targets);
    /// the option `k` (from 0) has the id `option_id(k)`.
    fn crux_with(
        c: &ChangeRequest,
        n: u128,
        kind: CruxKind,
        options: Vec<(&str, BTreeMap<Target, ExpectKind>)>,
    ) -> Crux {
        Crux::raise(
            CruxId::from_uuid(test_uuid(n)),
            CruxDraft {
                raised_in: c.id(),
                findings: Vec::new(),
                kind,
                question: Prose::try_from("which one?").unwrap(),
                options: options
                    .into_iter()
                    .enumerate()
                    .map(|(k, (choice, expects))| {
                        CruxOption::new(
                            option_id(k as u128),
                            choice_of(choice),
                            None,
                            expects,
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

    fn choice_of(s: &str) -> Choice {
        Choice::try_from(s).unwrap()
    }

    /// The question about a detail (which error type; C2 for a new one): `up` (0) asks nothing, `to even` (1)
    /// asks for C2.
    fn asking(c: &ChangeRequest, n: u128) -> Crux {
        crux_with(
            c,
            n,
            CruxKind::Ambiguity,
            vec![
                ("up", BTreeMap::new()),
                ("to even", BTreeMap::from([(detail(), ExpectKind::Target)])),
            ],
        )
    }

    /// An answer on an arbitrary basis, by a person, choosing the option `k`.
    fn chosen(k: u128) -> Answer {
        Answer::new(
            AnswerId::from_uuid(test_uuid(900)),
            Decision::Chosen {
                option: option_id(k),
                reason: None,
            },
            Basis::Arbitrary {
                reason: Prose::try_from("nothing derives it").unwrap(),
            },
            DecisionMaker::person("alice").unwrap(),
            Timestamp::from_millis(5),
        )
    }

    /// `crux` decided for `c`: the option `k` chosen.
    fn decided(c: &mut ChangeRequest, crux: &mut Crux, k: u128, p: &Project) {
        decide(crux, c, chosen(k), p, None).unwrap();
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

    /// Resolves `c` as done against the change requests `all`, with no Crux.
    fn finish(c: &mut ChangeRequest, all: &[ChangeRequest]) -> Result<(), ChangeRequestError> {
        resolve(c, done(), all, &[])
    }

    // The Crux.

    #[test]
    fn a_request_is_decided_when_it_has_cruxes_and_every_one_has_a_decision() {
        let p = project();
        let mut c = cr(&p);
        let mut other = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(2)),
            draft(&p, Intent::ChangeBehavior, &[]),
        );
        // No Crux: open. A Crux raised in another request does not count.
        assert_eq!(state_of(&c, &[]), ChangeRequestState::Open);
        let mut first = asking(&c, 500);
        let mut second = asking(&c, 501);
        let mut elsewhere = asking(&other, 502);
        assert_eq!(
            state_of(&c, &[first.clone(), elsewhere.clone()]),
            ChangeRequestState::Open
        );
        decide(&mut elsewhere, &mut other, chosen(0), &p, None).unwrap();
        // One of two decided: open still.
        decided(&mut c, &mut first, 0, &p);
        let all = [first.clone(), second.clone(), elsewhere.clone()];
        assert_eq!(state_of(&c, &all), ChangeRequestState::Open);
        assert_eq!(state_of(&other, &all), ChangeRequestState::Decided);
        // Every one decided, a withdrawal counted: decided.
        decide(
            &mut second,
            &mut c,
            Answer::new(
                AnswerId::from_uuid(test_uuid(901)),
                Decision::Withdrawn,
                Basis::Arbitrary {
                    reason: Prose::try_from("not needed").unwrap(),
                },
                DecisionMaker::person("alice").unwrap(),
                Timestamp::from_millis(6),
            ),
            &p,
            None,
        )
        .unwrap();
        let all = [first, second, elsewhere];
        assert_eq!(state_of(&c, &all), ChangeRequestState::Decided);
        finish_with(&mut c, &[], &all).unwrap();
        assert_eq!(state_of(&c, &all), ChangeRequestState::Resolved);
    }

    /// Resolves `c` as done against the change requests `all` and the Cruxes `cruxes`.
    fn finish_with(
        c: &mut ChangeRequest,
        all: &[ChangeRequest],
        cruxes: &[Crux],
    ) -> Result<(), ChangeRequestError> {
        resolve(c, done(), all, cruxes)
    }

    #[test]
    fn a_request_with_a_crux_open_resolves_in_no_way_until_it_is_decided() {
        let p = project();
        let mut c = cr(&p);
        let mut q = asking(&c, 500);
        for kind in [
            ResolutionKind::Done,
            ResolutionKind::WontDo,
            ResolutionKind::Invalid,
        ] {
            assert_eq!(
                resolve(&mut c, resolution(kind), &[], std::slice::from_ref(&q)),
                Err(ChangeRequestError::Undecided(q.id()))
            );
        }
        assert!(!c.is_resolved());
        // A request for which the Crux is raised in another is not held by it.
        let mut other = ChangeRequest::open(
            ChangeRequestId::from_uuid(test_uuid(2)),
            draft(&p, Intent::ChangeBehavior, &[]),
        );
        resolve(&mut other, done(), &[], std::slice::from_ref(&q)).unwrap();
        decided(&mut c, &mut q, 0, &p);
        finish_with(&mut c, &[], std::slice::from_ref(&q)).unwrap();
        assert!(c.is_resolved());
    }

    #[test]
    fn a_request_names_what_every_option_of_its_cruxes_could_ask() {
        let p = project();
        let c = cr(&p);
        let q = asking(&c, 500);
        let mut kinds: Vec<_> = possible_kinds(&c, std::slice::from_ref(&q)).collect();
        kinds.sort();
        assert_eq!(
            kinds,
            [
                (clause(), ExpectKind::Target),
                (detail(), ExpectKind::Target)
            ]
        );
        // Without the Crux, only its own.
        assert_eq!(possible_kinds(&c, &[]).count(), 1);
    }

    #[test]
    fn requests_and_cruxes_that_do_not_fit_together_are_refused() {
        let p = project();
        let mut c = cr(&p);
        let mut q = asking(&c, 500);
        assert_eq!(
            check_requests_and_cruxes(&[c.clone()], &[q.clone()]),
            Ok(())
        );
        // A Crux raised in a request there is not.
        let ghost = ChangeRequestId::from_uuid(test_uuid(77));
        let lost = ChangeRequest::open(ghost, draft(&p, Intent::ChangeBehavior, &[]));
        let stray = asking(&lost, 501);
        assert_eq!(
            check_requests_and_cruxes(&[c.clone()], &[stray]),
            Err(RecordsInconsistent {
                cr: ghost,
                error: ChangeRequestError::RaisedInUnknown(ghost)
            })
        );
        // Resolved as done with a Crux open: refused. Resolved otherwise: a record of what was.
        let mut dropped = c.clone();
        dropped.resolve(resolution(ResolutionKind::WontDo)).unwrap();
        assert_eq!(check_requests_and_cruxes(&[dropped], &[q.clone()]), Ok(()));
        let mut closed = c.clone();
        closed.resolve(done()).unwrap();
        assert_eq!(
            check_requests_and_cruxes(&[closed.clone()], &[q.clone()]),
            Err(RecordsInconsistent {
                cr: c.id(),
                error: ChangeRequestError::Undecided(q.id())
            })
        );
        // Once decided, it is not.
        decided(&mut c, &mut q, 1, &p);
        assert_eq!(
            check_requests_and_cruxes(&[c.clone()], &[q.clone()]),
            Ok(())
        );
        // A request reopened after it was done, with a question raised since, is held by that question only.
        let mut reopened = closed;
        reopened
            .reopen(Reopening::new(
                Prose::try_from("a new question").unwrap(),
                DecisionMaker::person("bob").unwrap(),
                Timestamp::from_millis(8),
            ))
            .unwrap();
        let fresh = asking(&reopened, 502);
        assert_eq!(check_requests_and_cruxes(&[reopened], &[fresh]), Ok(()));
        // An expectation recorded for a decision that no decision asks.
        let recorded = ChangeRequest::restore(
            c.id(),
            draft(
                &p,
                Intent::ChangeBehavior,
                &[(clause(), ExpectKind::Target)],
            ),
            vec![DecidedExpectation {
                target: detail(),
                kind: ExpectKind::Keep,
                recorded: Revision::first(),
                answer: q.answer_given().unwrap().id(),
            }],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        // `to even` asks C2 to be revised, not kept.
        assert_eq!(
            check_requests_and_cruxes(&[recorded], &[q.clone()]),
            Err(RecordsInconsistent {
                cr: c.id(),
                error: ChangeRequestError::UnaskedExpectation(detail())
            })
        );
        // The kind is the one the decision asked, but the answer it names is none of the Cruxes raised in the request.
        let nameless = ChangeRequest::restore(
            c.id(),
            draft(
                &p,
                Intent::ChangeBehavior,
                &[(clause(), ExpectKind::Target)],
            ),
            vec![DecidedExpectation {
                target: detail(),
                kind: ExpectKind::Target,
                recorded: Revision::first(),
                answer: AnswerId::from_uuid(test_uuid(777)),
            }],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            check_requests_and_cruxes(&[nameless], &[q.clone()]),
            Err(RecordsInconsistent {
                cr: c.id(),
                error: ChangeRequestError::UnaskedExpectation(detail())
            })
        );
        // The kind that was asked is what the decision recorded.
        assert_eq!(c.decided_expects()[0].kind, ExpectKind::Target);
    }

    // Resolving looks at explicit states only.

    #[test]
    fn a_request_whose_targets_were_never_touched_resolves_as_done() {
        let p = project();
        let mut c = cr(&p);
        assert_eq!(
            target_progress(&c, &p, &[]),
            [TargetProgress {
                target: clause(),
                progress: Progress::NotTouched
            }]
        );
        // The revision of the target is not consulted, and neither is the work.
        finish(&mut c, &[]).unwrap();
        assert!(c.is_resolved());
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
        assert!(mine.is_resolved());
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
                resolve(&mut parent, resolution(kind), &all, &[]),
                Err(ChangeRequestError::ChildrenNotResolved {
                    children: vec![part.id()]
                })
            );
        }
        assert!(!parent.is_resolved());
        // A part ends any way it likes, and then its parent may close.
        resolve(&mut part, resolution(ResolutionKind::WontDo), &all, &[]).unwrap();
        let all = [parent.clone(), part];
        assert!(open_parts(parent.id(), &all).is_empty());
        finish(&mut parent, &all).unwrap();
        assert!(parent.is_resolved());
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
                resolve(&mut c, resolution(kind), &all, &[]),
                Err(ChangeRequestError::ReferenceUnknown(ghost))
            );
            assert!(!c.is_resolved());
        }
        for kind in kinds(all[0].id()) {
            let mut c = all[0].clone();
            assert_eq!(
                resolve(&mut c, resolution(kind), &all, &[]),
                Err(ChangeRequestError::ReferencesItself)
            );
        }
        for kind in kinds(other.id()) {
            let mut c = all[0].clone();
            resolve(&mut c, resolution(kind), &all, &[]).unwrap();
            assert_eq!(c.resolution().map(|r| r.kind()), Some(kind));
        }
    }

    #[test]
    fn a_resolved_request_is_resolved_once_until_it_is_reopened() {
        let p = project();
        let mut c = cr(&p);
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
        assert!(!c.is_resolved());
        resolve(&mut c, resolution(ResolutionKind::WontDo), &[], &[]).unwrap();
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
        resolve(&mut dropped, resolution(ResolutionKind::WontDo), &[], &[]).unwrap();
        assert!(targets_untouched(&[dropped.clone()], &p).is_empty());
        assert!(resolved_without_work(&[dropped]).is_empty());
    }

    #[test]
    fn a_change_set_that_moved_nothing_is_no_work() {
        let p = project();
        let mut c = cr(&p);
        c.record_work(Footprint::new(2, Vec::new())).unwrap();
        assert!(!has_work(&c, &[]));
        finish(&mut c, &[]).unwrap();
        assert_eq!(resolved_without_work(&[c.clone()]), [c.id()]);
    }

    #[test]
    fn what_the_chosen_option_names_is_a_target_too() {
        let p = project();
        let mut c = cr(&p);
        let mut q = asking(&c, 500);
        assert_eq!(target_progress(&c, &p, &[]).len(), 1);
        decided(&mut c, &mut q, 1, &p);
        assert_eq!(
            target_progress(&c, &p, &[])
                .iter()
                .map(|t| t.target)
                .collect::<Vec<_>>(),
            [clause(), detail()]
        );
        assert_eq!(frame(&c), BTreeSet::from([clause(), detail()]));
    }

    #[test]
    fn a_request_is_about_the_contexts_of_its_frame_and_its_keeps_not_of_an_undecided_option() {
        let mut p = project();
        p.add_context(context(billing(), "billing")).unwrap();
        let billing_context = Target::Context(billing());
        // The frame is in core, the keep is billing itself.
        let c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[
                (clause(), ExpectKind::Target),
                (billing_context, ExpectKind::Keep),
            ],
        );
        assert_eq!(contexts_in(&c, &p), BTreeSet::from([core(), billing()]));
        // A request that names nothing is about no Context.
        let bare = open(&p, 2, Intent::ChangeBehavior, &[]);
        assert!(contexts_in(&bare, &p).is_empty());
        // An option names billing, but nothing is chosen: until it is, it is not settled what the request is about.
        let mut asked = cr(&p);
        let mut crux = crux_with(
            &asked,
            500,
            CruxKind::Conflict,
            vec![(
                "move it",
                BTreeMap::from([(billing_context, ExpectKind::Target)]),
            )],
        );
        assert_eq!(contexts_in(&asked, &p), BTreeSet::from([core()]));
        decided(&mut asked, &mut crux, 0, &p);
        assert_eq!(contexts_in(&asked, &p), BTreeSet::from([core(), billing()]));
        // The Project is no Context's.
        let project_wide = open(
            &p,
            3,
            Intent::ChangeBehavior,
            &[(Target::Project, ExpectKind::Target)],
        );
        assert!(contexts_in(&project_wide, &p).is_empty());
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
            super::keep_moved(&all, &p, None),
            [KeepMoved {
                cr: mine.id(),
                moved: vec![clause()]
            }]
        );
    }

    #[test]
    fn an_option_that_keeps_a_target_is_a_keep_once_chosen() {
        let p = project();
        let mut c = cr(&p);
        let mut crux = crux_with(
            &c,
            500,
            CruxKind::Conflict,
            vec![
                ("drop C2", BTreeMap::from([(detail(), ExpectKind::Target)])),
                ("keep both", BTreeMap::from([(detail(), ExpectKind::Keep)])),
            ],
        );
        decided(&mut c, &mut crux, 1, &p);
        assert_eq!(c.decided_expects()[0].kind, ExpectKind::Keep);
        assert_eq!(keep_progress(&c, &p).len(), 1);
        assert_eq!(frame(&c), BTreeSet::from([clause()]));
    }

    // A keep that an open request's target breaks.

    fn conflict(kept: Target, target: Target, by: &ChangeRequest) -> Conflict {
        Conflict {
            kept,
            target,
            by: by.id(),
        }
    }

    #[test]
    fn a_target_named_by_another_open_request_breaks_the_keep_of_the_same_target() {
        let p = project();
        let a = open(&p, 1, Intent::KeepBehavior, &[(clause(), ExpectKind::Keep)]);
        let b = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        let all = [a.clone(), b.clone()];
        assert_eq!(
            super::keep_targeted(&all, &p, None),
            [KeepTargeted {
                cr: a.id(),
                conflicts: vec![conflict(clause(), clause(), &b)]
            }]
        );
        // Two keeps that are not named, and a target that is not kept, tell nothing.
        let c = open(&p, 3, Intent::KeepBehavior, &[(detail(), ExpectKind::Keep)]);
        assert!(super::keep_targeted(&[b, c], &p, None).is_empty());
    }

    #[test]
    fn a_part_of_a_kept_contract_interface_or_harness_that_another_request_names_breaks_the_keep() {
        let p = project();
        let keeps =
            |n: u128, kept: Target| open(&p, n, Intent::KeepBehavior, &[(kept, ExpectKind::Keep)]);
        let names = |n: u128, target: Target| {
            open(
                &p,
                n,
                Intent::ChangeBehavior,
                &[(target, ExpectKind::Target)],
            )
        };
        for (kept, part) in [
            (interface(), item()),
            (contract(), clause()),
            (contract(), detail()),
            (harness(), case()),
        ] {
            let a = keeps(1, kept);
            let b = names(2, part);
            assert_eq!(
                super::keep_targeted(&[a.clone(), b.clone()], &p, None),
                [KeepTargeted {
                    cr: a.id(),
                    conflicts: vec![conflict(kept, part, &b)]
                }]
            );
        }
    }

    #[test]
    fn what_is_not_moved_for_sure_by_a_target_is_not_a_conflict() {
        let p = project();
        let keeps =
            |n: u128, kept: Target| open(&p, n, Intent::KeepBehavior, &[(kept, ExpectKind::Keep)]);
        let names = |n: u128, target: Target| {
            open(
                &p,
                n,
                Intent::ChangeBehavior,
                &[(target, ExpectKind::Target)],
            )
        };
        // A Context moves when an asset is added or removed, not when something inside it is revised.
        let context = Target::Context(core());
        for part in [interface(), item(), clause(), contract(), case()] {
            assert!(
                super::keep_targeted(&[keeps(1, context), names(2, part)], &p, None).is_empty()
            );
        }
        // A kept Item is not moved by its Interface being revised, nor a kept Clause by its Contract.
        assert!(
            super::keep_targeted(&[keeps(1, item()), names(2, interface())], &p, None).is_empty()
        );
        assert!(
            super::keep_targeted(&[keeps(1, clause()), names(2, contract())], &p, None).is_empty()
        );
        // The part of another Contract does not break the keep of this one.
        assert!(
            super::keep_targeted(&[keeps(1, interface()), names(2, clause())], &p, None).is_empty()
        );
        // The same Context is the same target.
        assert_eq!(
            super::keep_targeted(&[keeps(1, context), names(2, context)], &p, None).len(),
            1
        );
    }

    #[test]
    fn a_target_the_project_does_not_have_conflicts_only_as_the_same_target() {
        let p = project();
        let a = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(interface(), ExpectKind::Keep), (item(), ExpectKind::Keep)],
        );
        let b = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(item(), ExpectKind::Target)],
        );
        let gone = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        let all = [a.clone(), b.clone()];
        assert_eq!(
            super::keep_targeted(&all, &gone, None),
            [KeepTargeted {
                cr: a.id(),
                conflicts: vec![conflict(item(), item(), &b)]
            }]
        );
    }

    #[test]
    fn only_requests_judged_open_are_compared_on_either_side() {
        let p = project();
        let mut a = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(interface(), ExpectKind::Keep)],
        );
        let mut b = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(item(), ExpectKind::Target)],
        );
        let both = |a: &ChangeRequest, b: &ChangeRequest| [a.clone(), b.clone()];
        assert_eq!(super::keep_targeted(&both(&a, &b), &p, None).len(), 1);
        // The one that targets is a record.
        finish(&mut b, &[]).unwrap();
        assert!(super::keep_targeted(&both(&a, &b), &p, None).is_empty());
        // Unless it is the one being resolved.
        assert_eq!(
            super::keep_targeted(&both(&a, &b), &p, Some(b.id())),
            [KeepTargeted {
                cr: a.id(),
                conflicts: vec![conflict(interface(), item(), &b)]
            }]
        );
        // The one that keeps is a record.
        let mut b = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(item(), ExpectKind::Target)],
        );
        finish(&mut a, &[]).unwrap();
        assert!(super::keep_targeted(&both(&a, &b), &p, None).is_empty());
        assert_eq!(
            super::keep_targeted(&both(&a, &b), &p, Some(a.id())).len(),
            1
        );
        // Both resolved: the one being resolved is judged against the other, which is a record.
        finish(&mut b, &[]).unwrap();
        assert!(super::keep_targeted(&both(&a, &b), &p, Some(a.id())).is_empty());
    }

    #[test]
    fn a_request_that_keeps_a_container_and_names_a_part_of_it_conflicts_with_itself() {
        let p = project();
        let a = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[
                (interface(), ExpectKind::Keep),
                (item(), ExpectKind::Target),
            ],
        );
        assert_eq!(
            super::keep_targeted(std::slice::from_ref(&a), &p, None),
            [KeepTargeted {
                cr: a.id(),
                conflicts: vec![conflict(interface(), item(), &a)]
            }]
        );
    }

    #[test]
    fn what_a_decision_expects_is_compared_and_the_options_not_decided_are_not() {
        let p = project();
        let a = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(interface(), ExpectKind::Keep)],
        );
        let mut b = open(&p, 2, Intent::ChangeBehavior, &[]);
        let mut crux = crux_with(
            &b,
            500,
            CruxKind::Conflict,
            vec![
                ("leave it", BTreeMap::new()),
                ("revise it", BTreeMap::from([(item(), ExpectKind::Target)])),
            ],
        );
        assert!(super::keep_targeted(&[a.clone(), b.clone()], &p, None).is_empty());
        decided(&mut b, &mut crux, 1, &p);
        assert_eq!(
            super::keep_targeted(&[a.clone(), b.clone()], &p, None),
            [KeepTargeted {
                cr: a.id(),
                conflicts: vec![conflict(interface(), item(), &b)]
            }]
        );
        // The decision keeping a target is a keep as well, for another request's target.
        let c = open(
            &p,
            3,
            Intent::ChangeBehavior,
            &[(detail(), ExpectKind::Target)],
        );
        let mut d = open(&p, 4, Intent::KeepBehavior, &[]);
        let mut keeping = crux_with(
            &d,
            501,
            CruxKind::Conflict,
            vec![
                ("drop C2", BTreeMap::new()),
                ("keep C2", BTreeMap::from([(detail(), ExpectKind::Keep)])),
            ],
        );
        decided(&mut d, &mut keeping, 1, &p);
        assert_eq!(
            super::keep_targeted(&[c.clone(), d.clone()], &p, None),
            [KeepTargeted {
                cr: d.id(),
                conflicts: vec![conflict(detail(), detail(), &c)]
            }]
        );
    }

    #[test]
    fn conflicts_come_in_the_order_of_the_keeps_then_of_the_requests_then_of_their_targets() {
        let p = project();
        let a = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[
                (interface(), ExpectKind::Keep),
                (contract(), ExpectKind::Keep),
            ],
        );
        let b = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(item(), ExpectKind::Target)],
        );
        let c = open(
            &p,
            3,
            Intent::ChangeBehavior,
            &[(item(), ExpectKind::Target), (clause(), ExpectKind::Target)],
        );
        let keeper = open(
            &p,
            4,
            Intent::KeepBehavior,
            &[(harness(), ExpectKind::Keep)],
        );
        let d = open(
            &p,
            5,
            Intent::ChangeBehavior,
            &[(case(), ExpectKind::Target)],
        );
        let found = super::keep_targeted(
            &[keeper.clone(), a.clone(), b.clone(), c.clone(), d.clone()],
            &p,
            None,
        );
        assert_eq!(
            found,
            [
                KeepTargeted {
                    cr: keeper.id(),
                    conflicts: vec![conflict(harness(), case(), &d)]
                },
                KeepTargeted {
                    cr: a.id(),
                    conflicts: vec![
                        conflict(contract(), clause(), &c),
                        conflict(interface(), item(), &b),
                        conflict(interface(), item(), &c),
                    ]
                },
            ]
        );
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
                moved: vec![interface(), item()],
                by_work: vec![(interface(), 3), (item(), 3)],
            }]
        );
    }

    #[test]
    fn a_target_moved_outside_the_frame_by_two_change_sets_is_listed_for_each() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        work(&mut p, &mut c, 3, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)")
        });
        work(&mut p, &mut c, 5, |p| {
            put_interface(p, 100, 10, "fn a(x: u16)")
        });
        let found = out_of_frame(std::slice::from_ref(&c));
        assert_eq!(found[0].moved, [interface(), item()]);
        assert_eq!(
            found[0].by_work,
            [(interface(), 3), (interface(), 5), (item(), 3), (item(), 5)]
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

    // Frame: the Cases that follow what the request frames.

    /// A Project whose Case `t` (40) cites the Clause C1 (30) and the Item `a`.
    fn project_with_a_case_on_c1() -> Project {
        let mut p = project();
        put_cases(&mut p, "fn t() {}", &[(40, "t", &[30])]);
        p
    }

    #[test]
    fn the_cases_that_follow_a_framed_contract_are_inside_its_frame() {
        let mut p = project_with_a_case_on_c1();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(contract(), ExpectKind::Target)],
        );
        // the Harness is put again: the Case that cites C1 of the Contract changes, and one is added
        work(&mut p, &mut c, 2, |p| {
            put_cases(
                p,
                "fn t() { 1 }\nfn u() {}",
                &[(40, "t", &[30]), (41, "u", &[31])],
            )
        });
        let case_moves: Vec<Target> = c.work()[0]
            .targets()
            .filter(|t| matches!(t, Target::Case(_)))
            .collect();
        assert_eq!(case_moves.len(), 2, "{:?}", c.work());
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
    }

    #[test]
    fn a_case_that_cites_nothing_the_frame_holds_is_outside_it() {
        // the Case cites the Item `a` and, from now on, C2; the request is about C1 alone
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        work(&mut p, &mut c, 2, |p| {
            put_cases(p, "fn t() { 1 }", &[(40, "t", &[31])])
        });
        let found = out_of_frame(std::slice::from_ref(&c));
        assert_eq!(found[0].moved, [harness(), case()]);
    }

    #[test]
    fn one_citation_inside_the_frame_is_enough() {
        let mut p = project();
        put_cases(&mut p, "fn t() {}", &[(40, "t", &[31])]);
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        // the Case cites C2 (outside) and the Item `a` (outside) before, and C1 (inside) as well after
        work(&mut p, &mut c, 2, |p| {
            put_cases(p, "fn t() { 1 }", &[(40, "t", &[30, 31])])
        });
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
    }

    #[test]
    fn a_case_that_is_retired_stays_inside_through_what_it_cited() {
        let mut p = project_with_a_case_on_c1();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        // the Harness goes with its Case, which is not in the Project any more to say what it cited
        work(&mut p, &mut c, 2, |p| {
            p.retire_harness(core(), HarnessId::from_uuid(test_uuid(400)))
                .unwrap();
        });
        assert!(c.work()[0].targets().any(|t| t == case()));
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
    }

    #[test]
    fn a_change_set_that_retires_a_framed_clause_and_moves_its_cases_off_it_keeps_them_inside() {
        let mut p = project_with_a_case_on_c1();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target)],
        );
        // the Case is moved to C2 and C1 goes, in one change set: the Case is judged by what it cited before it too
        work(&mut p, &mut c, 2, |p| {
            put_cases(p, "fn t() { 1 }", &[(40, "t", &[31])]);
            put_second_only(p);
        });
        assert!(c.work()[0].targets().any(|t| t == case()));
        assert!(c.work()[0].targets().any(|t| t == clause()));
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
    }

    #[test]
    fn a_footprint_recorded_without_citations_is_judged_by_containment_alone() {
        let p = project_with_a_case_on_c1();
        let within = vec![Target::Project, Target::Context(core()), harness()];
        let frame = [(clause(), ExpectKind::Target)];
        let mut old = open(&p, 1, Intent::ChangeBehavior, &frame);
        old.record_work(Footprint::new(2, vec![Move::new(case(), within.clone())]))
            .unwrap();
        assert_eq!(out_of_frame(std::slice::from_ref(&old))[0].moved, [case()]);
        // the same move with what the Case cited is inside
        let mut new = open(&p, 2, Intent::ChangeBehavior, &frame);
        new.record_work(
            Footprint::new(2, vec![Move::new(case(), within)]).with_citations(BTreeMap::from([(
                CaseId::from_uuid(test_uuid(40)),
                Citations::new(
                    vec![clause()],
                    vec![Target::Project, Target::Context(core()), contract()],
                ),
            )])),
        )
        .unwrap();
        assert!(out_of_frame(std::slice::from_ref(&new)).is_empty());
    }

    // Resolving: the request being resolved is judged although the state read has it resolved.

    #[test]
    fn the_request_being_resolved_is_judged_for_its_frame_and_its_keeps() {
        let mut p = project();
        let mut mine = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Target), (detail(), ExpectKind::Keep)],
        );
        work(&mut p, &mut mine, 2, |p| {
            put_contract(p, "a half rounds to even");
            put_interface(p, 100, 10, "fn a(x: u8)");
        });
        // the Clause it keeps moved, whoever moved it: the revision alone
        work(&mut p, &mut mine, 3, |p| {
            put_both(p, "a half rounds to even", "a refusal is InvalidRate");
        });
        finish(&mut mine, &[]).unwrap();
        let all = [mine.clone()];
        // as a record it is not judged ...
        assert!(super::out_of_frame(&all, None).is_empty());
        assert!(super::keep_moved(&all, &p, None).is_empty());
        // ... but it is when it is the one being resolved, and no other is
        let other = ChangeRequestId::from_uuid(test_uuid(2));
        assert!(super::out_of_frame(&all, Some(other)).is_empty());
        assert!(super::keep_moved(&all, &p, Some(other)).is_empty());
        assert_eq!(super::out_of_frame(&all, Some(mine.id())).len(), 1);
        assert_eq!(
            super::keep_moved(&all, &p, Some(mine.id())),
            [KeepMoved {
                cr: mine.id(),
                moved: vec![detail()]
            }]
        );
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
