//! Pinning the code of an Interface, and telling whether a Pin still holds.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::entity::context::Context;
use crate::domain::entity::interface::Interface;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{ContractId, HarnessId, InterfaceId, PinId};
use crate::domain::vo::pin_basis::PinBasis;

/// Why the code of an Interface cannot be pinned.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PinError {
    /// No Context of the Project has the Interface.
    #[error("the interface {0} is not in the project")]
    UnknownInterface(InterfaceId),
    /// No Case cites an Item of the Interface, so nothing checked the code.
    #[error("no case cites an item of the interface {0}")]
    Unchecked(InterfaceId),
}

/// Why a Pin no longer holds: what moved past the revision it recorded, or is gone, or is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleReason {
    /// The pinned Interface changed.
    Interface,
    /// This Harness changed, is gone, or newly checks the Interface.
    Harness(HarnessId),
    /// This Contract changed, is gone, or is newly cited.
    Contract(ContractId),
    /// This other Interface changed, is gone, or is newly cited.
    UsedInterface(InterfaceId),
}

/// Whether a Pin still holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staleness {
    /// Nothing it stood on has changed.
    Fresh,
    /// The pinned Interface is not in the Project any more.
    InterfaceGone,
    /// These moved, in the order of [`StaleReason`] and then of their ids.
    Stale(Vec<StaleReason>),
}

/// Pins the code of `interface` with the digest `code`, recording what it stands on in `project` now. Some Case
/// must cite one of its Items: code nothing checked is not accepted.
pub fn record(
    project: &Project,
    interface: InterfaceId,
    id: PinId,
    code: Digest,
) -> Result<Pin, PinError> {
    let (context, pinned) =
        find(project, interface).ok_or(PinError::UnknownInterface(interface))?;
    let basis = basis(project, context, pinned);
    if basis.harnesses().is_empty() {
        return Err(PinError::Unchecked(interface));
    }
    Ok(Pin::new(id, interface, code, basis))
}

/// Whether `pin` still holds in `project`: every revision it recorded is still the one there, and the citations
/// reach the same assets. An Interface no Case checks any more has lost every Harness it stood on.
pub fn staleness(pin: &Pin, project: &Project) -> Staleness {
    let Some((context, pinned)) = find(project, pin.interface()) else {
        return Staleness::InterfaceGone;
    };
    let now = basis(project, context, pinned);
    let was = pin.basis();
    let mut why = Vec::new();
    if now.interface() != was.interface() {
        why.push(StaleReason::Interface);
    }
    why.extend(moved(was.harnesses(), now.harnesses()).map(StaleReason::Harness));
    why.extend(moved(was.contracts(), now.contracts()).map(StaleReason::Contract));
    why.extend(moved(was.used_interfaces(), now.used_interfaces()).map(StaleReason::UsedInterface));
    if why.is_empty() {
        Staleness::Fresh
    } else {
        Staleness::Stale(why)
    }
}

/// What the code of `interface` stands on in `project` now: the revisions a Pin made at this moment would record.
/// A measurement of the code records it, to tell later that what it measured has moved. `None` when the Interface
/// is not in the Project.
pub fn current_basis(project: &Project, interface: InterfaceId) -> Option<PinBasis> {
    find(project, interface).map(|(context, pinned)| basis(project, context, pinned))
}

/// The keys whose revision differs between `was` and `now`, or that are in only one of them, in order.
fn moved<'a, K: Ord + Copy, V: PartialEq>(
    was: &'a BTreeMap<K, V>,
    now: &'a BTreeMap<K, V>,
) -> impl Iterator<Item = K> + 'a {
    let mut keys: Vec<K> = was.keys().chain(now.keys()).copied().collect();
    keys.sort();
    keys.dedup();
    keys.into_iter().filter(move |k| was.get(k) != now.get(k))
}

/// The Context that has `interface`, and the Interface.
fn find(project: &Project, interface: InterfaceId) -> Option<(&Context, &Interface)> {
    project
        .contexts()
        .find_map(|c| c.interface(interface).map(|i| (c, i)))
}

/// What the code of `pinned` (of `context`) stands on in `project` now: gathered by following the citations of
/// every Case of the Context that cites one of its Items.
fn basis(project: &Project, context: &Context, pinned: &Interface) -> PinBasis {
    let interface = pinned.id();
    let reach: Vec<&Context> = std::iter::once(context)
        .chain(context.uses().iter().filter_map(|u| project.context(*u)))
        .collect();
    let mut harnesses = BTreeMap::new();
    let mut contracts = BTreeMap::new();
    let mut used_interfaces = BTreeMap::new();
    for harness in context.harnesses() {
        for case in harness.cases() {
            let cites = case.cites();
            if !cites.items().keys().any(|i| pinned.item(*i).is_some()) {
                continue;
            }
            harnesses.insert(harness.id(), harness.revision());
            for item in cites.items().keys() {
                if let Some(other) = reach.iter().find_map(|c| c.interface_of(*item))
                    && other.id() != interface
                {
                    used_interfaces.insert(other.id(), other.revision());
                }
            }
            for clause in cites.clauses().keys() {
                if let Some(contract) = reach.iter().find_map(|c| c.contract_of(*clause)) {
                    contracts.insert(contract.id(), contract.revision());
                }
            }
        }
    }
    PinBasis::new(pinned.revision(), harnesses, contracts, used_interfaces)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::id::{CaseId, ClauseId, ContextId, ItemId, ProjectId, test_uuid};
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::title::Title;

    fn ctx(n: u128) -> ContextId {
        ContextId::from_uuid(test_uuid(n))
    }

    fn iface(n: u128) -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(n))
    }

    fn put_interface(p: &mut Project, context: u128, id: u128, item: u128, sig: &str) {
        p.put_interface(
            ctx(context),
            iface(id),
            Source::new(format!("{sig}\n")),
            vec![(
                ItemId::from_uuid(test_uuid(item)),
                ItemKey::try_from(sig.split('(').next().unwrap()).unwrap(),
                LineRange::new(1, 1).unwrap(),
            )],
        )
        .unwrap();
    }

    fn put_contract(p: &mut Project, text: &str) {
        p.put_contract(
            ctx(1),
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

    /// Puts on `core` the Harness 400 with one Case citing `items` (and Clause 30 when `clause`).
    fn put_harness(p: &mut Project, items: &[u128], clause: bool, body: &str) {
        let cites = CaseCites::new(
            items
                .iter()
                .map(|i| (ItemId::from_uuid(test_uuid(*i)), Revision::first()))
                .collect(),
            if clause {
                BTreeMap::from([(ClauseId::from_uuid(test_uuid(30)), Revision::first())])
            } else {
                BTreeMap::new()
            },
        )
        .unwrap();
        p.put_harness(
            ctx(1),
            HarnessId::from_uuid(test_uuid(400)),
            Source::new(format!("{body}\n")),
            vec![(
                CaseId::from_uuid(test_uuid(40)),
                CaseName::try_from("t").unwrap(),
                LineRange::new(1, 1).unwrap(),
                cites,
            )],
        )
        .unwrap();
    }

    /// `core` (1) uses `base` (2). `core` has the Interfaces 100 (Item 10) and 101 (Item 11), the Contract 300
    /// (Clause 30) and the Harness 400 whose Case cites Item 10, Item 20 of `base`'s Interface 200, and Clause 30.
    fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(test_uuid(1000)));
        p.add_context(
            Context::new(
                ctx(2),
                ContextName::try_from("base").unwrap(),
                BTreeSet::new(),
            )
            .unwrap(),
        )
        .unwrap();
        p.add_context(
            Context::new(
                ctx(1),
                ContextName::try_from("core").unwrap(),
                BTreeSet::from([ctx(2)]),
            )
            .unwrap(),
        )
        .unwrap();
        put_interface(&mut p, 1, 100, 10, "fn a()");
        put_interface(&mut p, 1, 101, 11, "fn b()");
        put_interface(&mut p, 2, 200, 20, "fn base()");
        put_contract(&mut p, "a promise");
        put_harness(&mut p, &[10, 20], true, "fn t() {}");
        p
    }

    fn pin(p: &Project) -> Pin {
        record(
            p,
            iface(100),
            PinId::from_uuid(test_uuid(900)),
            Digest::of(b"code"),
        )
        .unwrap()
    }

    #[test]
    fn a_pin_records_what_the_citations_reach() {
        let p = project();
        let pin = pin(&p);
        assert_eq!(pin.code(), Digest::of(b"code"));
        let b = pin.basis();
        assert_eq!(b.interface(), Revision::first());
        assert_eq!(
            b.harnesses().keys().collect::<Vec<_>>(),
            [&HarnessId::from_uuid(test_uuid(400))]
        );
        assert_eq!(
            b.contracts().keys().collect::<Vec<_>>(),
            [&ContractId::from_uuid(test_uuid(300))]
        );
        assert_eq!(
            b.used_interfaces().keys().collect::<Vec<_>>(),
            [&iface(200)]
        );
        assert_eq!(staleness(&pin, &p), Staleness::Fresh);
    }

    #[test]
    fn an_interface_no_case_checks_or_not_in_the_project_cannot_be_pinned() {
        let p = project();
        let id = PinId::from_uuid(test_uuid(900));
        assert_eq!(
            record(&p, iface(101), id, Digest::of(b"")),
            Err(PinError::Unchecked(iface(101)))
        );
        assert_eq!(
            record(&p, iface(999), id, Digest::of(b"")),
            Err(PinError::UnknownInterface(iface(999)))
        );
    }

    #[test]
    fn a_change_to_what_it_stood_on_stales_the_pin() {
        let mut p = project();
        let pin = pin(&p);
        put_interface(&mut p, 1, 100, 10, "fn a(x: u8)");
        put_contract(&mut p, "another promise");
        put_interface(&mut p, 2, 200, 20, "fn base(x: u8)");
        assert_eq!(
            staleness(&pin, &p),
            Staleness::Stale(vec![
                StaleReason::Interface,
                StaleReason::Contract(ContractId::from_uuid(test_uuid(300))),
                StaleReason::UsedInterface(iface(200)),
            ])
        );
    }

    #[test]
    fn a_change_the_citations_do_not_reach_stales_nothing() {
        let mut p = project();
        let pin = pin(&p);
        put_interface(&mut p, 1, 101, 11, "fn b(x: u8)");
        assert_eq!(staleness(&pin, &p), Staleness::Fresh);
    }

    #[test]
    fn a_changed_or_dropped_check_stales_the_pin() {
        let mut p = project();
        let pin = pin(&p);
        // the Case now cites only Item 10: the Harness changed, Contract 300 and Interface 200 are no longer cited
        put_harness(&mut p, &[10], false, "fn t() {}");
        assert_eq!(
            staleness(&pin, &p),
            Staleness::Stale(vec![
                StaleReason::Harness(HarnessId::from_uuid(test_uuid(400))),
                StaleReason::Contract(ContractId::from_uuid(test_uuid(300))),
                StaleReason::UsedInterface(iface(200)),
            ])
        );
        // no Case checks Interface 100 any more
        put_harness(&mut p, &[11], false, "fn t() {}");
        assert!(matches!(staleness(&pin, &p), Staleness::Stale(_)));
    }
}
