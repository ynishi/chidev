//! Schemas 1 and 2 of the stored documents. These types are a versioned storage format, not a copy of the domain: they
//! must stay readable as the domain changes, and they reach the domain only through its `restore` functions, which
//! re-check every rule.
//!
//! Schema 2 differs from schema 1 in the Items of a Project's Interfaces alone: an Item is kept by its key, not by
//! its signature (the Interface derives the signature from its code and the Item's lines). The Pin and change
//! request documents have the same shape in both. A Project document of schema 1 is converted on these types, in
//! [`ProjectDoc::convert_schema_1`], before the domain is built from it: that is the one place the old shape is known.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft, Crux};
use crate::domain::entity::context::{Basis, Context, ContextHead};
use crate::domain::entity::contract::Contract;
use crate::domain::entity::harness::Harness;
use crate::domain::entity::interface::{Interface, InterfaceError};
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::answer::{Answer, Basis as AnswerBasis, DecisionMaker};
use crate::domain::vo::assumption::{Assumption, Evidence, KnowledgeRef, RepoPath};
use crate::domain::vo::case::{CaseCites, CaseName};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
use crate::domain::vo::confidence::Confidence;
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Importance, Reasoned, Recommended};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
use crate::domain::vo::footprint::{Footprint, Move};
use crate::domain::vo::harness_kind::HarnessKind;
use crate::domain::vo::id::{
    CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
    PinId, ProjectId,
};
use crate::domain::vo::intent::{Intent, Issuer};
use crate::domain::vo::item_key::ItemKey;
use crate::domain::vo::line_range::LineRange;
use crate::domain::vo::name::ContextName;
use crate::domain::vo::pin_basis::PinBasis;
use crate::domain::vo::policy::{Policy, PolicyLine, PolicyLineId};
use crate::domain::vo::prose::Prose;
use crate::domain::vo::resolution::{Reopening, Resolution, ResolutionKind, Transition};
use crate::domain::vo::retired::Retired;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::source::Source;
use crate::domain::vo::target::{ExpectKind, Expectation, Target};
use crate::domain::vo::timestamp::Timestamp;
use crate::domain::vo::title::Title;

/// The schema number these documents are written with.
pub(crate) const SCHEMA: u32 = 2;

/// The schema before an Item was kept by its key.
const SCHEMA_1: u32 = 1;

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
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectDoc {
    pub schema: u32,
    id: String,
    /// The revision of the Project; a document without it is from before the Project had one and does not load.
    revision: u64,
    contexts: Vec<ContextDoc>,
    /// The decision Policy; a document without it is from before the Project had one and does not load.
    policy: PolicyDoc,
    /// What the Project retired, in the order it was retired; a document without it does not load.
    retired: Vec<RetiredDoc>,
}

/// A thing the Project retired: names and revisions, never the text. `holder` and `label` are present for a Clause,
/// an Item and a Case, and absent for the rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetiredDoc {
    target: TargetDoc,
    id: String,
    context: String,
    holder: Option<String>,
    label: Option<String>,
    last_revision: u64,
    at: u64,
}

impl From<&Retired> for RetiredDoc {
    fn from(r: &Retired) -> RetiredDoc {
        let (target, id) = target_doc(&r.target());
        RetiredDoc {
            target,
            id: id.unwrap_or_default(),
            context: r.context().to_string(),
            holder: r.holder().map(|h| h.to_string()),
            label: r.label().map(str::to_string),
            last_revision: r.last_revision().number(),
            at: r.at().number(),
        }
    }
}

impl TryFrom<RetiredDoc> for Retired {
    type Error = Bad;

    fn try_from(d: RetiredDoc) -> Result<Retired, Bad> {
        Retired::restore(
            target_of(&d.target, &Some(d.id))?,
            ContextName::try_from(d.context.as_str()).map_err(bad)?,
            d.holder.as_deref().map(uuid).transpose()?,
            d.label,
            revision(d.last_revision)?,
            revision(d.at)?,
        )
        .map_err(bad)
    }
}

/// The decision Policy of a Project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyDoc {
    revision: u64,
    lines: Vec<PolicyLineDoc>,
}

/// One line of the decision Policy and the revision at which it got its text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyLineDoc {
    id: String,
    text: String,
    changed_at: u64,
}

impl From<&Policy> for PolicyDoc {
    fn from(p: &Policy) -> PolicyDoc {
        PolicyDoc {
            revision: p.revision().number(),
            lines: p
                .lines()
                .iter()
                .map(|l| PolicyLineDoc {
                    id: l.id().to_string(),
                    text: l.text().to_string(),
                    changed_at: l.changed_at().number(),
                })
                .collect(),
        }
    }
}

impl TryFrom<&PolicyDoc> for Policy {
    type Error = Bad;

    fn try_from(d: &PolicyDoc) -> Result<Policy, Bad> {
        let lines = d
            .lines
            .iter()
            .map(|l| {
                Ok(PolicyLine::restore(
                    PolicyLineId::try_from(l.id.as_str()).map_err(bad)?,
                    Prose::try_from(l.text.as_str()).map_err(bad)?,
                    revision(l.changed_at)?,
                ))
            })
            .collect::<Result<Vec<_>, Bad>>()?;
        Policy::restore(revision(d.revision)?, lines).map_err(bad)
    }
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
    /// The revision of the Context; a document without it does not load.
    revision: u64,
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

/// An Item: a document of schema 2 has its `key`, and one of schema 1 its `signature` (which the conversion of
/// [`ProjectDoc::convert_schema_1`] turns into the key). The signature is not kept any more: the Interface derives it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ItemDoc {
    id: String,
    revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    first: usize,
    last: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessDoc {
    id: String,
    revision: u64,
    /// The kind's word; left out for a Harness of tests, which is what every document written before kinds says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    /// The ids of the Interfaces a sim Harness builds; left out for a Harness that builds none (tests).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    builds: Vec<String>,
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

/// An old declaration or label as a key: each line without the white space at its ends, joined by one space (a
/// key is one line). A line with nothing in it is no line of the key, so a key never starts or ends with a space.
fn one_line(text: &str) -> String {
    text.lines()
        .flat_map(|line| line.split('\r'))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl ProjectDoc {
    /// Turns a document of schema 1 into one of schema 2, in place. Schema 1 named an Item by its signature and kept
    /// it; schema 2 names it by a key and derives the signature from the code and the Item's lines. The signature
    /// that was kept is dropped, and no revision moves (the Item is the same, written another way, so no Pin or
    /// citation goes stale). The key of an Item is its signature on one line (see [`one_line`]); an Interface that
    /// had one signature twice keeps the first as it is and gives the next ones ` #2`, ` #3`, ... in the order of
    /// the Items, each the least number that is not already the key of another Item of the Interface. What a Project
    /// retired names an Item by the same rule. That an Item's lines still hold a declaration is the domain's to
    /// say when the document is read.
    fn convert_schema_1(&mut self) -> Result<(), Bad> {
        for interface in self
            .contexts
            .iter_mut()
            .flat_map(|c| c.interfaces.iter_mut())
        {
            let mut bases = Vec::with_capacity(interface.items.len());
            for it in &mut interface.items {
                let signature = it.signature.take().ok_or_else(|| {
                    format!(
                        "the item {} of the interface {} has no signature",
                        it.id, interface.id
                    )
                })?;
                if it.key.is_some() {
                    return Err(format!(
                        "the item {} of the interface {} has a key, which a document of schema 1 does not keep",
                        it.id, interface.id
                    ));
                }
                bases.push(one_line(&signature));
            }
            let everyone: BTreeSet<&str> = bases.iter().map(String::as_str).collect();
            let mut used: BTreeSet<String> = BTreeSet::new();
            for (it, base) in interface.items.iter_mut().zip(&bases) {
                let key = if used.contains(base) {
                    (2..)
                        .map(|n| format!("{base} #{n}"))
                        .find(|c| !used.contains(c) && !everyone.contains(c.as_str()))
                        .unwrap_or_default()
                } else {
                    base.clone()
                };
                used.insert(key.clone());
                it.key = Some(key);
            }
        }
        for r in &mut self.retired {
            if matches!(r.target, TargetDoc::Item) {
                r.label = r.label.take().map(|l| one_line(&l));
            }
        }
        self.schema = SCHEMA;
        Ok(())
    }
}

impl From<&Project> for ProjectDoc {
    fn from(p: &Project) -> ProjectDoc {
        ProjectDoc {
            schema: SCHEMA,
            id: p.id().as_uuid().to_string(),
            revision: p.revision().number(),
            contexts: p.contexts().map(ContextDoc::from).collect(),
            policy: PolicyDoc::from(p.policy()),
            retired: p.retired().iter().map(RetiredDoc::from).collect(),
        }
    }
}

impl From<&Context> for ContextDoc {
    fn from(c: &Context) -> ContextDoc {
        ContextDoc {
            id: c.id().as_uuid().to_string(),
            revision: c.revision().number(),
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
                    key: Some(it.key().to_string()),
                    signature: None,
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
            kind: match h.kind() {
                HarnessKind::Tests => None,
                other => Some(other.word().to_string()),
            },
            builds: h.builds().iter().map(|i| i.as_uuid().to_string()).collect(),
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

    fn try_from(mut d: ProjectDoc) -> Result<Project, Bad> {
        if d.schema == SCHEMA_1 {
            d.convert_schema_1()?;
        }
        let contexts = d
            .contexts
            .into_iter()
            .map(Context::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let retired = d
            .retired
            .into_iter()
            .map(Retired::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Project::restore(
            ProjectId::from_uuid(uuid(&d.id)?),
            revision(d.revision)?,
            contexts,
            Policy::try_from(&d.policy)?,
            retired,
        )
        .map_err(bad)
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
            ContextHead {
                id: ContextId::from_uuid(uuid(&d.id)?),
                revision: revision(d.revision)?,
                name: ContextName::try_from(d.name.as_str()).map_err(bad)?,
                basis,
                uses,
            },
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
            if it.signature.is_some() {
                return Err(format!(
                    "the item {} of the interface {} has a signature, which a document of this schema does not keep",
                    it.id, d.id
                ));
            }
            let key = it.key.as_deref().ok_or_else(|| {
                format!("the item {} of the interface {} has no key", it.id, d.id)
            })?;
            items.push((
                ItemId::from_uuid(uuid(&it.id)?),
                revision(it.revision)?,
                ItemKey::try_from(key).map_err(bad)?,
                lines(it.first, it.last)?,
            ));
        }
        let InterfaceDoc {
            id,
            revision: rev,
            source,
            items: docs,
        } = d;
        Interface::restore(
            InterfaceId::from_uuid(uuid(&id)?),
            revision(rev)?,
            Source::new(source),
            items,
        )
        .map_err(|e| match e {
            InterfaceError::BlankItem { id: item } => {
                let key = docs
                    .iter()
                    .find(|it| it.id == item.as_uuid().to_string())
                    .and_then(|it| it.key.as_deref())
                    .unwrap_or_default();
                format!(
                    "the interface {id} has the item {item} (`{key}`) on lines with nothing in them, so it declares nothing"
                )
            }
            other => format!("the interface {id}: {other}"),
        })
    }
}

impl TryFrom<HarnessDoc> for Harness {
    type Error = Bad;

    fn try_from(d: HarnessDoc) -> Result<Harness, Bad> {
        let mut cases = Vec::with_capacity(d.cases.len());
        for c in &d.cases {
            let cites = CaseCites::of(
                unrevs(&c.items, ItemId::from_uuid)?,
                unrevs(&c.clauses, ClauseId::from_uuid)?,
            );
            cases.push((
                CaseId::from_uuid(uuid(&c.id)?),
                revision(c.revision)?,
                CaseName::try_from(c.name.as_str()).map_err(bad)?,
                lines(c.first, c.last)?,
                cites,
            ));
        }
        let kind = match d.kind.as_deref() {
            None => HarnessKind::Tests,
            Some(word) => HarnessKind::from_word(word)
                .ok_or_else(|| bad(format!("a harness of kind `{word}`")))?,
        };
        let builds = d
            .builds
            .iter()
            .map(|i| uuid(i).map(InterfaceId::from_uuid))
            .collect::<Result<BTreeSet<_>, Bad>>()?;
        Harness::restore(
            HarnessId::from_uuid(uuid(&d.id)?),
            kind,
            revision(d.revision)?,
            builds,
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

/// A change request, with its Crux (and the answer to it) and its Assumptions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeRequestDoc {
    pub schema: u32,
    id: String,
    title: String,
    situation: Option<String>,
    intent: IntentDoc,
    parent: Option<String>,
    issuer: Option<String>,
    expects: Vec<ExpectDoc>,
    crux: Option<CruxDoc>,
    /// What the option decided expects, with the revision each target had at the decision.
    decided_expects: Vec<ExpectDoc>,
    /// The interpretations it records; a document without them is from before they existed and does not load.
    assumptions: Vec<AssumptionDoc>,
    /// The findings it deals with.
    addresses: Vec<AddressDoc>,
    /// What the change sets applied for it moved, in the order they were kept.
    work: Vec<FootprintDoc>,
    /// Every resolution and reopening, in the order they happened; resolved when the last is a resolution.
    history: Vec<TransitionDoc>,
}

/// One step of the history of a change request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TransitionDoc {
    Resolved(ResolutionDoc),
    Reopened(ReopeningDoc),
}

/// How a change request ended, and why, by whom and when (milliseconds since the epoch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionDoc {
    kind: ResolutionKindDoc,
    reason: String,
    by: MakerDoc,
    at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResolutionKindDoc {
    Done,
    WontDo,
    Duplicate { of: String },
    Superseded { by: String },
    Invalid,
}

/// Why a change request was opened again, by whom and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReopeningDoc {
    reason: String,
    by: MakerDoc,
    at: u64,
}

fn maker_doc(by: &DecisionMaker) -> MakerDoc {
    match by {
        DecisionMaker::Role(n) => MakerDoc::Role(n.to_string()),
        DecisionMaker::Person(n) => MakerDoc::Person(n.to_string()),
    }
}

fn maker_of(by: &MakerDoc) -> Result<DecisionMaker, Bad> {
    match by {
        MakerDoc::Role(n) => DecisionMaker::role(n),
        MakerDoc::Person(n) => DecisionMaker::person(n),
    }
    .map_err(bad)
}

impl From<&Transition> for TransitionDoc {
    fn from(t: &Transition) -> TransitionDoc {
        match t {
            Transition::Resolved(r) => TransitionDoc::Resolved(ResolutionDoc {
                kind: match r.kind() {
                    ResolutionKind::Done => ResolutionKindDoc::Done,
                    ResolutionKind::WontDo => ResolutionKindDoc::WontDo,
                    ResolutionKind::Duplicate { of } => ResolutionKindDoc::Duplicate {
                        of: of.as_uuid().to_string(),
                    },
                    ResolutionKind::Superseded { by } => ResolutionKindDoc::Superseded {
                        by: by.as_uuid().to_string(),
                    },
                    ResolutionKind::Invalid => ResolutionKindDoc::Invalid,
                },
                reason: r.reason().to_string(),
                by: maker_doc(r.by()),
                at: r.at().millis(),
            }),
            Transition::Reopened(r) => TransitionDoc::Reopened(ReopeningDoc {
                reason: r.reason().to_string(),
                by: maker_doc(r.by()),
                at: r.at().millis(),
            }),
        }
    }
}

impl TryFrom<&TransitionDoc> for Transition {
    type Error = Bad;

    fn try_from(d: &TransitionDoc) -> Result<Transition, Bad> {
        let id = |s: &str| Ok::<_, Bad>(ChangeRequestId::from_uuid(uuid(s)?));
        Ok(match d {
            TransitionDoc::Resolved(r) => Transition::Resolved(Resolution::new(
                match &r.kind {
                    ResolutionKindDoc::Done => ResolutionKind::Done,
                    ResolutionKindDoc::WontDo => ResolutionKind::WontDo,
                    ResolutionKindDoc::Duplicate { of } => {
                        ResolutionKind::Duplicate { of: id(of)? }
                    }
                    ResolutionKindDoc::Superseded { by } => {
                        ResolutionKind::Superseded { by: id(by)? }
                    }
                    ResolutionKindDoc::Invalid => ResolutionKind::Invalid,
                },
                Prose::try_from(r.reason.as_str()).map_err(bad)?,
                maker_of(&r.by)?,
                Timestamp::from_millis(r.at),
            )),
            TransitionDoc::Reopened(r) => Transition::Reopened(Reopening::new(
                Prose::try_from(r.reason.as_str()).map_err(bad)?,
                maker_of(&r.by)?,
                Timestamp::from_millis(r.at),
            )),
        })
    }
}

/// A target, as a container of a move is kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TargetRefDoc {
    target: TargetDoc,
    id: Option<String>,
}

/// A target a change set moved, with the containers it was in (the outermost first).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MoveDoc {
    target: TargetDoc,
    id: Option<String>,
    within: Vec<TargetRefDoc>,
}

/// What one change set applied for a change request moved, and the store revision it was kept at.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FootprintDoc {
    store_revision: u64,
    moves: Vec<MoveDoc>,
}

impl From<&Footprint> for FootprintDoc {
    fn from(f: &Footprint) -> FootprintDoc {
        FootprintDoc {
            store_revision: f.store_revision(),
            moves: f
                .moves()
                .iter()
                .map(|m| {
                    let (target, id) = target_doc(&m.target());
                    MoveDoc {
                        target,
                        id,
                        within: m
                            .within()
                            .iter()
                            .map(|t| {
                                let (target, id) = target_doc(t);
                                TargetRefDoc { target, id }
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }
}

impl TryFrom<&FootprintDoc> for Footprint {
    type Error = Bad;

    fn try_from(d: &FootprintDoc) -> Result<Footprint, Bad> {
        let moves = d
            .moves
            .iter()
            .map(|m| {
                let within = m
                    .within
                    .iter()
                    .map(|t| target_of(&t.target, &t.id))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<_, Bad>(Move::new(target_of(&m.target, &m.id)?, within))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Footprint::new(d.store_revision, moves))
    }
}

/// A finding a change request addresses: the rule, and what it is about.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AddressDoc {
    rule: String,
    subject: Option<SubjectDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SubjectKindDoc {
    Context,
    Interface,
    Item,
    Clause,
    Case,
    ChangeRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubjectDoc {
    kind: SubjectKindDoc,
    id: String,
}

impl From<&FindingRef> for AddressDoc {
    fn from(f: &FindingRef) -> AddressDoc {
        let (kind, id) = match f.subject() {
            None => (None, String::new()),
            Some(Subject::Context(i)) => (Some(SubjectKindDoc::Context), i.as_uuid().to_string()),
            Some(Subject::Interface(i)) => {
                (Some(SubjectKindDoc::Interface), i.as_uuid().to_string())
            }
            Some(Subject::Item(i)) => (Some(SubjectKindDoc::Item), i.as_uuid().to_string()),
            Some(Subject::Clause(i)) => (Some(SubjectKindDoc::Clause), i.as_uuid().to_string()),
            Some(Subject::Case(i)) => (Some(SubjectKindDoc::Case), i.as_uuid().to_string()),
            Some(Subject::ChangeRequest(i)) => {
                (Some(SubjectKindDoc::ChangeRequest), i.as_uuid().to_string())
            }
        };
        AddressDoc {
            rule: f.rule().to_string(),
            subject: kind.map(|kind| SubjectDoc { kind, id }),
        }
    }
}

impl TryFrom<&AddressDoc> for FindingRef {
    type Error = Bad;

    fn try_from(d: &AddressDoc) -> Result<FindingRef, Bad> {
        let subject = match &d.subject {
            None => None,
            Some(s) => {
                let id = uuid(&s.id)?;
                Some(match s.kind {
                    SubjectKindDoc::Context => Subject::Context(ContextId::from_uuid(id)),
                    SubjectKindDoc::Interface => Subject::Interface(InterfaceId::from_uuid(id)),
                    SubjectKindDoc::Item => Subject::Item(ItemId::from_uuid(id)),
                    SubjectKindDoc::Clause => Subject::Clause(ClauseId::from_uuid(id)),
                    SubjectKindDoc::Case => Subject::Case(CaseId::from_uuid(id)),
                    SubjectKindDoc::ChangeRequest => {
                        Subject::ChangeRequest(ChangeRequestId::from_uuid(id))
                    }
                })
            }
        };
        Ok(FindingRef::new(
            RuleId::try_from(d.rule.as_str()).map_err(bad)?,
            subject,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IntentDoc {
    ChangeBehavior,
    KeepBehavior,
}

/// The kind of a target. Every kind has an id but the Project, whose `id` is null.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TargetDoc {
    Project,
    Context,
    Contract,
    Interface,
    Harness,
    Clause,
    Item,
    Case,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExpectKindDoc {
    Target,
    Keep,
}

impl From<ExpectKind> for ExpectKindDoc {
    fn from(k: ExpectKind) -> ExpectKindDoc {
        match k {
            ExpectKind::Target => ExpectKindDoc::Target,
            ExpectKind::Keep => ExpectKindDoc::Keep,
        }
    }
}

impl From<ExpectKindDoc> for ExpectKind {
    fn from(k: ExpectKindDoc) -> ExpectKind {
        match k {
            ExpectKindDoc::Target => ExpectKind::Target,
            ExpectKindDoc::Keep => ExpectKind::Keep,
        }
    }
}

/// What a change request asks of a target (to revise or retire it, or to keep it), and the revision the target had
/// when it was recorded.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExpectDoc {
    target: TargetDoc,
    id: Option<String>,
    kind: ExpectKindDoc,
    recorded: u64,
}

/// What a Crux option expects of a target when it is chosen.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptionExpectDoc {
    target: TargetDoc,
    id: Option<String>,
    kind: ExpectKindDoc,
}

fn target_doc(t: &Target) -> (TargetDoc, Option<String>) {
    let id = |u: Uuid| Some(u.to_string());
    match t {
        Target::Project => (TargetDoc::Project, None),
        Target::Context(i) => (TargetDoc::Context, id(i.as_uuid())),
        Target::Contract(i) => (TargetDoc::Contract, id(i.as_uuid())),
        Target::Interface(i) => (TargetDoc::Interface, id(i.as_uuid())),
        Target::Harness(i) => (TargetDoc::Harness, id(i.as_uuid())),
        Target::Clause(i) => (TargetDoc::Clause, id(i.as_uuid())),
        Target::Item(i) => (TargetDoc::Item, id(i.as_uuid())),
        Target::Case(i) => (TargetDoc::Case, id(i.as_uuid())),
    }
}

fn target_of(kind: &TargetDoc, id: &Option<String>) -> Result<Target, Bad> {
    if let TargetDoc::Project = kind {
        return match id {
            None => Ok(Target::Project),
            Some(_) => Err("the project target has no id".to_string()),
        };
    }
    let id = uuid(id.as_deref().ok_or("a target has no id")?)?;
    Ok(match kind {
        TargetDoc::Project => Target::Project,
        TargetDoc::Context => Target::Context(ContextId::from_uuid(id)),
        TargetDoc::Contract => Target::Contract(ContractId::from_uuid(id)),
        TargetDoc::Interface => Target::Interface(InterfaceId::from_uuid(id)),
        TargetDoc::Harness => Target::Harness(HarnessId::from_uuid(id)),
        TargetDoc::Clause => Target::Clause(ClauseId::from_uuid(id)),
        TargetDoc::Item => Target::Item(ItemId::from_uuid(id)),
        TargetDoc::Case => Target::Case(CaseId::from_uuid(id)),
    })
}

fn expect_docs(expects: &BTreeMap<Target, Expectation>) -> Vec<ExpectDoc> {
    expects
        .iter()
        .map(|(t, e)| {
            let (target, id) = target_doc(t);
            ExpectDoc {
                target,
                id,
                kind: e.kind.into(),
                recorded: e.recorded.number(),
            }
        })
        .collect()
}

/// The expectations in `docs`; a target twice is refused: a target has one expectation.
fn expects_of(docs: &[ExpectDoc]) -> Result<BTreeMap<Target, Expectation>, Bad> {
    let mut out = BTreeMap::new();
    for d in docs {
        let target = target_of(&d.target, &d.id)?;
        let expectation = Expectation {
            kind: d.kind.into(),
            recorded: revision(d.recorded)?,
        };
        if out.insert(target, expectation).is_some() {
            return Err(format!("{target:?} is expected twice"));
        }
    }
    Ok(out)
}

/// The expectations of a Crux option; a target twice is refused.
fn option_expects_of(docs: &[OptionExpectDoc]) -> Result<BTreeMap<Target, ExpectKind>, Bad> {
    let mut out = BTreeMap::new();
    for d in docs {
        let target = target_of(&d.target, &d.id)?;
        if out.insert(target, d.kind.into()).is_some() {
            return Err(format!("{target:?} is expected twice by one option"));
        }
    }
    Ok(out)
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
#[serde(deny_unknown_fields)]
struct CruxDoc {
    kind: CruxKindDoc,
    options: Vec<OptionDoc>,
    recommended: Option<RecommendedDoc>,
    importance: Option<ImportanceDoc>,
    /// The id of the change request whose answered Crux this one replaces.
    supersedes: Option<String>,
    answer: Option<AnswerDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecommendedDoc {
    choice: String,
    reason: Option<String>,
    confidence: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ImportanceDoc {
    Low,
    Normal,
    High,
}

/// The answer to a Crux: the decision, what it rests on, who gave it, and when (milliseconds since the epoch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerDoc {
    decision: DecisionDoc,
    basis: AnswerBasisDoc,
    by: MakerDoc,
    at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AnswerBasisDoc {
    Policy { line: String, policy_revision: u64 },
    Arbitrary { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MakerDoc {
    Role(String),
    Person(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssumptionDoc {
    statement: String,
    reason: String,
    evidence: Vec<EvidenceDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceDoc {
    Repo {
        path: String,
        lines: Option<(usize, usize)>,
    },
    Knowledge {
        reference: String,
    },
}

impl From<&Decision> for DecisionDoc {
    fn from(d: &Decision) -> DecisionDoc {
        match d {
            Decision::Chosen(r) => DecisionDoc::Chosen(ReasonedDoc::from(r)),
            Decision::Worded { wording, reason } => DecisionDoc::Worded {
                wording: wording.to_string(),
                reason: reason.as_ref().map(|p| p.to_string()),
            },
            Decision::Unspecified { keep } => DecisionDoc::Unspecified {
                keep: keep.iter().map(|p| p.to_string()).collect(),
            },
        }
    }
}

impl TryFrom<&DecisionDoc> for Decision {
    type Error = Bad;

    fn try_from(d: &DecisionDoc) -> Result<Decision, Bad> {
        Ok(match d {
            DecisionDoc::Chosen(r) => Decision::Chosen(Reasoned::try_from(r)?),
            DecisionDoc::Worded { wording, reason } => Decision::Worded {
                wording: Prose::try_from(wording.as_str()).map_err(bad)?,
                reason: prose(reason)?,
            },
            DecisionDoc::Unspecified { keep } => Decision::Unspecified {
                keep: keep
                    .iter()
                    .map(|k| Prose::try_from(k.as_str()).map_err(bad))
                    .collect::<Result<_, _>>()?,
            },
        })
    }
}

impl From<&Answer> for AnswerDoc {
    fn from(a: &Answer) -> AnswerDoc {
        AnswerDoc {
            decision: DecisionDoc::from(a.decision()),
            basis: match a.basis() {
                AnswerBasis::Policy {
                    line,
                    policy_revision,
                } => AnswerBasisDoc::Policy {
                    line: line.to_string(),
                    policy_revision: policy_revision.number(),
                },
                AnswerBasis::Arbitrary { reason } => AnswerBasisDoc::Arbitrary {
                    reason: reason.to_string(),
                },
            },
            by: match a.by() {
                DecisionMaker::Role(n) => MakerDoc::Role(n.to_string()),
                DecisionMaker::Person(n) => MakerDoc::Person(n.to_string()),
            },
            at: a.at().millis(),
        }
    }
}

impl TryFrom<&AnswerDoc> for Answer {
    type Error = Bad;

    fn try_from(d: &AnswerDoc) -> Result<Answer, Bad> {
        let basis = match &d.basis {
            AnswerBasisDoc::Policy {
                line,
                policy_revision,
            } => AnswerBasis::Policy {
                line: PolicyLineId::try_from(line.as_str()).map_err(bad)?,
                policy_revision: revision(*policy_revision)?,
            },
            AnswerBasisDoc::Arbitrary { reason } => AnswerBasis::Arbitrary {
                reason: Prose::try_from(reason.as_str()).map_err(bad)?,
            },
        };
        let by = match &d.by {
            MakerDoc::Role(n) => DecisionMaker::role(n),
            MakerDoc::Person(n) => DecisionMaker::person(n),
        }
        .map_err(bad)?;
        Ok(Answer::new(
            Decision::try_from(&d.decision)?,
            basis,
            by,
            Timestamp::from_millis(d.at),
        ))
    }
}

impl From<&Assumption> for AssumptionDoc {
    fn from(a: &Assumption) -> AssumptionDoc {
        AssumptionDoc {
            statement: a.statement().to_string(),
            reason: a.reason().to_string(),
            evidence: a
                .evidence()
                .iter()
                .map(|e| match e {
                    Evidence::Repo { path, lines } => EvidenceDoc::Repo {
                        path: path.to_string(),
                        lines: lines.map(|l| (l.first(), l.last())),
                    },
                    Evidence::Knowledge { reference } => EvidenceDoc::Knowledge {
                        reference: reference.to_string(),
                    },
                })
                .collect(),
        }
    }
}

impl TryFrom<&AssumptionDoc> for Assumption {
    type Error = Bad;

    fn try_from(d: &AssumptionDoc) -> Result<Assumption, Bad> {
        let mut evidence = Vec::with_capacity(d.evidence.len());
        for e in &d.evidence {
            evidence.push(match e {
                EvidenceDoc::Repo { path, lines: range } => Evidence::Repo {
                    path: RepoPath::try_from(path.as_str()).map_err(bad)?,
                    lines: range.map(|(first, last)| lines(first, last)).transpose()?,
                },
                EvidenceDoc::Knowledge { reference } => Evidence::Knowledge {
                    reference: KnowledgeRef::try_from(reference.as_str()).map_err(bad)?,
                },
            });
        }
        Ok(Assumption::new(
            Prose::try_from(d.statement.as_str()).map_err(bad)?,
            Prose::try_from(d.reason.as_str()).map_err(bad)?,
            evidence,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptionDoc {
    choice: String,
    change: Option<String>,
    expects: Vec<OptionExpectDoc>,
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
            expects: expect_docs(c.expects()),
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
                        expects: o
                            .expects()
                            .iter()
                            .map(|(t, k)| {
                                let (target, id) = target_doc(t);
                                OptionExpectDoc {
                                    target,
                                    id,
                                    kind: (*k).into(),
                                }
                            })
                            .collect(),
                        consequence: o.consequence().map(|p| p.to_string()),
                    })
                    .collect(),
                recommended: q.recommended().map(|r| RecommendedDoc {
                    choice: r.choice().to_string(),
                    reason: r.reason().map(|p| p.to_string()),
                    confidence: r.confidence().map(|c| u64::from(c.percent())),
                }),
                importance: q.importance().map(|i| match i {
                    Importance::Low => ImportanceDoc::Low,
                    Importance::Normal => ImportanceDoc::Normal,
                    Importance::High => ImportanceDoc::High,
                }),
                supersedes: q.supersedes().map(|s| s.as_uuid().to_string()),
                answer: q.answer().map(AnswerDoc::from),
            }),
            decided_expects: expect_docs(c.decided_expects()),
            assumptions: c.assumptions().iter().map(AssumptionDoc::from).collect(),
            addresses: c.addresses().iter().map(AddressDoc::from).collect(),
            work: c.work().iter().map(FootprintDoc::from).collect(),
            history: c.history().iter().map(TransitionDoc::from).collect(),
        }
    }
}

impl TryFrom<ChangeRequestDoc> for ChangeRequest {
    type Error = Bad;

    fn try_from(d: ChangeRequestDoc) -> Result<ChangeRequest, Bad> {
        let expects = expects_of(&d.expects)?;
        let decided_expects = expects_of(&d.decided_expects)?;
        let crux = match &d.crux {
            None => None,
            Some(q) => {
                let mut options = Vec::with_capacity(q.options.len());
                for o in &q.options {
                    options.push(CruxOption::new(
                        Choice::try_from(o.choice.as_str()).map_err(bad)?,
                        prose(&o.change)?,
                        option_expects_of(&o.expects)?,
                        prose(&o.consequence)?,
                    ));
                }
                let kind = match q.kind {
                    CruxKindDoc::Ambiguity => CruxKind::Ambiguity,
                    CruxKindDoc::Conflict => CruxKind::Conflict,
                    CruxKindDoc::Model => CruxKind::Model,
                    CruxKindDoc::Disagreement => CruxKind::Disagreement,
                };
                let recommended = q
                    .recommended
                    .as_ref()
                    .map(|r| {
                        Ok::<_, Bad>(Recommended::new(
                            Choice::try_from(r.choice.as_str()).map_err(bad)?,
                            prose(&r.reason)?,
                            r.confidence
                                .map(|c| Confidence::try_from(c).map_err(bad))
                                .transpose()?,
                        ))
                    })
                    .transpose()?;
                let importance = q.importance.map(|i| match i {
                    ImportanceDoc::Low => Importance::Low,
                    ImportanceDoc::Normal => Importance::Normal,
                    ImportanceDoc::High => Importance::High,
                });
                let supersedes = q
                    .supersedes
                    .as_deref()
                    .map(|s| Ok::<_, Bad>(ChangeRequestId::from_uuid(uuid(s)?)))
                    .transpose()?;
                Some(Crux::new(kind, options, recommended, importance, supersedes).map_err(bad)?)
            }
        };
        let answer = d
            .crux
            .as_ref()
            .and_then(|q| q.answer.as_ref())
            .map(Answer::try_from)
            .transpose()?;
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
            expects,
            crux,
            assumptions: d
                .assumptions
                .iter()
                .map(Assumption::try_from)
                .collect::<Result<_, _>>()?,
            addresses: d
                .addresses
                .iter()
                .map(FindingRef::try_from)
                .collect::<Result<_, _>>()?,
        };
        ChangeRequest::restore(
            ChangeRequestId::from_uuid(uuid(&d.id)?),
            draft,
            answer,
            decided_expects,
            d.work
                .iter()
                .map(Footprint::try_from)
                .collect::<Result<_, _>>()?,
            d.history
                .iter()
                .map(Transition::try_from)
                .collect::<Result<_, _>>()?,
        )
        .map_err(bad)
    }
}
