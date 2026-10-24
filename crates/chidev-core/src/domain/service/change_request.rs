//! Deciding and resolving a change request against the Project, and finding the ones the targets have not
//! followed.
//!
//! An expectation is met by comparing the revision its target has now (`revision_of`) with the one it recorded:
//! a change when the revision is past it, a removal when the target is gone, a keep while it is the same. A change
//! or a removal is also owed to the request: it counts only when the target moved in the work of the request or of
//! a part of it (the footprints of the change sets applied for it), so a target that someone else moved, or that
//! moved in a change set applied for no request, does not meet it. A keep guards against anyone.
//!
//! The targets a request expects to change or remove are its frame: naming a container frames everything in it. The
//! work of a request is to stay inside its frame, and inside the frame of every request it is a part of that has
//! one (a parent only aggregates: it need not have a frame, and when it has one it bounds its parts).

use std::collections::{BTreeMap, BTreeSet};

use super::cite::revision_of;
use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestError, ChangeRequestState, Crux,
};
use crate::domain::entity::project::Project;
use crate::domain::vo::answer::{Answer, Basis};
use crate::domain::vo::footprint::Move;
use crate::domain::vo::id::ChangeRequestId;
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

/// The expectations `wanted` as a change request records them: each target with what is expected of it and the
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

/// Gives the Crux of `cr` its answer, once, with each target the option chosen expects something of at the
/// revision it has in `project` now. An answer on the basis of a Policy line must cite a line the Policy of
/// `project` has, at the revision it has now (the line applied is the line as it reads at the decision). A Crux
/// that is answered already takes no other answer.
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

/// Where the targets of one change request stand against a Project.
#[derive(Debug, Default)]
struct Standing {
    /// To change, and still at the recorded revision.
    unchanged: Vec<Target>,
    /// To change, and gone.
    gone: Vec<Target>,
    /// To remove, and still there.
    not_removed: Vec<Target>,
    /// To keep, and not at the recorded revision (or gone).
    moved: Vec<Target>,
    /// To change or remove, and the target did move, or go, but not in the work of the request.
    elsewhere: Vec<Target>,
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

/// The change requests `cr` is a part of, nearest first. A cycle of parents is followed once.
fn ancestors<'a>(cr: &ChangeRequest, all: &'a [ChangeRequest]) -> Vec<&'a ChangeRequest> {
    let mut out: Vec<&ChangeRequest> = Vec::new();
    let mut seen = BTreeSet::from([cr.id()]);
    let mut parent = cr.parent();
    while let Some(id) = parent {
        let Some(found) = all.iter().find(|c| c.id() == id) else {
            break;
        };
        if !seen.insert(id) {
            break;
        }
        out.push(found);
        parent = found.parent();
    }
    out
}

/// The targets that moved in the work of `cr` or of a part of it.
fn moved_under(cr: &ChangeRequest, all: &[ChangeRequest]) -> BTreeSet<Target> {
    lineage(cr, all)
        .into_iter()
        .flat_map(|c| c.work())
        .flat_map(|f| f.targets())
        .collect()
}

/// Reads every expectation of `cr` (its own and those its decision asked) against `project`, and against the work
/// of `cr` and of the change requests in `all` that are a part of it. A target in the lists once.
fn standing(cr: &ChangeRequest, project: &Project, all: &[ChangeRequest]) -> Standing {
    let moved_here = moved_under(cr, all);
    let mut s = Standing::default();
    for (target, expectation) in cr.expectations() {
        let now = revision_of(project, target);
        let owed = moved_here.contains(&target);
        if expectation.is_met_by(now) && (owed || expectation.kind == ExpectKind::Keep) {
            continue;
        }
        let (list, elsewhere) = match (expectation.kind, now) {
            (ExpectKind::Change, Some(now)) => (&mut s.unchanged, now > expectation.recorded),
            (ExpectKind::Change, None) => (&mut s.gone, false),
            (ExpectKind::Remove, now) => (&mut s.not_removed, now.is_none()),
            (ExpectKind::Keep, _) => (&mut s.moved, false),
        };
        if !list.contains(&target) {
            list.push(target);
        }
        if elsewhere && !s.elsewhere.contains(&target) {
            s.elsewhere.push(target);
        }
    }
    s
}

/// The change requests, in `crs`, that are a part of `parent` (their parent is it) and are not resolved.
pub fn open_parts(parent: ChangeRequestId, crs: &[ChangeRequest]) -> Vec<ChangeRequestId> {
    crs.iter()
        .filter(|c| c.parent() == Some(parent) && c.state() != ChangeRequestState::Resolved)
        .map(|c| c.id())
        .collect()
}

/// Closes the record of `cr` once the change is applied: its Crux, if it has one, is decided; every expectation to
/// change or remove, its own and those of the decision, is met by the work of `cr` or of a part of it; no
/// expectation to keep has been broken; and every change request in `all` that is a part of it is resolved. A
/// change request is a record of its own, so `all` is every change request there is (`cr` itself may be among
/// them).
pub fn resolve(
    cr: &mut ChangeRequest,
    project: &Project,
    all: &[ChangeRequest],
) -> Result<(), ChangeRequestError> {
    if !cr.awaits_decision() {
        let s = standing(cr, project, all);
        if !s.unchanged.is_empty() || !s.gone.is_empty() || !s.not_removed.is_empty() {
            return Err(ChangeRequestError::NotApplied {
                unchanged: s.unchanged,
                gone: s.gone,
                not_removed: s.not_removed,
                elsewhere: s.elsewhere,
            });
        }
        if !s.moved.is_empty() {
            return Err(ChangeRequestError::KeepMoved { moved: s.moved });
        }
        let children = open_parts(cr.id(), all);
        if !children.is_empty() {
            return Err(ChangeRequestError::ChildrenNotResolved { children });
        }
    }
    cr.resolve()
}

/// An unresolved change request the targets have not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapplied {
    /// The change request.
    pub cr: ChangeRequestId,
    /// The targets to change that are still at the revision it recorded, in order.
    pub unchanged: Vec<Target>,
    /// The targets to change that the Project does not have any more, in order.
    pub gone: Vec<Target>,
    /// The targets to remove that are still in the Project, in order.
    pub not_removed: Vec<Target>,
    /// The targets among `unchanged` and `not_removed` that moved, or went, but not in the work of the request or of
    /// a part of it, in order.
    pub elsewhere: Vec<Target>,
}

/// Every unresolved change request in `crs` with an expectation to change or remove, its own or one its decision
/// asked, that is not met: the target is as it was, or is gone, or is still there, or moved but not in the work of
/// the request or of a part of it (`crs` is every change request there is, for the parts). One whose Crux is not
/// decided yet is here for its own expectations, and also in [`awaiting_decision`]. An expectation to keep that
/// was broken is not here: see [`keep_moved`].
pub fn unapplied(crs: &[ChangeRequest], project: &Project) -> Vec<Unapplied> {
    crs.iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved)
        .filter_map(|cr| {
            let s = standing(cr, project, crs);
            (!s.unchanged.is_empty() || !s.gone.is_empty() || !s.not_removed.is_empty()).then(
                || Unapplied {
                    cr: cr.id(),
                    unchanged: s.unchanged,
                    gone: s.gone,
                    not_removed: s.not_removed,
                    elsewhere: s.elsewhere,
                },
            )
        })
        .collect()
}

/// An unresolved change request one of whose expectations to keep was broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepMoved {
    /// The change request.
    pub cr: ChangeRequestId,
    /// The targets it expected to keep that are not at the revision it recorded (or are gone), in order.
    pub moved: Vec<Target>,
}

/// Every unresolved change request in `crs` that expects a target to stay as it is and the target has moved on
/// in `project`, whoever moved it: a keep is judged by the revision alone. A resolved one is a record: what changes
/// after it is another change request's business.
pub fn keep_moved(crs: &[ChangeRequest], project: &Project) -> Vec<KeepMoved> {
    crs.iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved)
        .filter_map(|cr| {
            let moved = standing(cr, project, crs).moved;
            (!moved.is_empty()).then(|| KeepMoved { cr: cr.id(), moved })
        })
        .collect()
}

/// The targets `cr` expects to change or remove, its own and those its decision asked: its frame. A container in it
/// frames everything inside.
pub fn frame(cr: &ChangeRequest) -> BTreeSet<Target> {
    cr.expectations()
        .filter(|(_, e)| e.kind != ExpectKind::Keep)
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

/// Every unresolved change request in `crs` with work that moved a target outside its frame, or outside the frame
/// of a request it is a part of (one that has no frame does not bound its parts: it only aggregates). A target is
/// inside a frame when it is, or is inside, a target the frame names. A container that moved only because something
/// inside it moved within the frame (a Contract that took a new Clause, the Context that holds a new asset) is not
/// outside. The work of a part is judged for the part, not again for its parent.
pub fn out_of_frame(crs: &[ChangeRequest]) -> Vec<OutOfFrame> {
    crs.iter()
        .filter(|c| c.state() != ChangeRequestState::Resolved && !c.work().is_empty())
        .filter_map(|cr| {
            let mut frames = vec![frame(cr)];
            frames.extend(ancestors(cr, crs).into_iter().map(frame));
            frames.retain(|f| !f.is_empty());
            let own_empty = frame(cr).is_empty();
            let inside = |m: &Move| {
                !own_empty
                    && frames
                        .iter()
                        .all(|f| f.iter().any(|container| m.is_inside(*container)))
            };
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
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::policy::PolicyLineId;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::signature::Signature;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::timestamp::Timestamp;
    use crate::domain::vo::title::Title;

    /// [`super::unapplied`] for the change requests given (they are every change request there is).
    fn unapplied<const N: usize>(crs: [&ChangeRequest; N], p: &Project) -> Vec<Unapplied> {
        let all: Vec<ChangeRequest> = crs.into_iter().cloned().collect();
        super::unapplied(&all, p)
    }

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

    fn harness() -> Target {
        Target::Harness(HarnessId::from_uuid(test_uuid(400)))
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
                Signature::try_from(sig).unwrap(),
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
        let mut d = draft(p, Intent::ChangeBehavior, &[(clause(), ExpectKind::Change)]);
        d.crux = crux;
        ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(1)), d)
    }

    /// A settled change (C1, its own expectation) and a question about a detail (which error type; C2 for a new
    /// one).
    fn person(p: &Project) -> ChangeRequest {
        let crux = Crux::new(
            CruxKind::Ambiguity,
            vec![
                CruxOption::new(choice("up"), None, BTreeMap::new(), None),
                CruxOption::new(
                    choice("to even"),
                    Some(Prose::try_from("C2 names a new error").unwrap()),
                    BTreeMap::from([(detail(), ExpectKind::Change)]),
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

    #[test]
    fn the_choice_of_the_recommended_option_still_requires_the_own_expectations() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("up"), &p).unwrap();
        let unchanged = || Unapplied {
            cr: c.id(),
            unchanged: vec![clause()],
            gone: Vec::new(),
            not_removed: Vec::new(),
            elsewhere: Vec::new(),
        };
        assert_eq!(unapplied([&c], &p), [unchanged()]);
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::NotApplied {
                unchanged: vec![clause()],
                gone: Vec::new(),
                not_removed: Vec::new(),
                elsewhere: Vec::new()
            })
        );
        work(&mut p, &mut c, 2, |p| {
            put_both(
                p,
                "a discount above 50% is refused",
                "a refusal is InvalidDiscount",
            );
        });
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    #[test]
    fn what_the_chosen_option_expects_is_required_besides_the_own_expectations() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                cr: c.id(),
                unchanged: vec![clause(), detail()],
                gone: Vec::new(),
                not_removed: Vec::new(),
                elsewhere: Vec::new()
            }]
        );
        // The own expectation alone is not enough.
        work(&mut p, &mut c, 2, |p| {
            put_both(
                p,
                "a discount above 50% is refused",
                "a refusal is InvalidDiscount",
            );
        });
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::NotApplied {
                unchanged: vec![detail()],
                gone: Vec::new(),
                not_removed: Vec::new(),
                elsewhere: Vec::new()
            })
        );
        work(&mut p, &mut c, 3, |p| {
            put_both(
                p,
                "a discount above 50% is refused",
                "a refusal is InvalidRate",
            );
        });
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_second_decision_is_refused_and_what_the_first_asked_still_holds() {
        let mut p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        assert_eq!(
            decide(&mut c, chosen("up"), &p),
            Err(ChangeRequestError::AlreadyAnswered)
        );
        // The detail the first decision asked is still behind, besides the own expectation.
        assert_eq!(unapplied([&c], &p)[0].unchanged, [clause(), detail()]);
        work(&mut p, &mut c, 2, |p| {
            put_both(
                p,
                "a discount above 50% is refused",
                "a refusal is InvalidRate",
            );
        });
        resolve(&mut c, &p, &[]).unwrap();
    }

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
    fn a_change_request_with_no_crux_waits_for_its_expectations_to_be_met() {
        let mut p = project();
        let mut c = cr(&p, None);
        assert_eq!(unapplied([&c], &p).len(), 1);
        assert!(resolve(&mut c, &p, &[]).is_err());
        work(&mut p, &mut c, 2, |p| {
            put_contract(p, "a half rounds up, to whole cents");
        });
        resolve(&mut c, &p, &[]).unwrap();
        assert!(unapplied([&c], &p).is_empty());
    }

    #[test]
    fn an_undecided_crux_is_not_resolved_even_when_the_expectations_are_met_and_is_awaited() {
        let mut p = project();
        let mut c = person(&p);
        assert_eq!(awaiting_decision([&c]), [&c]);
        // Its own expectation is behind too: it is reported for both.
        assert_eq!(unapplied([&c], &p).len(), 1);
        work(&mut p, &mut c, 2, |p| {
            put_both(
                p,
                "a discount above 50% is refused",
                "a refusal is InvalidDiscount",
            );
        });
        assert!(unapplied([&c], &p).is_empty());
        assert_eq!(awaiting_decision([&c]), [&c]);
        assert_eq!(resolve(&mut c, &p, &[]), Err(ChangeRequestError::Undecided));
        decide(&mut c, chosen("up"), &p).unwrap();
        assert!(awaiting_decision([&c]).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        assert!(awaiting_decision([&c]).is_empty());
    }

    #[test]
    fn a_target_to_change_that_is_gone_is_reported() {
        let p = project();
        let mut c = person(&p);
        decide(&mut c, chosen("to even"), &p).unwrap();
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(
            unapplied([&c], &empty),
            [Unapplied {
                cr: c.id(),
                unchanged: Vec::new(),
                gone: vec![clause(), detail()],
                not_removed: Vec::new(),
                elsewhere: Vec::new()
            }]
        );
    }

    // M1: a change request that keeps the behaviour resolves, without a warning.

    #[test]
    fn a_keep_behavior_request_keeps_a_clause_and_changes_an_item_and_resolves() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(clause(), ExpectKind::Keep), (item(), ExpectKind::Change)],
        );
        assert_eq!(unapplied([&c], &p).len(), 1);
        assert!(keep_moved([&c], &p).is_empty());
        work(&mut p, &mut c, 2, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)")
        });
        assert!(unapplied([&c], &p).is_empty());
        assert!(keep_moved([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        assert_eq!(c.state(), ChangeRequestState::Resolved);
    }

    // M2: adding is a change of what holds the new thing.

    #[test]
    fn a_clause_is_added_as_a_change_of_its_contract() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(contract(), ExpectKind::Change)],
        );
        assert_eq!(unapplied([&c], &p).len(), 1);
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
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn an_interface_is_added_as_a_change_of_its_context() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(Target::Context(core()), ExpectKind::Change)],
        );
        // A change inside an asset does not move the Context.
        work(&mut p, &mut c, 2, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)");
            put_contract(p, "another promise");
        });
        assert_eq!(unapplied([&c], &p).len(), 1);
        // A new asset does.
        work(&mut p, &mut c, 3, |p| put_interface(p, 101, 11, "fn b()"));
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_context_is_added_as_a_change_of_the_project() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(Target::Project, ExpectKind::Change)],
        );
        // A change inside a Context does not move the Project.
        work(&mut p, &mut c, 2, |p| put_interface(p, 101, 11, "fn b()"));
        assert_eq!(unapplied([&c], &p).len(), 1);
        work(&mut p, &mut c, 3, |p| {
            p.add_context(context(billing(), "billing")).unwrap();
        });
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_new_use_or_basis_moves_the_context() {
        let mut p = project();
        p.add_context(context(billing(), "billing")).unwrap();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(Target::Context(core()), ExpectKind::Change)],
        );
        work(&mut p, &mut c, 2, |p| {
            p.set_uses(core(), BTreeSet::from([billing()])).unwrap();
        });
        assert!(unapplied([&c], &p).is_empty());
        let mut c = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[(Target::Context(core()), ExpectKind::Change)],
        );
        // The same uses again move nothing.
        work(&mut p, &mut c, 3, |p| {
            p.set_uses(core(), BTreeSet::from([billing()])).unwrap();
        });
        assert_eq!(unapplied([&c], &p).len(), 1);
        work(&mut p, &mut c, 4, |p| p.base_on_contract(core()).unwrap());
        assert!(unapplied([&c], &p).is_empty());
    }

    // M3: removing resolves once the target is gone.

    #[test]
    fn a_clause_is_removed_when_it_is_no_longer_in_its_contract() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(detail(), ExpectKind::Remove)],
        );
        assert_eq!(
            unapplied([&c], &p),
            [Unapplied {
                cr: c.id(),
                unchanged: Vec::new(),
                gone: Vec::new(),
                not_removed: vec![detail()],
                elsewhere: Vec::new()
            }]
        );
        // Changing it is not removing it.
        work(&mut p, &mut c, 2, |p| {
            put_both(p, "a half rounds up", "a refusal is InvalidRate");
        });
        assert!(resolve(&mut c, &p, &[]).is_err());
        work(&mut p, &mut c, 3, |p| put_first_only(p, "a half rounds up"));
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_context_is_removed_when_the_project_no_longer_has_it() {
        let mut with_two = project();
        with_two.add_context(context(billing(), "billing")).unwrap();
        let mut c = open(
            &with_two,
            1,
            Intent::ChangeBehavior,
            &[(Target::Context(billing()), ExpectKind::Remove)],
        );
        assert_eq!(unapplied([&c], &with_two).len(), 1);
        work(&mut with_two, &mut c, 2, |p| {
            p.retire_context(billing()).unwrap();
        });
        assert!(unapplied([&c], &with_two).is_empty());
        resolve(&mut c, &with_two, &[]).unwrap();
    }

    // M4: a Case can be changed.

    #[test]
    fn a_case_is_changed_when_its_code_changes() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(case(), ExpectKind::Change)],
        );
        assert_eq!(unapplied([&c], &p).len(), 1);
        work(&mut p, &mut c, 2, |p| {
            put_harness(p, "fn t() { assert!(true); }");
        });
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
        // The container and the leaf are told apart.
        let mut h = open(
            &p,
            2,
            Intent::ChangeBehavior,
            &[
                (harness(), ExpectKind::Change),
                (interface(), ExpectKind::Keep),
            ],
        );
        assert_eq!(unapplied([&h], &p).len(), 1);
        work(&mut p, &mut h, 3, |p| {
            put_harness(p, "fn t() { assert!(false); }");
        });
        assert!(unapplied([&h], &p).is_empty());
    }

    // Keep.

    #[test]
    fn a_keep_that_moved_refuses_the_resolve_and_is_reported_apart() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::KeepBehavior,
            &[(clause(), ExpectKind::Keep), (item(), ExpectKind::Change)],
        );
        work(&mut p, &mut c, 2, |p| {
            put_interface(p, 100, 10, "fn a(x: u8)")
        });
        // Someone else moves the clause, in a change set for no request.
        put_contract(&mut p, "a half rounds to even");
        assert!(unapplied([&c], &p).is_empty());
        assert_eq!(
            keep_moved([&c], &p),
            [KeepMoved {
                cr: c.id(),
                moved: vec![clause()]
            }]
        );
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::KeepMoved {
                moved: vec![clause()]
            })
        );
        assert_eq!(c.state(), ChangeRequestState::Open);
        // Gone is moved too.
        let empty = Project::new(ProjectId::from_uuid(test_uuid(1001)));
        assert_eq!(keep_moved([&c], &empty)[0].moved, [clause()]);
    }

    #[test]
    fn a_resolved_request_is_a_record_and_reports_nothing() {
        let mut p = project();
        let mut c = open(&p, 1, Intent::KeepBehavior, &[(clause(), ExpectKind::Keep)]);
        resolve(&mut c, &p, &[]).unwrap();
        put_contract(&mut p, "a half rounds to even");
        assert!(keep_moved([&c], &p).is_empty());
        assert!(unapplied([&c], &p).is_empty());
    }

    // Options.

    #[test]
    fn an_option_that_removes_a_target_requires_it_gone() {
        let mut p = project();
        let crux = Crux::new(
            CruxKind::Conflict,
            vec![
                CruxOption::new(
                    choice("drop C2"),
                    None,
                    BTreeMap::from([(detail(), ExpectKind::Remove)]),
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
        decide(&mut c, chosen("drop C2"), &p).unwrap();
        work(&mut p, &mut c, 2, |p| {
            put_contract(p, "a half rounds to even")
        });
        assert_eq!(
            resolve(&mut c, &p, &[]),
            Err(ChangeRequestError::NotApplied {
                unchanged: Vec::new(),
                gone: Vec::new(),
                not_removed: vec![detail()],
                elsewhere: Vec::new()
            })
        );
        work(&mut p, &mut c, 3, |p| {
            put_first_only(p, "a half rounds to even");
        });
        resolve(&mut c, &p, &[]).unwrap();
        // Choosing the other option instead asks to keep it.
        let fresh = project();
        let mut other = cr(&fresh, c.crux().cloned());
        decide(&mut other, chosen("keep both"), &fresh).unwrap();
        assert_eq!(other.decided_expects()[&detail()].kind, ExpectKind::Keep);
    }

    // M6: a parent waits for its parts.

    #[test]
    fn a_parent_is_not_resolved_while_a_part_is_not() {
        let p = project();
        let mut parent = open(&p, 1, Intent::ChangeBehavior, &[]);
        let mut d = draft(&p, Intent::ChangeBehavior, &[]);
        d.title = Title::try_from("part").unwrap();
        d.parent = Some(parent.id());
        let mut part = ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(2)), d);
        let all = [parent.clone(), part.clone()];
        assert_eq!(open_parts(parent.id(), &all), [part.id()]);
        assert_eq!(
            resolve(&mut parent, &p, &all),
            Err(ChangeRequestError::ChildrenNotResolved {
                children: vec![part.id()]
            })
        );
        assert_eq!(parent.state(), ChangeRequestState::Open);
        resolve(&mut part, &p, &all).unwrap();
        let all = [parent.clone(), part];
        assert!(open_parts(parent.id(), &all).is_empty());
        resolve(&mut parent, &p, &all).unwrap();
        assert_eq!(parent.state(), ChangeRequestState::Resolved);
    }

    // Attribution: a change or a removal is owed to the request whose work moved it.

    /// A request with this title and id, expecting `wanted` of `p`'s targets, a part of `parent` if it has one.
    fn titled(
        p: &Project,
        n: u128,
        title: &str,
        parent: Option<&ChangeRequest>,
        wanted: &[(Target, ExpectKind)],
    ) -> ChangeRequest {
        let mut d = draft(p, Intent::ChangeBehavior, wanted);
        d.title = Title::try_from(title).unwrap();
        d.parent = parent.map(ChangeRequest::id);
        ChangeRequest::open(ChangeRequestId::from_uuid(test_uuid(n)), d)
    }

    #[test]
    fn a_change_is_met_by_the_work_of_its_own_request() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Change)],
        );
        work(&mut p, &mut c, 2, |p| {
            put_contract(p, "a half rounds to even")
        });
        assert!(unapplied([&c], &p).is_empty());
        resolve(&mut c, &p, &[]).unwrap();
    }

    #[test]
    fn a_change_made_for_another_request_or_for_none_does_not_meet_it() {
        let mut p = project();
        let mut mine = titled(&p, 1, "mine", None, &[(clause(), ExpectKind::Change)]);
        let mut other = titled(&p, 2, "other", None, &[(clause(), ExpectKind::Change)]);
        // For `other`: the clause moved, and it is told apart from the targets that did not.
        work(&mut p, &mut other, 2, |p| {
            put_contract(p, "a half rounds to even")
        });
        let all = [mine.clone(), other.clone()];
        assert_eq!(
            super::unapplied(&all, &p),
            [Unapplied {
                cr: mine.id(),
                unchanged: vec![clause()],
                gone: Vec::new(),
                not_removed: Vec::new(),
                elsewhere: vec![clause()]
            }]
        );
        assert_eq!(
            resolve(&mut mine, &p, &all),
            Err(ChangeRequestError::NotApplied {
                unchanged: vec![clause()],
                gone: Vec::new(),
                not_removed: Vec::new(),
                elsewhere: vec![clause()]
            })
        );
        // In a change set for no request: nobody's work, so it meets nothing.
        put_contract(&mut p, "a half rounds to odd");
        assert_eq!(unapplied([&mine], &p)[0].elsewhere, [clause()]);
        // Its own work meets it.
        work(&mut p, &mut mine, 4, |p| {
            put_contract(p, "a half rounds up")
        });
        assert!(super::unapplied(&[mine.clone(), other], &p).is_empty());
        resolve(&mut mine, &p, &[]).unwrap();
    }

    #[test]
    fn a_removal_is_owed_to_the_request_that_took_the_target_out() {
        let mut p = project();
        let mut mine = titled(&p, 1, "mine", None, &[(detail(), ExpectKind::Remove)]);
        let mut other = titled(&p, 2, "other", None, &[(case(), ExpectKind::Change)]);
        work(&mut p, &mut other, 2, |p| {
            put_first_only(p, "a half rounds up")
        });
        let found = unapplied([&mine], &p);
        assert_eq!(found[0].not_removed, [detail()]);
        assert_eq!(found[0].elsewhere, [detail()]);
        // The same set, for `mine`, would have met it.
        let mut q = project();
        work(&mut q, &mut mine, 2, |p| {
            put_first_only(p, "a half rounds up")
        });
        assert!(unapplied([&mine], &q).is_empty());
    }

    #[test]
    fn a_request_is_met_by_the_work_of_a_part_of_it_and_not_by_a_sibling_of_its_parent() {
        let mut p = project();
        let parent = titled(&p, 1, "parent", None, &[(contract(), ExpectKind::Change)]);
        let mut part = titled(
            &p,
            2,
            "part",
            Some(&parent),
            &[(clause(), ExpectKind::Change)],
        );
        let mut stranger = titled(&p, 3, "stranger", None, &[(detail(), ExpectKind::Change)]);
        work(&mut p, &mut stranger, 2, |p| {
            put_both(p, "a half rounds up", "a refusal is InvalidRate");
        });
        let all = [parent.clone(), part.clone(), stranger.clone()];
        // The contract moved, but for the stranger: the parent is not met.
        assert_eq!(super::unapplied(&all, &p)[0].cr, parent.id());
        work(&mut p, &mut part, 3, |p| {
            put_contract(p, "a half rounds to even")
        });
        let all = [parent.clone(), part.clone(), stranger];
        let behind: Vec<ChangeRequestId> =
            super::unapplied(&all, &p).iter().map(|u| u.cr).collect();
        assert!(!behind.contains(&parent.id()));
        assert!(!behind.contains(&part.id()));
        let mut parent = parent;
        resolve(&mut part, &p, &all).unwrap();
        let all = [parent.clone(), part, all[2].clone()];
        resolve(&mut parent, &p, &all).unwrap();
    }

    #[test]
    fn a_keep_is_judged_by_the_revision_whoever_moved_it() {
        let mut p = project();
        let mut mine = titled(&p, 1, "mine", None, &[(clause(), ExpectKind::Keep)]);
        let mut other = titled(&p, 2, "other", None, &[(clause(), ExpectKind::Change)]);
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
        assert_eq!(
            resolve(&mut mine, &p, &all),
            Err(ChangeRequestError::KeepMoved {
                moved: vec![clause()]
            })
        );
    }

    #[test]
    fn a_resolved_request_takes_no_more_work() {
        let p = project();
        let mut c = open(&p, 1, Intent::KeepBehavior, &[]);
        resolve(&mut c, &p, &[]).unwrap();
        assert_eq!(
            c.record_work(Footprint::new(2, Vec::new())),
            Err(ChangeRequestError::AlreadyResolved)
        );
    }

    // Frame: the work stays inside what the request said it changes or removes.

    #[test]
    fn work_on_a_target_the_request_did_not_name_is_out_of_frame() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Change)],
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
            &[(contract(), ExpectKind::Change)],
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
            &[(Target::Context(core()), ExpectKind::Change)],
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
    fn the_frame_of_a_parent_bounds_its_parts_and_a_parent_with_none_does_not() {
        let mut p = project();
        let bounding = titled(&p, 1, "bounding", None, &[(contract(), ExpectKind::Change)]);
        let aggregating = titled(&p, 2, "aggregating", None, &[]);
        let mut under_bounding = titled(
            &p,
            3,
            "under bounding",
            Some(&bounding),
            &[
                (clause(), ExpectKind::Change),
                (interface(), ExpectKind::Change),
            ],
        );
        let mut under_aggregating = titled(
            &p,
            4,
            "under aggregating",
            Some(&aggregating),
            &[(interface(), ExpectKind::Change)],
        );
        work(&mut p, &mut under_bounding, 2, |p| {
            put_contract(p, "a half rounds to even");
            put_interface(p, 100, 10, "fn a(x: u8)");
        });
        work(&mut p, &mut under_aggregating, 3, |p| {
            put_interface(p, 100, 10, "fn a(x: u16)");
        });
        let all = [
            bounding,
            aggregating,
            under_bounding.clone(),
            under_aggregating,
        ];
        // The part named the interface, but its parent's frame is the contract: the interface is outside.
        assert_eq!(
            out_of_frame(&all),
            [OutOfFrame {
                cr: under_bounding.id(),
                moved: vec![interface(), item()]
            }]
        );
    }

    #[test]
    fn a_resolved_request_is_not_reported_for_its_work() {
        let mut p = project();
        let mut c = open(
            &p,
            1,
            Intent::ChangeBehavior,
            &[(clause(), ExpectKind::Change)],
        );
        work(&mut p, &mut c, 2, |p| {
            put_contract(p, "a half rounds to even");
            put_interface(p, 100, 10, "fn a(x: u8)");
        });
        assert_eq!(out_of_frame(std::slice::from_ref(&c)).len(), 1);
        resolve(&mut c, &p, &[]).unwrap();
        assert!(out_of_frame(std::slice::from_ref(&c)).is_empty());
    }
}
