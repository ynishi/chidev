//! Schema 1 of the stored documents. These types are a versioned storage format, not a copy of the domain: they must
//! stay readable as the domain changes (a later schema is a new module beside this one), and they reach the domain
//! only through its `restore` functions, which re-check every rule.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entity::change_request::{
    ChangeRequest, ChangeRequestDraft, ChangeRequestState, Crux, Route,
};
use crate::domain::entity::context::{Basis, Context};
use crate::domain::entity::contract::Contract;
use crate::domain::entity::harness::Harness;
use crate::domain::entity::interface::Interface;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::case::{CaseCites, CaseName};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{
    CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
    PinId, ProjectId,
};
use crate::domain::vo::intent::{Intent, Issuer};
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::name::ContextName;
use crate::domain::vo::pin_basis::PinBasis;
use crate::domain::vo::prose::Prose;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::signature::Signature;
use crate::domain::vo::source::Source;
use crate::domain::vo::target::Target;
use crate::domain::vo::title::Title;

/// The schema number these documents carry.
pub(crate) const SCHEMA: u32 = 1;

/// Why a stored document cannot be read as the model.
pub(crate) type Bad = String;

fn bad(e: impl std::fmt::Display) -> Bad {
    e.to_string()
}

fn uuid(text: &str) -> Result<Uuid, Bad> {
    Uuid::parse_str(text).map_err(|e| format!("`{text}` is not an id: {e}"))
}

fn revision(number: u64) -> Result<Revision, Bad> {
    Revision::restore(number).ok_or_else(|| "a revision is 0".to_string())
}

fn lines(first: usize, last: usize) -> Result<LineRange, Bad> {
    LineRange::new(first, last).map_err(bad)
}

fn prose(text: &Option<String>) -> Result<Option<Prose>, Bad> {
    text.as_deref()
        .map(|t| Prose::try_from(t).map_err(bad))
        .transpose()
}

/// An id and the revision a citation or a basis recorded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Rev {
    id: String,
    revision: u64,
}

fn revs<I: Copy>(map: &BTreeMap<I, Revision>, uuid_of: impl Fn(I) -> Uuid) -> Vec<Rev> {
    map.iter()
        .map(|(id, r)| Rev {
            id: uuid_of(*id).to_string(),
            revision: r.number(),
        })
        .collect()
}

fn unrevs<I: Ord>(list: &[Rev], make: impl Fn(Uuid) -> I) -> Result<BTreeMap<I, Revision>, Bad> {
    list.iter()
        .map(|r| Ok((make(uuid(&r.id)?), revision(r.revision)?)))
        .collect()
}

// ---------------------------------------------------------------- Project

/// The Project with everything in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectDoc {
    pub schema: u32,
    id: String,
    contexts: Vec<ContextDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BasisDoc {
    Interface,
    Contract,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContextDoc {
    id: String,
    name: String,
    basis: BasisDoc,
    uses: Vec<String>,
    contracts: Vec<ContractDoc>,
    interfaces: Vec<InterfaceDoc>,
    harnesses: Vec<HarnessDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContractDoc {
    id: String,
    revision: u64,
    title: String,
    clauses: Vec<ClauseDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClauseDoc {
    id: String,
    revision: u64,
    label: String,
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InterfaceDoc {
    id: String,
    revision: u64,
    source: String,
    items: Vec<ItemDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ItemDoc {
    id: String,
    revision: u64,
    signature: String,
    first: usize,
    last: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessDoc {
    id: String,
    revision: u64,
    source: String,
    cases: Vec<CaseDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaseDoc {
    id: String,
    revision: u64,
    name: String,
    first: usize,
    last: usize,
    items: Vec<Rev>,
    clauses: Vec<Rev>,
}

impl From<&Project> for ProjectDoc {
    fn from(p: &Project) -> ProjectDoc {
        ProjectDoc {
            schema: SCHEMA,
            id: p.id().as_uuid().to_string(),
            contexts: p.contexts().map(ContextDoc::from).collect(),
        }
    }
}

impl From<&Context> for ContextDoc {
    fn from(c: &Context) -> ContextDoc {
        ContextDoc {
            id: c.id().as_uuid().to_string(),
            name: c.name().to_string(),
            basis: match c.basis() {
                Basis::Interface => BasisDoc::Interface,
                Basis::Contract => BasisDoc::Contract,
            },
            uses: c.uses().iter().map(|u| u.as_uuid().to_string()).collect(),
            contracts: c.contracts().map(ContractDoc::from).collect(),
            interfaces: c.interfaces().map(InterfaceDoc::from).collect(),
            harnesses: c.harnesses().map(HarnessDoc::from).collect(),
        }
    }
}

impl From<&Contract> for ContractDoc {
    fn from(k: &Contract) -> ContractDoc {
        ContractDoc {
            id: k.id().as_uuid().to_string(),
            revision: k.revision().number(),
            title: k.title().to_string(),
            clauses: k
                .clauses()
                .iter()
                .map(|c| ClauseDoc {
                    id: c.id().as_uuid().to_string(),
                    revision: c.revision().number(),
                    label: c.content().label().to_string(),
                    text: c.content().text().as_str().to_string(),
                })
                .collect(),
        }
    }
}

impl From<&Interface> for InterfaceDoc {
    fn from(i: &Interface) -> InterfaceDoc {
        InterfaceDoc {
            id: i.id().as_uuid().to_string(),
            revision: i.revision().number(),
            source: i.source().as_str().to_string(),
            items: i
                .items()
                .iter()
                .map(|it| ItemDoc {
                    id: it.id().as_uuid().to_string(),
                    revision: it.revision().number(),
                    signature: it.signature().to_string(),
                    first: it.lines().first(),
                    last: it.lines().last(),
                })
                .collect(),
        }
    }
}

impl From<&Harness> for HarnessDoc {
    fn from(h: &Harness) -> HarnessDoc {
        HarnessDoc {
            id: h.id().as_uuid().to_string(),
            revision: h.revision().number(),
            source: h.source().as_str().to_string(),
            cases: h
                .cases()
                .iter()
                .map(|c| CaseDoc {
                    id: c.id().as_uuid().to_string(),
                    revision: c.revision().number(),
                    name: c.name().to_string(),
                    first: c.lines().first(),
                    last: c.lines().last(),
                    items: revs(c.cites().items(), |i: ItemId| i.as_uuid()),
                    clauses: revs(c.cites().clauses(), |i: ClauseId| i.as_uuid()),
                })
                .collect(),
        }
    }
}

impl TryFrom<ProjectDoc> for Project {
    type Error = Bad;

    fn try_from(d: ProjectDoc) -> Result<Project, Bad> {
        let contexts = d
            .contexts
            .into_iter()
            .map(Context::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Project::restore(ProjectId::from_uuid(uuid(&d.id)?), contexts).map_err(bad)
    }
}

impl TryFrom<ContextDoc> for Context {
    type Error = Bad;

    fn try_from(d: ContextDoc) -> Result<Context, Bad> {
        let uses: BTreeSet<ContextId> = d
            .uses
            .iter()
            .map(|u| Ok(ContextId::from_uuid(uuid(u)?)))
            .collect::<Result<_, Bad>>()?;
        let basis = match d.basis {
            BasisDoc::Interface => Basis::Interface,
            BasisDoc::Contract => Basis::Contract,
        };
        Context::restore(
            ContextId::from_uuid(uuid(&d.id)?),
            ContextName::try_from(d.name.as_str()).map_err(bad)?,
            basis,
            uses,
            d.contracts
                .into_iter()
                .map(Contract::try_from)
                .collect::<Result<_, _>>()?,
            d.interfaces
                .into_iter()
                .map(Interface::try_from)
                .collect::<Result<_, _>>()?,
            d.harnesses
                .into_iter()
                .map(Harness::try_from)
                .collect::<Result<_, _>>()?,
        )
        .map_err(bad)
    }
}

impl TryFrom<ContractDoc> for Contract {
    type Error = Bad;

    fn try_from(d: ContractDoc) -> Result<Contract, Bad> {
        let mut clauses = Vec::with_capacity(d.clauses.len());
        for c in &d.clauses {
            clauses.push((
                ClauseId::from_uuid(uuid(&c.id)?),
                revision(c.revision)?,
                ClauseContent::new(
                    ClauseLabel::try_from(c.label.as_str()).map_err(bad)?,
                    ClauseText::try_from(c.text.as_str()).map_err(bad)?,
                ),
            ));
        }
        Contract::restore(
            ContractId::from_uuid(uuid(&d.id)?),
            revision(d.revision)?,
            Title::try_from(d.title.as_str()).map_err(bad)?,
            clauses,
        )
        .map_err(bad)
    }
}

impl TryFrom<InterfaceDoc> for Interface {
    type Error = Bad;

    fn try_from(d: InterfaceDoc) -> Result<Interface, Bad> {
        let mut items = Vec::with_capacity(d.items.len());
        for it in &d.items {
            items.push((
                ItemId::from_uuid(uuid(&it.id)?),
                revision(it.revision)?,
                Signature::try_from(it.signature.as_str()).map_err(bad)?,
                lines(it.first, it.last)?,
            ));
        }
        Interface::restore(
            InterfaceId::from_uuid(uuid(&d.id)?),
            revision(d.revision)?,
            Source::new(d.source),
            items,
        )
        .map_err(bad)
    }
}

impl TryFrom<HarnessDoc> for Harness {
    type Error = Bad;

    fn try_from(d: HarnessDoc) -> Result<Harness, Bad> {
        let mut cases = Vec::with_capacity(d.cases.len());
        for c in &d.cases {
            let cites = CaseCites::new(
                unrevs(&c.items, ItemId::from_uuid)?,
                unrevs(&c.clauses, ClauseId::from_uuid)?,
            )
            .map_err(bad)?;
            cases.push((
                CaseId::from_uuid(uuid(&c.id)?),
                revision(c.revision)?,
                CaseName::try_from(c.name.as_str()).map_err(bad)?,
                lines(c.first, c.last)?,
                cites,
            ));
        }
        Harness::restore(
            HarnessId::from_uuid(uuid(&d.id)?),
            revision(d.revision)?,
            Source::new(d.source),
            cases,
        )
        .map_err(bad)
    }
}

// -------------------------------------------------------------------- Pin

/// A Pin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PinDoc {
    pub schema: u32,
    id: String,
    interface: String,
    code: String,
    basis: PinBasisDoc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PinBasisDoc {
    interface: u64,
    harnesses: Vec<Rev>,
    contracts: Vec<Rev>,
    used_interfaces: Vec<Rev>,
}

impl From<&Pin> for PinDoc {
    fn from(p: &Pin) -> PinDoc {
        let b = p.basis();
        PinDoc {
            schema: SCHEMA,
            id: p.id().as_uuid().to_string(),
            interface: p.interface().as_uuid().to_string(),
            code: p.code().to_string(),
            basis: PinBasisDoc {
                interface: b.interface().number(),
                harnesses: revs(b.harnesses(), |i: HarnessId| i.as_uuid()),
                contracts: revs(b.contracts(), |i: ContractId| i.as_uuid()),
                used_interfaces: revs(b.used_interfaces(), |i: InterfaceId| i.as_uuid()),
            },
        }
    }
}

impl TryFrom<PinDoc> for Pin {
    type Error = Bad;

    fn try_from(d: PinDoc) -> Result<Pin, Bad> {
        let code =
            Digest::restore(&d.code).ok_or_else(|| format!("`{}` is not a digest", d.code))?;
        let basis = PinBasis::new(
            revision(d.basis.interface)?,
            unrevs(&d.basis.harnesses, HarnessId::from_uuid)?,
            unrevs(&d.basis.contracts, ContractId::from_uuid)?,
            unrevs(&d.basis.used_interfaces, InterfaceId::from_uuid)?,
        );
        Ok(Pin::new(
            PinId::from_uuid(uuid(&d.id)?),
            InterfaceId::from_uuid(uuid(&d.interface)?),
            code,
            basis,
        ))
    }
}

// ---------------------------------------------------------- ChangeRequest

/// A change request, with its Crux and the decision on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChangeRequestDoc {
    pub schema: u32,
    id: String,
    title: String,
    situation: Option<String>,
    intent: IntentDoc,
    parent: Option<String>,
    issuer: Option<String>,
    lines: Vec<LineDoc>,
    crux: Option<CruxDoc>,
    decision: Option<DecisionDoc>,
    resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IntentDoc {
    ChangeBehavior,
    KeepBehavior,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TargetDoc {
    Item,
    Clause,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LineDoc {
    target: TargetDoc,
    id: String,
    revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CruxKindDoc {
    Ambiguity,
    Conflict,
    Model,
    Disagreement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CruxDoc {
    kind: CruxKindDoc,
    options: Vec<OptionDoc>,
    default: Option<ReasonedDoc>,
    recommended: Option<ReasonedDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptionDoc {
    choice: String,
    change: Option<String>,
    consequence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReasonedDoc {
    choice: String,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DecisionDoc {
    Chosen(ReasonedDoc),
    Worded {
        wording: String,
        reason: Option<String>,
    },
    Unspecified {
        keep: Vec<String>,
    },
}

impl From<&Reasoned> for ReasonedDoc {
    fn from(r: &Reasoned) -> ReasonedDoc {
        ReasonedDoc {
            choice: r.choice().to_string(),
            reason: r.reason().map(|p| p.to_string()),
        }
    }
}

impl TryFrom<&ReasonedDoc> for Reasoned {
    type Error = Bad;

    fn try_from(d: &ReasonedDoc) -> Result<Reasoned, Bad> {
        Ok(Reasoned::new(
            Choice::try_from(d.choice.as_str()).map_err(bad)?,
            prose(&d.reason)?,
        ))
    }
}

impl From<&ChangeRequest> for ChangeRequestDoc {
    fn from(c: &ChangeRequest) -> ChangeRequestDoc {
        ChangeRequestDoc {
            schema: SCHEMA,
            id: c.id().as_uuid().to_string(),
            title: c.title().to_string(),
            situation: c.situation().map(|p| p.to_string()),
            intent: match c.intent() {
                Intent::ChangeBehavior => IntentDoc::ChangeBehavior,
                Intent::KeepBehavior => IntentDoc::KeepBehavior,
            },
            parent: c.parent().map(|p| p.as_uuid().to_string()),
            issuer: c.issuer().map(|i| i.to_string()),
            lines: c
                .lines()
                .iter()
                .map(|(t, r)| match t {
                    Target::Item(i) => LineDoc {
                        target: TargetDoc::Item,
                        id: i.as_uuid().to_string(),
                        revision: r.number(),
                    },
                    Target::Clause(i) => LineDoc {
                        target: TargetDoc::Clause,
                        id: i.as_uuid().to_string(),
                        revision: r.number(),
                    },
                })
                .collect(),
            crux: c.crux().map(|q| CruxDoc {
                kind: match q.kind() {
                    CruxKind::Ambiguity => CruxKindDoc::Ambiguity,
                    CruxKind::Conflict => CruxKindDoc::Conflict,
                    CruxKind::Model => CruxKindDoc::Model,
                    CruxKind::Disagreement => CruxKindDoc::Disagreement,
                },
                options: q
                    .options()
                    .iter()
                    .map(|o| OptionDoc {
                        choice: o.choice().to_string(),
                        change: o.change().map(|p| p.to_string()),
                        consequence: o.consequence().map(|p| p.to_string()),
                    })
                    .collect(),
                default: q.default().map(ReasonedDoc::from),
                recommended: q.recommended().map(ReasonedDoc::from),
            }),
            decision: c.crux().and_then(|q| q.decision()).map(|d| match d {
                Decision::Chosen(r) => DecisionDoc::Chosen(ReasonedDoc::from(r)),
                Decision::Worded { wording, reason } => DecisionDoc::Worded {
                    wording: wording.to_string(),
                    reason: reason.as_ref().map(|p| p.to_string()),
                },
                Decision::Unspecified { keep } => DecisionDoc::Unspecified {
                    keep: keep.iter().map(|p| p.to_string()).collect(),
                },
            }),
            resolved: c.state() == ChangeRequestState::Resolved,
        }
    }
}

impl TryFrom<ChangeRequestDoc> for ChangeRequest {
    type Error = Bad;

    fn try_from(d: ChangeRequestDoc) -> Result<ChangeRequest, Bad> {
        let mut lines = BTreeMap::new();
        for l in &d.lines {
            let id = uuid(&l.id)?;
            let target = match l.target {
                TargetDoc::Item => Target::Item(ItemId::from_uuid(id)),
                TargetDoc::Clause => Target::Clause(ClauseId::from_uuid(id)),
            };
            lines.insert(target, revision(l.revision)?);
        }
        let route = match &d.crux {
            None => Route::Machine,
            Some(q) => {
                let mut options = Vec::with_capacity(q.options.len());
                for o in &q.options {
                    options.push(CruxOption::new(
                        Choice::try_from(o.choice.as_str()).map_err(bad)?,
                        prose(&o.change)?,
                        prose(&o.consequence)?,
                    ));
                }
                let kind = match q.kind {
                    CruxKindDoc::Ambiguity => CruxKind::Ambiguity,
                    CruxKindDoc::Conflict => CruxKind::Conflict,
                    CruxKindDoc::Model => CruxKind::Model,
                    CruxKindDoc::Disagreement => CruxKind::Disagreement,
                };
                Route::Person(
                    Crux::new(
                        kind,
                        options,
                        q.default.as_ref().map(Reasoned::try_from).transpose()?,
                        q.recommended.as_ref().map(Reasoned::try_from).transpose()?,
                    )
                    .map_err(bad)?,
                )
            }
        };
        let decision = match &d.decision {
            None => None,
            Some(DecisionDoc::Chosen(r)) => Some(Decision::Chosen(Reasoned::try_from(r)?)),
            Some(DecisionDoc::Worded { wording, reason }) => Some(Decision::Worded {
                wording: Prose::try_from(wording.as_str()).map_err(bad)?,
                reason: prose(reason)?,
            }),
            Some(DecisionDoc::Unspecified { keep }) => Some(Decision::Unspecified {
                keep: keep
                    .iter()
                    .map(|k| Prose::try_from(k.as_str()).map_err(bad))
                    .collect::<Result<_, _>>()?,
            }),
        };
        let draft = ChangeRequestDraft {
            title: Title::try_from(d.title.as_str()).map_err(bad)?,
            situation: prose(&d.situation)?,
            intent: match d.intent {
                IntentDoc::ChangeBehavior => Intent::ChangeBehavior,
                IntentDoc::KeepBehavior => Intent::KeepBehavior,
            },
            parent: d
                .parent
                .as_deref()
                .map(|p| Ok::<_, Bad>(ChangeRequestId::from_uuid(uuid(p)?)))
                .transpose()?,
            issuer: d
                .issuer
                .as_deref()
                .map(|i| Issuer::try_from(i).map_err(bad))
                .transpose()?,
            lines,
            route,
        };
        ChangeRequest::restore(
            ChangeRequestId::from_uuid(uuid(&d.id)?),
            draft,
            decision,
            d.resolved,
        )
        .map_err(bad)
    }
}
