//! How the aggregates are written to the store and read back: one JSON document each, in a versioned schema
//! ([`v1`], and [`crux`] for the Crux). A document that is newer than this code, or that the domain's rules refuse,
//! is `Corrupt`, never quietly repaired.
//!
//! The schemas are numbered per kind of document: the Project and the Pin are at 2, the change request at 4 and the
//! Crux at 3. A Crux of schema 2 added where what its answer put outside its change request went, and schema 3 added
//! the corrections made of its texts; a Crux of an earlier schema is read as it is, with neither. A change request
//! of schema 4 may have corrections in its history, and a document of schema 3 has none and is read as it is. A change
//! request document of schema 1 or 2 held its Crux; it is read only by the migration of the store to layout 2
//! ([`layout2`]), which writes it again as schema 3, and never by [`decode_change_request`].

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::crux::Crux;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::StoreError;
use crate::domain::vo::resolution::Transition;

pub(crate) mod crux;
pub(crate) mod layout2;
pub(crate) mod measure;
pub(crate) mod v1;

use self::crux::CruxDoc;
use self::v1::{CR_SCHEMA, CR_SCHEMA_3, ChangeRequestDoc, PinDoc, ProjectDoc, SCHEMA};

/// The schema number a Project or Pin document is written with.
pub(crate) const CURRENT: u32 = SCHEMA;

/// The schema number a change request document is written with.
pub(crate) const CURRENT_CHANGE_REQUEST: u32 = CR_SCHEMA;

/// The schema number a Crux document is written with.
pub(crate) const CURRENT_CRUX: u32 = crux::SCHEMA;

fn encode<D: Serialize>(doc: &D) -> Result<String, StoreError> {
    serde_json::to_string(doc)
        .map_err(|e| StoreError::Failed(format!("cannot write a document: {e}")))
}

/// Reads a document of `stored` schema (the column beside it), of a kind whose newest schema is `current`; the
/// document's own `schema` must agree.
fn decode<D, T>(
    stored: u32,
    current: u32,
    text: &str,
    own: impl Fn(&D) -> u32,
) -> Result<T, StoreError>
where
    D: DeserializeOwned,
    T: TryFrom<D, Error = String>,
{
    if stored > current {
        return Err(StoreError::Corrupt(format!(
            "schema {stored} is newer than this version reads ({current})"
        )));
    }
    let doc: D = serde_json::from_str(text).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    if own(&doc) != stored {
        return Err(StoreError::Corrupt(format!(
            "the document says schema {}, its record says {stored}",
            own(&doc)
        )));
    }
    T::try_from(doc).map_err(StoreError::Corrupt)
}

pub(crate) fn encode_project(p: &Project) -> Result<String, StoreError> {
    encode(&ProjectDoc::from(p))
}

pub(crate) fn decode_project(stored: u32, text: &str) -> Result<Project, StoreError> {
    decode::<ProjectDoc, Project>(stored, CURRENT, text, |d| d.schema)
}

pub(crate) fn encode_pin(p: &Pin) -> Result<String, StoreError> {
    encode(&PinDoc::from(p))
}

pub(crate) fn decode_pin(stored: u32, text: &str) -> Result<Pin, StoreError> {
    decode::<PinDoc, Pin>(stored, CURRENT, text, |d| d.schema)
}

pub(crate) fn encode_change_request(c: &ChangeRequest) -> Result<String, StoreError> {
    encode(&ChangeRequestDoc::from(c))
}

/// Reads a change request document of schema 3 or 4: a 3 has no corrected step in its history, and is read as it is.
/// A 3 that has one was never written by a version that wrote 3, and is corrupt.
pub(crate) fn decode_change_request(stored: u32, text: &str) -> Result<ChangeRequest, StoreError> {
    if stored < CR_SCHEMA_3 {
        return Err(StoreError::Corrupt(format!(
            "a change request of schema {stored} is read by the migration of the store to layout 2, not as it is"
        )));
    }
    let cr =
        decode::<ChangeRequestDoc, ChangeRequest>(stored, CURRENT_CHANGE_REQUEST, text, |d| {
            d.schema
        })?;
    if stored == CR_SCHEMA_3
        && cr
            .history()
            .iter()
            .any(|step| matches!(step, Transition::Corrected(_)))
    {
        return Err(StoreError::Corrupt(
            "a change request of schema 3 has a corrected step in its history, which schema 3 never had".to_string(),
        ));
    }
    Ok(cr)
}

pub(crate) fn encode_crux(c: &Crux) -> Result<String, StoreError> {
    encode(&CruxDoc::from(c))
}

/// Reads a Crux document of schema 1, 2 or 3: a document of schema 1 or 2 has no corrections, and is read as it is.
/// One that has them was never written by a version that wrote 1 or 2, and is corrupt.
pub(crate) fn decode_crux(stored: u32, text: &str) -> Result<Crux, StoreError> {
    let crux = decode::<CruxDoc, Crux>(stored, CURRENT_CRUX, text, |d| d.schema)?;
    if stored < CURRENT_CRUX && !crux.corrections().is_empty() {
        return Err(StoreError::Corrupt(format!(
            "a Crux of schema {stored} has corrections, which it never had before schema {CURRENT_CRUX}"
        )));
    }
    Ok(crux)
}

#[cfg(test)]
pub(crate) mod fixture {
    //! One Project, Pin, change requests and Cruxes that use every part of the stored format.

    use std::collections::BTreeMap;

    use uuid::Uuid;

    use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft};
    use crate::domain::entity::context::Context;
    use crate::domain::entity::crux::{Crux, CruxDraft};
    use crate::domain::entity::pin::Pin;
    use crate::domain::entity::project::Project;
    use crate::domain::service::change_request::expectations_now;
    use crate::domain::service::crux::decide;
    use crate::domain::service::pin::record;
    use crate::domain::vo::answer::{Answer, Basis, DecisionMaker};
    use crate::domain::vo::assumption::{Assumption, Evidence, KnowledgeRef, RepoPath};
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::confidence::Confidence;
    use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Importance, Recommended};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
    use crate::domain::vo::id::{
        AnswerId, CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, CruxId, CruxOptionId,
        HarnessId, InterfaceId, ItemId, PinId, ProjectId,
    };
    use crate::domain::vo::intent::{Intent, Issuer};
    use crate::domain::vo::item_key::ItemKey;
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::policy::PolicyLineId;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::resolution::{Reopening, Resolution, ResolutionKind};
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::target::{ExpectKind, Target};
    use crate::domain::vo::timestamp::Timestamp;
    use crate::domain::vo::title::Title;

    pub(crate) fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    pub(crate) const IFACE: u128 = 30;

    /// Two Contexts (the second uses the first and rests on its Contract), with a Contract of two Clauses, an
    /// Interface revised once, a Harness citing an Item and a Clause, and a decision Policy of two lines of which
    /// the second was reworded.
    pub(crate) fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(id(1)));
        let line = |n: &str, t: &str| (PolicyLineId::try_from(n).unwrap(), prose(t));
        p.put_policy(vec![
            line("P1", "take the recommendation from 80"),
            line("P2", "low importance: either"),
        ])
        .unwrap();
        p.put_policy(vec![
            line("P1", "take the recommendation from 80"),
            line("P2", "low importance and equal cost: the recommendation"),
        ])
        .unwrap();
        let base = ContextId::from_uuid(id(10));
        let top = ContextId::from_uuid(id(11));
        let ctx = |c: ContextId, name: &str, uses: &[ContextId]| {
            Context::new(
                c,
                ContextName::try_from(name).unwrap(),
                uses.iter().copied().collect(),
            )
            .unwrap()
        };
        p.add_context(ctx(base, "base", &[])).unwrap();
        p.add_context(ctx(top, "top", &[base])).unwrap();
        let clause = |l: &str, t: &str| {
            ClauseContent::new(
                ClauseLabel::try_from(l).unwrap(),
                ClauseText::try_from(t).unwrap(),
            )
        };
        p.put_contract(
            top,
            ContractId::from_uuid(id(20)),
            Title::try_from("Stack").unwrap(),
            vec![
                (ClauseId::from_uuid(id(21)), clause("C1", "push then pop")),
                (
                    ClauseId::from_uuid(id(22)),
                    clause("C2", "pop of empty fails"),
                ),
            ],
        )
        .unwrap();
        p.base_on_contract(top).unwrap();
        let src = "pub fn push(v: u8)\npub fn pop() -> u8\n";
        p.put_interface(
            top,
            InterfaceId::from_uuid(id(IFACE)),
            Source::new(src.to_string()),
            vec![
                (
                    ItemId::from_uuid(id(31)),
                    ItemKey::try_from("push").unwrap(),
                    LineRange::new(1, 1).unwrap(),
                ),
                (
                    ItemId::from_uuid(id(32)),
                    ItemKey::try_from("pop").unwrap(),
                    LineRange::new(2, 2).unwrap(),
                ),
            ],
        )
        .unwrap();
        let cites = CaseCites::new(
            BTreeMap::from([(ItemId::from_uuid(id(31)), Revision::first())]),
            BTreeMap::from([(ClauseId::from_uuid(id(21)), Revision::first())]),
        )
        .unwrap();
        p.put_harness(
            top,
            HarnessId::from_uuid(id(40)),
            Source::new("#[test]\nfn push_then_pop() {}\n".to_string()),
            vec![(
                CaseId::from_uuid(id(41)),
                CaseName::try_from("push_then_pop").unwrap(),
                LineRange::new(1, 2).unwrap(),
                cites,
            )],
        )
        .unwrap();
        p
    }

    pub(crate) fn pin(p: &Project) -> Pin {
        record(
            p,
            InterfaceId::from_uuid(id(IFACE)),
            PinId::from_uuid(id(50)),
            Digest::of(b"code"),
        )
        .unwrap()
    }

    fn prose(t: &str) -> Prose {
        Prose::try_from(t).unwrap()
    }

    /// The id of the option `k` of the Crux numbered `n`.
    pub(crate) fn option_id(n: u128, k: u128) -> CruxOptionId {
        CruxOptionId::from_uuid(id(n * 100 + k))
    }

    fn answer(n: u128, decision: Decision, basis: Basis, by: DecisionMaker, at: u64) -> Answer {
        Answer::new(
            AnswerId::from_uuid(id(8000 + n)),
            decision,
            basis,
            by,
            Timestamp::from_millis(at),
        )
    }

    fn assumption() -> Assumption {
        Assumption::new(
            prose("pop of empty is a failure, not a zero"),
            prose("the sibling functions fail on empty input"),
            vec![
                Evidence::Repo {
                    path: RepoPath::try_from("src/stack.rs").unwrap(),
                    lines: Some(LineRange::new(10, 14).unwrap()),
                },
                Evidence::Repo {
                    path: RepoPath::try_from("README.md").unwrap(),
                    lines: None,
                },
                Evidence::Knowledge {
                    reference: KnowledgeRef::try_from("Rust API guidelines C-FAILURE").unwrap(),
                },
            ],
        )
    }

    /// A change request that names a target or a keep of every kind, resolved, reopened and resolved again, and one
    /// with a parent, an Assumption, and the expectations of a decision on a Policy line that names a target. The
    /// Cruxes are [`cruxes`].
    pub(crate) fn change_requests(p: &Project) -> Vec<ChangeRequest> {
        records(p).0
    }

    /// The Cruxes of [`change_requests`]: one raised in `pop of empty` with two options, a recommendation and an
    /// importance, decided on a Policy line; and one raised in `rename push` that is open, from findings. The
    /// Cruxes of [`other_decisions`] come after them.
    pub(crate) fn cruxes(p: &Project) -> Vec<Crux> {
        records(p).1
    }

    fn records(p: &Project) -> (Vec<ChangeRequest>, Vec<Crux>) {
        let targets = [
            Target::Item(ItemId::from_uuid(id(31))),
            Target::Clause(ClauseId::from_uuid(id(21))),
        ];
        let of_every_kind = [
            (Target::Project, ExpectKind::Target),
            (
                Target::Context(ContextId::from_uuid(id(11))),
                ExpectKind::Target,
            ),
            (
                Target::Contract(ContractId::from_uuid(id(20))),
                ExpectKind::Keep,
            ),
            (
                Target::Interface(InterfaceId::from_uuid(id(IFACE))),
                ExpectKind::Target,
            ),
            (
                Target::Harness(HarnessId::from_uuid(id(40))),
                ExpectKind::Keep,
            ),
            (
                Target::Clause(ClauseId::from_uuid(id(22))),
                ExpectKind::Target,
            ),
            (Target::Item(ItemId::from_uuid(id(31))), ExpectKind::Target),
            (Target::Case(CaseId::from_uuid(id(41))), ExpectKind::Target),
        ];
        let mut first = ChangeRequest::open(
            ChangeRequestId::from_uuid(id(60)),
            ChangeRequestDraft {
                title: Title::try_from("rename push").unwrap(),
                situation: None,
                intent: Intent::KeepBehavior,
                parent: None,
                issuer: None,
                expects: expectations_now(p, of_every_kind),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            },
        );
        let alice = || DecisionMaker::person("alice").unwrap();
        first
            .resolve(Resolution::new(
                ResolutionKind::Done,
                prose("it is done"),
                alice(),
                Timestamp::from_millis(1_700_000_000_001),
            ))
            .unwrap();
        first
            .reopen(Reopening::new(
                prose("a caller still breaks"),
                DecisionMaker::role("orchestrator").unwrap(),
                Timestamp::from_millis(1_700_000_000_002),
            ))
            .unwrap();
        first
            .resolve(Resolution::new(
                ResolutionKind::Superseded {
                    by: ChangeRequestId::from_uuid(id(61)),
                },
                prose("the other one carries it"),
                alice(),
                Timestamp::from_millis(1_700_000_000_003),
            ))
            .unwrap();
        let mut second = ChangeRequest::open(
            ChangeRequestId::from_uuid(id(61)),
            ChangeRequestDraft {
                title: Title::try_from("pop of empty").unwrap(),
                situation: Some(prose("it is not said")),
                intent: Intent::ChangeBehavior,
                parent: Some(ChangeRequestId::from_uuid(id(60))),
                issuer: Some(Issuer::try_from("yutaka").unwrap()),
                // The Interface frames the Item the decision asks, which the request does not name.
                expects: expectations_now(
                    p,
                    targets
                        .into_iter()
                        .chain([Target::Interface(InterfaceId::from_uuid(id(IFACE)))])
                        .map(|t| (t, ExpectKind::Target)),
                ),
                assumptions: vec![assumption()],
                addresses: vec![
                    FindingRef::new(
                        RuleId::from_static("cite.uncited-clause"),
                        Some(Subject::Clause(ClauseId::from_uuid(id(22)))),
                    ),
                    FindingRef::new(RuleId::from_static("context.no-harness"), None),
                ],
            },
        );
        let mut asked = crux(
            62,
            second.id(),
            CruxKind::Ambiguity,
            "what does pop of an empty stack do?",
            vec![
                (
                    "fail",
                    Some("C2 says fail"),
                    BTreeMap::new(),
                    Some("callers check"),
                ),
                (
                    "zero",
                    None,
                    BTreeMap::from([
                        (targets[1], ExpectKind::Target),
                        (Target::Item(ItemId::from_uuid(id(32))), ExpectKind::Target),
                    ]),
                    None,
                ),
            ],
        );
        asked = with_recommendation(asked, 1);
        let basis = p
            .policy()
            .basis_on(PolicyLineId::try_from("P1").unwrap())
            .unwrap();
        decide(
            &mut asked,
            &mut second,
            answer(
                62,
                Decision::Chosen {
                    option: option_id(62, 1),
                    reason: Some(prose("because")),
                },
                basis,
                DecisionMaker::role("orchestrator").unwrap(),
                1_700_000_000_123,
            ),
            p,
            None,
            &[],
        )
        .unwrap();
        // An open question raised from findings, in a request that was resolved without it: a record of what was.
        let mut open = Crux::raise(
            CruxId::from_uuid(id(63)),
            CruxDraft {
                raised_in: first.id(),
                findings: vec![
                    FindingRef::new(
                        RuleId::from_static("cite.uncited-clause"),
                        Some(Subject::Clause(ClauseId::from_uuid(id(21)))),
                    ),
                    FindingRef::new(RuleId::from_static("context.no-harness"), None),
                ],
                kind: CruxKind::Model,
                question: prose("is a stack a list?"),
                options: vec![
                    CruxOption::new(
                        option_id(63, 0),
                        Choice::try_from("yes").unwrap(),
                        None,
                        BTreeMap::new(),
                        None,
                    ),
                    CruxOption::new(
                        option_id(63, 1),
                        Choice::try_from("no").unwrap(),
                        Some(prose("a Stack is not a List")),
                        BTreeMap::new(),
                        None,
                    ),
                ],
                recommended: None,
                importance: None,
                supersedes: None,
            },
        )
        .unwrap();
        open = with_importance(open, Importance::Low);
        (vec![first, second], vec![asked, open])
    }

    /// An open Crux numbered `n` raised in `raised_in`, with the options `(choice, change, expects, consequence)`;
    /// the option `k` has the id `option_id(n, k)`.
    #[allow(clippy::type_complexity)]
    fn crux(
        n: u128,
        raised_in: ChangeRequestId,
        kind: CruxKind,
        question: &str,
        options: Vec<(
            &str,
            Option<&str>,
            BTreeMap<Target, ExpectKind>,
            Option<&str>,
        )>,
    ) -> Crux {
        Crux::raise(
            CruxId::from_uuid(id(n)),
            CruxDraft {
                raised_in,
                findings: Vec::new(),
                kind,
                question: prose(question),
                options: options
                    .into_iter()
                    .enumerate()
                    .map(|(k, (choice, change, expects, consequence))| {
                        CruxOption::new(
                            option_id(n, k as u128),
                            Choice::try_from(choice).unwrap(),
                            change.map(prose),
                            expects,
                            consequence.map(prose),
                        )
                    })
                    .collect(),
                recommended: None,
                importance: Some(Importance::High),
                supersedes: None,
            },
        )
        .unwrap()
    }

    /// `crux` again, raised with the option `k` recommended, with a reason and 85 sure.
    fn with_recommendation(crux: Crux, k: u128) -> Crux {
        let n = crux.id().as_uuid().as_u128();
        redraw(&crux, |d| {
            d.recommended = Some(Recommended::new(
                option_id(n, k),
                Some(prose("callers already handle zero")),
                Some(Confidence::try_from(85).unwrap()),
            ));
        })
    }

    fn with_importance(crux: Crux, importance: Importance) -> Crux {
        redraw(&crux, |d| d.importance = Some(importance))
    }

    /// `crux` raised again with `change` made to what it was raised with.
    fn redraw(crux: &Crux, change: impl FnOnce(&mut CruxDraft)) -> Crux {
        let mut draft = CruxDraft {
            raised_in: crux.raised_in(),
            findings: crux.findings().to_vec(),
            kind: crux.kind(),
            question: crux.question().clone(),
            options: crux.options().to_vec(),
            recommended: crux.recommended().cloned(),
            importance: crux.importance(),
            supersedes: crux.supersedes(),
        };
        change(&mut draft);
        Crux::raise(crux.id(), draft).unwrap()
    }

    /// The other kinds of decision, on an arbitrary basis by a person, each in a request of its own and each on a
    /// Crux that supersedes the Crux of the change request `pop of empty` (the second of `change_requests`): the
    /// requests, and then the Cruxes.
    pub(crate) fn other_decisions(p: &Project) -> (Vec<ChangeRequest>, Vec<Crux>) {
        let open = |n: u128, title: &str| {
            ChangeRequest::open(
                ChangeRequestId::from_uuid(id(n)),
                ChangeRequestDraft {
                    title: Title::try_from(title).unwrap(),
                    situation: None,
                    intent: Intent::ChangeBehavior,
                    parent: None,
                    issuer: None,
                    expects: expectations_now(
                        p,
                        [(Target::Item(ItemId::from_uuid(id(31))), ExpectKind::Target)],
                    ),
                    assumptions: Vec::new(),
                    addresses: Vec::new(),
                },
            )
        };
        let superseding = |n: u128, raised_in: &ChangeRequest, kind: CruxKind| {
            let q = crux(
                n,
                raised_in.id(),
                kind,
                "which one?",
                vec![("a", None, BTreeMap::new(), None)],
            );
            redraw(&q, |d| {
                d.importance = Some(Importance::Low);
                d.supersedes = Some(CruxId::from_uuid(id(62)));
            })
        };
        let arbitrary = |reason: &str| Basis::Arbitrary {
            reason: prose(reason),
        };
        let alice = || DecisionMaker::person("alice").unwrap();
        let mut crs = Vec::new();
        let mut cruxes = Vec::new();
        for (n, title, kind, decision, basis, at) in [
            (
                70,
                "worded",
                CruxKind::Conflict,
                Decision::Worded {
                    wording: prose("in my words"),
                    reason: None,
                },
                arbitrary("the product leans this way"),
                1_700_000_000_456,
            ),
            (
                71,
                "free",
                CruxKind::Disagreement,
                Decision::Unspecified {
                    keep: vec![prose("it terminates")],
                },
                arbitrary("either is fine here"),
                1_700_000_000_789,
            ),
            (
                72,
                "withdrawn",
                CruxKind::Model,
                Decision::Withdrawn,
                arbitrary("the word is settled elsewhere"),
                1_700_000_000_999,
            ),
        ] {
            let mut cr = open(n, title);
            let mut q = superseding(n, &cr, kind);
            decide(
                &mut q,
                &mut cr,
                answer(n, decision, basis, alice(), at),
                p,
                None,
                &[],
            )
            .unwrap();
            crs.push(cr);
            cruxes.push(q);
        }
        (crs, cruxes)
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    #[test]
    fn a_project_comes_back_as_it_was_written() {
        let p = project();
        let text = encode_project(&p).unwrap();
        assert_eq!(decode_project(CURRENT, &text).unwrap(), p);
    }

    #[test]
    fn what_a_project_retired_comes_back_and_a_retired_id_that_is_live_is_corrupt() {
        use crate::domain::vo::id::{ContextId, HarnessId, InterfaceId};
        let mut p = project();
        let top = ContextId::from_uuid(id(11));
        p.retire_harness(top, HarnessId::from_uuid(id(40))).unwrap();
        p.retire_interface(top, InterfaceId::from_uuid(id(IFACE)))
            .unwrap();
        // a Harness with its Case, then an Interface with its two Items
        assert_eq!(p.retired().len(), 2 + 3);
        let text = encode_project(&p).unwrap();
        assert_eq!(decode_project(CURRENT, &text).unwrap(), p);

        // a retired id that a live thing has
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["retired"][0]["id"] = serde_json::json!(id(20).to_string());
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("retired")
        ));
        // a Case retired without the Harness that held it
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["retired"][1]["holder"] = serde_json::Value::Null;
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(_))
        ));
        // a document without the record of what was retired does not load
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc.as_object_mut().unwrap().remove("retired");
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("retired")
        ));
    }

    #[test]
    fn the_kind_of_a_harness_comes_back_and_a_document_without_one_is_a_harness_of_tests() {
        use std::collections::{BTreeMap, BTreeSet};

        use crate::domain::vo::case::CaseCites;
        use crate::domain::vo::case::CaseName;
        use crate::domain::vo::harness_kind::HarnessKind;
        use crate::domain::vo::id::{HarnessId, InterfaceId};
        use crate::domain::vo::source::Source;
        let mut p = project();
        let text = encode_project(&p).unwrap();
        // tests are the default and are not written, so a document of before kinds is the same document
        assert!(!text.contains("\"kind\""), "{text}");
        let harness = HarnessId::from_uuid(id(40));
        let top = p
            .contexts()
            .find(|c| c.harness(harness).is_some())
            .map(|c| c.id())
            .unwrap();
        // a Case of a sim cites Clauses and no Item, and is named `sweep`
        let given = || {
            let c = p.context(top).and_then(|c| c.harness(harness)).unwrap();
            c.cases()
                .iter()
                .map(|k| {
                    (
                        k.id(),
                        CaseName::try_from("sweep").unwrap(),
                        k.lines(),
                        CaseCites::of(BTreeMap::new(), k.cites().clauses().clone()),
                    )
                })
                .collect::<Vec<_>>()
        };
        let cases = given();
        let iface = InterfaceId::from_uuid(id(IFACE));
        p.put_harness_as(
            top,
            harness,
            Some(HarnessKind::Sim),
            BTreeSet::from([iface]),
            Source::new("#[test]\nfn push_then_pop() {}\n".to_string()),
            cases,
        )
        .unwrap();
        let text = encode_project(&p).unwrap();
        assert!(text.contains("\"kind\":\"sim\""), "{text}");
        // what it builds is written, by id, and comes back
        assert!(
            text.contains(&format!("\"builds\":[\"{}\"]", iface.as_uuid())),
            "{text}"
        );
        let back = decode_project(CURRENT, &text).unwrap();
        assert_eq!(back, p);
        let back_harness = back.context(top).and_then(|c| c.harness(harness)).unwrap();
        assert_eq!(back_harness.kind(), HarnessKind::Sim);
        assert_eq!(back_harness.builds(), &BTreeSet::from([iface]));
        // a kind nobody knows is corrupt, not quietly tests
        let odd = text.replace("\"kind\":\"sim\"", "\"kind\":\"fuzz\"");
        assert!(decode_project(CURRENT, &odd).is_err());
    }

    #[test]
    fn a_pin_comes_back_as_it_was_written() {
        let pin = pin(&project());
        let text = encode_pin(&pin).unwrap();
        assert_eq!(decode_pin(CURRENT, &text).unwrap(), pin);
    }

    #[test]
    fn change_requests_and_cruxes_come_back_as_they_were_written() {
        let p = project();
        let (others, other_cruxes) = other_decisions(&p);
        for cr in change_requests(&p).into_iter().chain(others) {
            let text = encode_change_request(&cr).unwrap();
            assert_eq!(
                decode_change_request(CURRENT_CHANGE_REQUEST, &text).unwrap(),
                cr
            );
        }
        let all = cruxes(&p);
        // Open and answered; chosen, worded, unspecified and withdrawn.
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].state().word(), "answered");
        assert_eq!(all[1].state().word(), "open");
        for q in all.into_iter().chain(other_cruxes) {
            let text = encode_crux(&q).unwrap();
            assert_eq!(decode_crux(CURRENT_CRUX, &text).unwrap(), q);
        }
    }

    #[test]
    fn a_crux_of_schema_1_is_read_as_it_is_and_an_answer_writes_its_beyond_only_when_it_has_one() {
        use crate::domain::vo::answer::Beyond;
        use crate::domain::vo::id::ChangeRequestId;
        use crate::domain::vo::prose::Prose;

        let p = project();
        let answered = cruxes(&p).remove(0);
        let text = encode_crux(&answered).unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["schema"], CURRENT_CRUX);
        // An answer that says nothing of what lies outside writes no `beyond`.
        assert!(doc["state"]["answered"].get("beyond").is_none());
        // The document as schema 1 wrote it is the same without the number: it reads, and its record says 1.
        doc["schema"] = serde_json::json!(1);
        assert_eq!(decode_crux(1, &doc.to_string()).unwrap(), answered);
        assert!(decode_crux(CURRENT_CRUX, &doc.to_string()).is_err());

        // Both forms of `beyond` come back as they were written.
        let request = ChangeRequestId::from_uuid(id(1));
        for (written, read) in [
            (
                serde_json::json!({"requests": [id(1).to_string(), id(2).to_string()]}),
                Beyond::taken_by(vec![request, ChangeRequestId::from_uuid(id(2))]).unwrap(),
            ),
            (
                serde_json::json!({"declined": "later"}),
                Beyond::declined(Prose::try_from("later").unwrap()),
            ),
        ] {
            let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
            doc["state"]["answered"]["beyond"] = written;
            let q = decode_crux(CURRENT_CRUX, &doc.to_string()).unwrap();
            assert_eq!(q.answer_given().and_then(|a| a.beyond()), Some(&read));
            let back: serde_json::Value = serde_json::from_str(&encode_crux(&q).unwrap()).unwrap();
            assert_eq!(back, doc);
        }
        // What the domain refuses is corrupt: none, one twice, an id that is none, a blank reason, a form unknown.
        for odd in [
            serde_json::json!({"requests": []}),
            serde_json::json!({"requests": [id(1).to_string(), id(1).to_string()]}),
            serde_json::json!({"requests": ["not an id"]}),
            serde_json::json!({"declined": "  "}),
            serde_json::json!({"neither": true}),
        ] {
            let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
            doc["state"]["answered"]["beyond"] = odd;
            assert!(
                matches!(
                    decode_crux(CURRENT_CRUX, &doc.to_string()),
                    Err(StoreError::Corrupt(_))
                ),
                "{doc}"
            );
        }
    }

    #[test]
    fn the_work_of_a_change_request_comes_back_with_what_moved_and_where_it_was() {
        use crate::domain::vo::footprint::{Footprint, Move};
        use crate::domain::vo::id::{ContextId, ContractId};
        use crate::domain::vo::target::Target;

        let p = project();
        let mut cr = change_requests(&p)
            .into_iter()
            .find(|c| !c.is_resolved())
            .unwrap();
        let context = Target::Context(ContextId::from_uuid(id(1)));
        let contract = Target::Contract(ContractId::from_uuid(id(2)));
        cr.record_work(Footprint::new(
            7,
            vec![
                Move::new(Target::Project, Vec::new()),
                Move::new(contract, vec![Target::Project, context]),
            ],
        ))
        .unwrap();
        cr.record_work(Footprint::new(
            9,
            vec![Move::new(context, vec![Target::Project])],
        ))
        .unwrap();
        assert_eq!(cr.work().len(), 2);
        let text = encode_change_request(&cr).unwrap();
        let read = decode_change_request(CURRENT_CHANGE_REQUEST, &text).unwrap();
        assert_eq!(read.work(), cr.work());
        assert_eq!(read, cr);
        // a document without it is of before it existed and does not load
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc.as_object_mut().unwrap().remove("work");
        assert!(decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).is_err());
    }

    #[test]
    fn a_moved_case_comes_back_with_what_it_cited_and_a_footprint_without_it_cites_nothing() {
        use std::collections::BTreeMap;

        use crate::domain::vo::footprint::{Citations, Footprint, Move};
        use crate::domain::vo::id::{CaseId, ClauseId, ContextId, ContractId, ItemId};
        use crate::domain::vo::target::Target;

        let p = project();
        let mut cr = change_requests(&p)
            .into_iter()
            .find(|c| !c.is_resolved())
            .unwrap();
        let context = Target::Context(ContextId::from_uuid(id(1)));
        let contract = Target::Contract(ContractId::from_uuid(id(2)));
        let clause = Target::Clause(ClauseId::from_uuid(id(3)));
        let item = Target::Item(ItemId::from_uuid(id(4)));
        let case_id = CaseId::from_uuid(id(5));
        let case = Target::Case(case_id);
        let within = vec![Target::Project, context];
        cr.record_work(
            Footprint::new(
                7,
                vec![
                    Move::new(contract, within.clone()),
                    Move::new(case, within.clone()),
                ],
            )
            .with_citations(BTreeMap::from([(
                case_id,
                Citations::new(vec![clause, item], vec![Target::Project, context, contract]),
            )])),
        )
        .unwrap();
        let text = encode_change_request(&cr).unwrap();
        let read = decode_change_request(CURRENT_CHANGE_REQUEST, &text).unwrap();
        assert_eq!(read.work(), cr.work());
        let footprint = &read.work()[0];
        assert_eq!(footprint.citations()[&case_id].cites(), [clause, item]);
        assert!(footprint.cites_inside(case, contract));
        assert!(!footprint.cites_inside(contract, contract));
        // a footprint stored before citations were recorded has no such key, and is read as citing nothing
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        for footprint in doc["work"].as_array_mut().unwrap() {
            assert!(
                footprint
                    .as_object_mut()
                    .unwrap()
                    .remove("citations")
                    .is_some()
            );
        }
        let old = decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).unwrap();
        let footprint = &old.work()[0];
        assert_eq!(footprint.moves()[1].target(), case);
        assert!(footprint.citations().is_empty());
        assert!(!footprint.cites_inside(case, clause));
        assert!(footprint.moves()[1].is_inside(context));
    }

    #[test]
    fn a_new_name_comes_back_as_it_was_kept_and_a_move_without_one_is_a_revision_move() {
        use crate::domain::vo::clause::ClauseLabel;
        use crate::domain::vo::footprint::{Change, Footprint, Move, Rename};
        use crate::domain::vo::id::{ClauseId, ContextId, ContractId, InterfaceId, ItemId};
        use crate::domain::vo::item_key::ItemKey;
        use crate::domain::vo::target::Target;

        let p = project();
        let mut cr = change_requests(&p)
            .into_iter()
            .find(|c| !c.is_resolved())
            .unwrap();
        let context = Target::Context(ContextId::from_uuid(id(1)));
        let contract = Target::Contract(ContractId::from_uuid(id(2)));
        let interface = Target::Interface(InterfaceId::from_uuid(id(6)));
        let label = |s: &str| ClauseLabel::try_from(s).unwrap();
        let key = |s: &str| ItemKey::try_from(s).unwrap();
        let in_contract = vec![Target::Project, context, contract];
        let in_interface = vec![Target::Project, context, interface];
        let moves = vec![
            Move::relabeled(
                ClauseId::from_uuid(id(3)),
                in_contract.clone(),
                label("C1"),
                label("C2"),
                false,
            ),
            Move::relabeled(
                ClauseId::from_uuid(id(7)),
                in_contract.clone(),
                label("C3"),
                label("C4"),
                true,
            ),
            Move::rekeyed(
                ItemId::from_uuid(id(4)),
                in_interface,
                key("fn a"),
                key("fn b"),
                false,
            ),
            Move::new(contract, vec![Target::Project, context]),
        ];
        cr.record_work(Footprint::new(7, moves.clone())).unwrap();
        let text = encode_change_request(&cr).unwrap();
        let read = decode_change_request(CURRENT_CHANGE_REQUEST, &text).unwrap();
        assert_eq!(read.work(), cr.work());
        assert_eq!(read.work()[0].moves(), moves);
        assert_eq!(read.work()[0].renames().count(), 3);
        assert!(matches!(
            read.work()[0].moves()[1].change(),
            Change::RevisedAndRenamed(Rename::Label { .. })
        ));
        // a move kept before names were recorded has no `renamed`, and is a revision move
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        for m in doc["work"][0]["moves"].as_array_mut().unwrap() {
            m.as_object_mut().unwrap().remove("renamed");
        }
        let old = decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).unwrap();
        assert!(
            old.work()[0]
                .moves()
                .iter()
                .all(|m| m.change() == &Change::Revised)
        );
        // a move written without the `renamed` key at all, as it was kept before
        let literal = serde_json::json!({
            "target": "clause",
            "id": id(3).to_string(),
            "within": [],
        });
        assert!(literal.get("renamed").is_none());
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["work"][0]["moves"] = serde_json::json!([literal]);
        let old = decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).unwrap();
        assert_eq!(old.work()[0].moves()[0].change(), &Change::Revised);
        // a name on a target that has none, or one that cannot be read, is refused
        for (target, from, to) in [("contract", "C1", "C2"), ("clause", "one", "C2")] {
            let target_id = if target == "clause" { id(3) } else { id(2) };
            let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
            doc["work"][0]["moves"] = serde_json::json!([{
                "target": target,
                "id": target_id.to_string(),
                "within": [],
                "renamed": { "from": from, "to": to },
            }]);
            assert!(
                decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).is_err(),
                "{doc}"
            );
        }
    }

    #[test]
    fn an_answer_comes_back_with_its_id_its_basis_its_maker_and_its_time() {
        use crate::domain::vo::answer::Basis;
        use crate::domain::vo::crux::Importance;
        let p = project();
        let q = &cruxes(&p)[0];
        let text = encode_crux(q).unwrap();
        let read = decode_crux(CURRENT_CRUX, &text).unwrap();
        let answer = read.answer_given().unwrap();
        assert_eq!(answer.id().as_uuid(), id(8062));
        assert!(matches!(
            answer.basis(),
            Basis::Policy { line, policy_revision }
                if line.to_string() == "P1" && policy_revision.number() == p.policy().revision().number()
        ));
        assert_eq!(answer.by().to_string(), "role orchestrator");
        assert_eq!(answer.at().millis(), 1_700_000_000_123);
        assert_eq!(
            read.recommended()
                .and_then(|r| r.confidence())
                .map(|c| c.percent()),
            Some(85)
        );
        assert_eq!(read.importance(), Some(Importance::High));
        assert_eq!(read.options().len(), 2);
        assert_eq!(read.options()[1].id(), option_id(62, 1));
        assert_eq!(
            read.recommended().map(|r| r.option()),
            Some(option_id(62, 1))
        );
        // What the decision asked is in the change request, with the revisions of the targets then.
        let second = &change_requests(&p)[1];
        assert_eq!(second.decided_expects().len(), 1);
        let (others, other_cruxes) = other_decisions(&p);
        assert!(others.iter().all(|c| c.decided_expects().is_empty()));
        for superseding in &other_cruxes {
            let text = encode_crux(superseding).unwrap();
            let read = decode_crux(CURRENT_CRUX, &text).unwrap();
            assert_eq!(read.supersedes(), Some(q.id()));
        }
        // The open Crux keeps the findings it was raised from.
        let open = &cruxes(&p)[1];
        let read = decode_crux(CURRENT_CRUX, &encode_crux(open).unwrap()).unwrap();
        assert_eq!(read.findings().len(), 2);
        assert!(read.is_open());
        assert_eq!(read.raised_in(), change_requests(&p)[0].id());
        // The assumptions are the change request's.
        assert_eq!(second.assumptions().len(), 1);
        assert_eq!(second.assumptions()[0].evidence().len(), 3);
    }

    #[test]
    fn the_policy_comes_back_with_its_revision_and_the_revision_of_each_line() {
        let p = project();
        let text = encode_project(&p).unwrap();
        let read = decode_project(CURRENT, &text).unwrap();
        assert_eq!(read.policy(), p.policy());
        let lines = read.policy().lines();
        assert_eq!(read.policy().revision().number(), 3);
        assert_eq!(lines[0].changed_at().number(), 2);
        assert_eq!(lines[1].changed_at().number(), 3);
    }

    #[test]
    fn a_document_of_an_older_layout_is_refused_and_says_what_is_wrong() {
        let p = project();
        // Without the Assumptions, or the Policy, a document does not load: there is no migration of those.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc.as_object_mut().unwrap().remove("assumptions");
        assert!(matches!(
            decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("assumptions")
        ));
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_project(&p).unwrap()).unwrap();
        doc.as_object_mut().unwrap().remove("policy");
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("policy")
        ));
        // A change request that still holds a Crux, and one that still has a `decision`, are refused by name.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc["crux"] = serde_json::json!(null);
        assert!(matches!(
            decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("crux")
        ));
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc["decision"] = serde_json::json!(null);
        assert!(matches!(
            decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("decision")
        ));
        // A request without `addresses` does not load either.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[0]).unwrap()).unwrap();
        doc.as_object_mut().unwrap().remove("addresses");
        assert!(matches!(
            decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("addresses")
        ));
        // A Crux that still names its options by text, or lacks its question, does not load.
        let crux = encode_crux(&cruxes(&p)[0]).unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&crux).unwrap();
        doc.as_object_mut().unwrap().remove("question");
        assert!(matches!(
            decode_crux(CURRENT_CRUX, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("question")
        ));
        let mut doc: serde_json::Value = serde_json::from_str(&crux).unwrap();
        doc["recommended"] =
            serde_json::json!({ "choice": "zero", "reason": null, "confidence": null });
        assert!(matches!(
            decode_crux(CURRENT_CRUX, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("choice")
        ));
    }

    #[test]
    fn an_invariant_the_domain_holds_is_checked_on_restore() {
        let p = project();
        let asked = encode_crux(&cruxes(&p)[0]).unwrap();
        let corrupt = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut doc: serde_json::Value = serde_json::from_str(&asked).unwrap();
            edit(&mut doc);
            decode_crux(CURRENT_CRUX, &doc.to_string())
        };
        let stray = serde_json::json!(id(9999).to_string());
        // A confidence above 100, a blank maker, an answer to an option the Crux does not have, a bad line id, a
        // recommendation of an option it does not have, no options at all.
        assert!(matches!(
            corrupt(&|d| d["recommended"]["confidence"] = serde_json::json!(101)),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["state"]["answered"]["by"] = serde_json::json!({ "role": " " })),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["state"]["answered"]["decision"] =
                serde_json::json!({ "chosen": { "option": stray, "reason": null } })),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["state"]["answered"]["basis"] =
                serde_json::json!({ "policy": { "line": "P0", "policy_revision": 3 } })),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["recommended"]["option"] = stray.clone()),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["options"] = serde_json::json!([])),
            Err(StoreError::Corrupt(_))
        ));
        // A model question that was left unspecified.
        assert!(matches!(
            corrupt(&|d| {
                d["kind"] = serde_json::json!("model");
                d["state"]["answered"]["decision"] =
                    serde_json::json!({ "unspecified": { "keep": [] } });
            }),
            Err(StoreError::Corrupt(_))
        ));
        // Evidence that leaves the repository.
        let second = encode_change_request(&change_requests(&p)[1]).unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&second).unwrap();
        doc["assumptions"][0]["evidence"][0]["repo"]["path"] = serde_json::json!("../secret");
        assert!(matches!(
            decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()),
            Err(StoreError::Corrupt(_))
        ));
        // A Policy line changed after the Policy's own revision.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_project(&p).unwrap()).unwrap();
        doc["policy"]["lines"][0]["changed_at"] = serde_json::json!(9);
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn a_change_request_that_asks_for_more_keeps_what_it_asked_and_an_old_document_is_refused() {
        let p = project();
        let second = &change_requests(&p)[1];
        // `zero` asks for C1 (the request asks it already, as it does: not recorded again) and for the Item `pop`.
        assert_eq!(second.decided_expects().len(), 1);
        // A document of the layout before expectations names no `decided_expects`: it does not load, and says why.
        for old in ["decided_expects", "expects"] {
            let mut doc: serde_json::Value =
                serde_json::from_str(&encode_change_request(second).unwrap()).unwrap();
            doc.as_object_mut().unwrap().remove(old);
            let refused = decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string());
            assert!(
                matches!(&refused, Err(StoreError::Corrupt(why)) if why.contains(old)),
                "{old}: {refused:?}"
            );
        }
    }

    #[test]
    fn a_change_request_keeps_an_expectation_of_every_kind_of_target() {
        let p = project();
        let first = &change_requests(&p)[0];
        assert_eq!(first.expects().len(), 8);
        let text = encode_change_request(first).unwrap();
        assert_eq!(
            &decode_change_request(CURRENT_CHANGE_REQUEST, &text).unwrap(),
            first
        );
    }

    #[test]
    fn the_history_of_a_change_request_keeps_every_resolution_and_reopening_in_order() {
        let p = project();
        let first = &change_requests(&p)[0];
        let doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(first).unwrap()).unwrap();
        let steps = doc["history"].as_array().unwrap();
        assert_eq!(steps.len(), 3);
        assert!(steps[0]["resolved"]["kind"] == "done");
        assert!(steps[1].get("reopened").is_some());
        assert!(steps[2]["resolved"]["kind"]["superseded"]["by"].is_string());
        let read = decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).unwrap();
        assert_eq!(read.history(), first.history());
        assert_eq!(
            read.resolution().map(|r| r.reason().as_str()),
            Some("the other one carries it")
        );
        // A document of the layout before resolutions had a history (`resolved`) does not load.
        let mut old = doc.clone();
        old.as_object_mut().unwrap().remove("history");
        old["resolved"] = serde_json::json!(true);
        for why in ["history", "resolved"] {
            assert!(matches!(
                decode_change_request(CURRENT_CHANGE_REQUEST, &old.to_string()),
                Err(StoreError::Corrupt(m)) if m.contains(why)
            ));
        }
        // Steps out of order are refused by the domain.
        let mut swapped = doc.clone();
        swapped["history"].as_array_mut().unwrap().swap(0, 1);
        assert!(matches!(
            decode_change_request(CURRENT_CHANGE_REQUEST, &swapped.to_string()),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn a_target_expected_twice_is_corrupt() {
        let p = project();
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[0]).unwrap()).unwrap();
        let again = doc["expects"][0].clone();
        doc["expects"].as_array_mut().unwrap().push(again);
        assert!(matches!(
            decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("twice")
        ));
    }

    #[test]
    fn a_decided_expectation_the_decision_does_not_ask_is_corrupt() {
        use crate::domain::service::change_request::check_requests_and_cruxes;
        let p = project();
        let (crs, mut all) = (change_requests(&p), cruxes(&p));
        assert_eq!(check_requests_and_cruxes(&crs, &all), Ok(()));
        // The decision now chooses the option that asks nothing: what is recorded for it is unasked.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_crux(&all[0]).unwrap()).unwrap();
        doc["state"]["answered"]["decision"] = serde_json::json!({ "chosen": { "option": option_id(62, 0).to_string(), "reason": null } });
        all[0] = decode_crux(CURRENT_CRUX, &doc.to_string()).unwrap();
        assert!(check_requests_and_cruxes(&crs, &all).is_err());
    }

    #[test]
    fn a_request_resolved_as_done_with_a_crux_open_is_refused_when_they_are_read_together() {
        use crate::domain::service::change_request::check_requests_and_cruxes;
        let p = project();
        let crs = change_requests(&p);
        let mut all = cruxes(&p);
        // The open Crux is raised in a request that was resolved as superseded: a record of what was.
        assert_eq!(check_requests_and_cruxes(&crs, &all), Ok(()));
        // Raised in the one decided, it is the same; but a request resolved as done is not.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&crs[0]).unwrap()).unwrap();
        doc["history"] = serde_json::json!([{ "resolved": {
            "kind": "done", "reason": "r", "by": { "person": "alice" }, "at": 1 } }]);
        let done = decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).unwrap();
        assert!(check_requests_and_cruxes(&[done, crs[1].clone()], &all).is_err());
        // A Crux raised in a request there is not.
        all[1] = decode_crux(
            CURRENT_CRUX,
            &encode_crux(&all[1])
                .unwrap()
                .replace(&id(60).to_string(), &id(9998).to_string()),
        )
        .unwrap();
        assert!(check_requests_and_cruxes(&crs, &all).is_err());
    }

    #[test]
    fn a_project_stored_without_the_revisions_of_the_project_and_its_contexts_is_refused() {
        let text = encode_project(&project()).unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["revision"], 3);
        doc.as_object_mut().unwrap().remove("revision");
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("revision")
        ));
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["contexts"][0]
            .as_object_mut()
            .unwrap()
            .remove("revision");
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("revision")
        ));
    }

    /// The document of `project()` as schema 1 wrote it, with the code of the Interface `IFACE` and its Items (id,
    /// revision, the signature that was kept, the line it is on) replaced by those given: an Item has its signature
    /// and no key.
    fn schema_1(source: &str, items: &[(u128, u64, &str, usize)]) -> serde_json::Value {
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_project(&project()).unwrap()).unwrap();
        doc["schema"] = serde_json::json!(1);
        let top = doc["contexts"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|c| c["name"] == "top")
            .unwrap();
        let interface = &mut top["interfaces"][0];
        interface["source"] = serde_json::json!(source);
        interface["items"] = serde_json::json!(
            items
                .iter()
                .map(|(n, revision, signature, line)| serde_json::json!({
                    "id": id(*n).to_string(),
                    "revision": revision,
                    "signature": signature,
                    "first": line,
                    "last": line,
                }))
                .collect::<Vec<_>>()
        );
        doc
    }

    /// The Interface `IFACE` of a Project: its revision, and the key and revision of each Item.
    fn interface_of(p: &Project) -> (u64, Vec<(String, u64)>) {
        use crate::domain::vo::id::{ContextId, InterfaceId};
        let i = p
            .context(ContextId::from_uuid(id(11)))
            .and_then(|c| c.interface(InterfaceId::from_uuid(id(IFACE))))
            .unwrap();
        (
            i.revision().number(),
            i.items()
                .iter()
                .map(|it| (it.key().to_string(), it.revision().number()))
                .collect(),
        )
    }

    fn owned(keys: &[(&str, u64)]) -> Vec<(String, u64)> {
        keys.iter().map(|(k, r)| (k.to_string(), *r)).collect()
    }

    #[test]
    fn a_document_of_schema_2_keeps_the_key_and_not_the_signature() {
        let text = encode_project(&project()).unwrap();
        assert!(text.contains("\"schema\":2"), "{text}");
        assert!(text.contains("\"key\":\"push\""), "{text}");
        assert!(!text.contains("signature"), "{text}");
        let back = decode_project(2, &text).unwrap();
        assert_eq!(back, project());
        assert_eq!(
            back.contexts()
                .flat_map(|c| c.interfaces())
                .flat_map(|i| i.items())
                .map(|it| it.signature().to_string())
                .collect::<Vec<_>>(),
            ["pub fn push(v: u8)", "pub fn pop() -> u8"]
        );
    }

    #[test]
    fn a_document_of_schema_2_without_a_key_or_with_a_signature_is_corrupt() {
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_project(&project()).unwrap()).unwrap();
        let item = |doc: &mut serde_json::Value| -> serde_json::Value {
            doc["contexts"][1]["interfaces"][0]["items"][0].take()
        };
        let mut lost = item(&mut doc);
        lost.as_object_mut().unwrap().remove("key");
        doc["contexts"][1]["interfaces"][0]["items"][0] = lost;
        assert!(matches!(
            decode_project(2, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("no key")
        ));
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_project(&project()).unwrap()).unwrap();
        doc["contexts"][1]["interfaces"][0]["items"][0]["signature"] = serde_json::json!("fn x()");
        assert!(matches!(
            decode_project(2, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("signature")
        ));
    }

    #[test]
    fn a_pin_of_schema_1_is_read_as_it_is_and_a_change_request_of_schema_1_or_2_is_for_the_migration()
     {
        let p = project();
        let pin = pin(&p);
        let text = encode_pin(&pin)
            .unwrap()
            .replace("\"schema\":2", "\"schema\":1");
        assert_eq!(decode_pin(1, &text).unwrap(), pin);
        for cr in change_requests(&p) {
            let text = encode_change_request(&cr).unwrap();
            assert!(text.contains("\"schema\":4"), "{text}");
            for old in [1, 2] {
                let text = text.replace("\"schema\":4", &format!("\"schema\":{old}"));
                assert!(matches!(
                    decode_change_request(old, &text),
                    Err(StoreError::Corrupt(why)) if why.contains("migration")
                ));
            }
        }
    }

    #[test]
    fn a_project_of_schema_1_names_its_items_by_their_signatures_on_one_line_and_moves_no_revision()
    {
        let code = "pub fn push(v: u8)\npub fn pop(\n    ) -> u8\n";
        let doc = schema_1(
            code,
            &[
                (31, 3, "pub fn push(v: u8)", 1),
                (32, 2, "pub fn pop(\n    ) -> u8", 2),
            ],
        );
        let p = decode_project(1, &doc.to_string()).unwrap();
        // the key is each line without the white space at its ends, joined by one space
        assert_eq!(
            interface_of(&p).1,
            owned(&[("pub fn push(v: u8)", 3), ("pub fn pop( ) -> u8", 2)])
        );
        // nothing moved: the Interface keeps its revision (the fixture's, 1), and so does the Project
        assert_eq!(interface_of(&p).0, 1);
        assert_eq!(p.revision().number(), project().revision().number());
        // the signature kept is dropped: the Interface derives it from its code and lines
        let derived: Vec<String> = p
            .contexts()
            .flat_map(|c| c.interfaces())
            .flat_map(|i| i.items())
            .map(|it| it.signature().to_string())
            .collect();
        assert_eq!(derived, ["pub fn push(v: u8)", "pub fn pop("]);
        // it is written as schema 2 next, and reads back as the same Project
        let text = encode_project(&p).unwrap();
        assert!(
            text.contains("\"schema\":2") && !text.contains("signature"),
            "{text}"
        );
        assert_eq!(decode_project(2, &text).unwrap(), p);
    }

    #[test]
    fn a_signature_twice_in_an_interface_is_numbered_from_the_second_in_the_order_of_the_items() {
        let code = "fn a()\nfn a()\nfn a()\nfn b()\n";
        let doc = schema_1(
            code,
            &[
                (31, 1, "fn a()", 1),
                (32, 1, "fn a()", 2),
                (33, 1, "fn a()", 3),
                (34, 1, "fn b()", 4),
            ],
        );
        let p = decode_project(1, &doc.to_string()).unwrap();
        assert_eq!(
            interface_of(&p).1,
            owned(&[
                ("fn a()", 1),
                ("fn a() #2", 1),
                ("fn a() #3", 1),
                ("fn b()", 1)
            ])
        );
        // white space around the lines does not make two signatures two
        let doc = schema_1(
            "fn a()\n  fn a()  \n",
            &[(31, 1, "fn a()", 1), (32, 1, "\n  fn a()  \n", 2)],
        );
        let p = decode_project(1, &doc.to_string()).unwrap();
        assert_eq!(
            interface_of(&p).1,
            owned(&[("fn a()", 1), ("fn a() #2", 1)])
        );
    }

    #[test]
    fn a_number_is_moved_on_until_it_is_no_other_items_key() {
        // the third signature is the key the second would get, so the second goes on to `#3`
        let code = "fn a()\nfn a()\nfn a() #2\n";
        let doc = schema_1(
            code,
            &[
                (31, 1, "fn a()", 1),
                (32, 1, "fn a()", 2),
                (33, 1, "fn a() #2", 3),
            ],
        );
        let p = decode_project(1, &doc.to_string()).unwrap();
        assert_eq!(
            interface_of(&p).1,
            owned(&[("fn a()", 1), ("fn a() #3", 1), ("fn a() #2", 1)])
        );
        // the same when the signature that ends in `#2` comes first, and when the numbers pile up
        let code = "fn a() #2\nfn a()\nfn a()\nfn a()\n";
        let doc = schema_1(
            code,
            &[
                (31, 1, "fn a() #2", 1),
                (32, 1, "fn a()", 2),
                (33, 1, "fn a()", 3),
                (34, 1, "fn a()", 4),
            ],
        );
        let p = decode_project(1, &doc.to_string()).unwrap();
        assert_eq!(
            interface_of(&p).1,
            owned(&[
                ("fn a() #2", 1),
                ("fn a()", 1),
                ("fn a() #3", 1),
                ("fn a() #4", 1)
            ])
        );
    }

    #[test]
    fn the_first_put_after_the_conversion_changes_no_revision_when_the_content_is_the_same() {
        use crate::domain::vo::id::{ContextId, InterfaceId, ItemId};
        use crate::domain::vo::item_key::ItemKey;
        use crate::domain::vo::line_range::LineRange;
        use crate::domain::vo::source::Source;
        let code = "pub fn push(v: u8)\npub fn pop(\n    ) -> u8\n";
        let doc = schema_1(
            code,
            &[
                (31, 3, "pub fn push(v: u8)", 1),
                (32, 2, "pub fn pop(\n    ) -> u8", 2),
            ],
        );
        let mut p = decode_project(1, &doc.to_string()).unwrap();
        let before = p.clone();
        // the put names the Items by their keys, as `status` shows them
        let put = |p: &mut Project, code: &str| {
            p.put_interface(
                ContextId::from_uuid(id(11)),
                InterfaceId::from_uuid(id(IFACE)),
                Source::new(code.to_string()),
                vec![
                    (
                        ItemId::from_uuid(id(31)),
                        ItemKey::try_from("pub fn push(v: u8)").unwrap(),
                        LineRange::new(1, 1).unwrap(),
                    ),
                    (
                        ItemId::from_uuid(id(32)),
                        ItemKey::try_from("pub fn pop( ) -> u8").unwrap(),
                        LineRange::new(2, 2).unwrap(),
                    ),
                ],
            )
            .unwrap();
        };
        put(&mut p, code);
        assert_eq!(p, before);
        // a change of one declaration moves that Item and the Interface, and no other Item
        put(&mut p, "pub fn push(v: u16)\npub fn pop(\n    ) -> u8\n");
        assert_eq!(
            interface_of(&p),
            (
                2,
                owned(&[("pub fn push(v: u8)", 4), ("pub fn pop( ) -> u8", 2)])
            )
        );
    }

    #[test]
    fn what_a_project_of_schema_1_retired_is_named_by_the_same_rule() {
        use crate::domain::vo::id::ItemId;
        use crate::domain::vo::target::Target;
        let mut doc = schema_1("fn a()\n", &[(31, 1, "fn a()", 1)]);
        let retired = doc["retired"].as_array_mut().unwrap();
        retired.push(serde_json::json!({
            "target": "interface", "id": id(91).to_string(), "context": "top",
            "holder": null, "label": null, "last_revision": 1, "at": 2,
        }));
        retired.push(serde_json::json!({
            "target": "item", "id": id(92).to_string(), "context": "top",
            "holder": id(91).to_string(), "label": "  fn old(\n      x: u8)  \n",
            "last_revision": 1, "at": 2,
        }));
        let p = decode_project(1, &doc.to_string()).unwrap();
        let record = p
            .retired_of(Target::Item(ItemId::from_uuid(id(92))))
            .unwrap();
        assert_eq!(record.label(), Some("fn old( x: u8)"));
        assert_eq!(record.last_revision().number(), 1);
        // a document of schema 2 names it as it was written
        let mut now: serde_json::Value =
            serde_json::from_str(&encode_project(&p).unwrap()).unwrap();
        assert_eq!(now["retired"][1]["label"], "fn old( x: u8)");
        now["retired"][1]["label"] = serde_json::json!("old  key");
        let kept = decode_project(2, &now.to_string()).unwrap();
        let record = kept
            .retired_of(Target::Item(ItemId::from_uuid(id(92))))
            .unwrap();
        assert_eq!(record.label(), Some("old  key"));
    }

    #[test]
    fn an_item_that_declares_nothing_in_a_project_of_schema_1_is_corrupt_naming_it() {
        // the second line is empty: the Item's lines hold no declaration, so there is no signature to derive
        let doc = schema_1(
            "fn a()\n\n",
            &[(31, 1, "fn a()", 1), (32, 1, "fn  b( )", 2)],
        );
        let named = match decode_project(1, &doc.to_string()) {
            Err(StoreError::Corrupt(why)) => why,
            other => panic!("{other:?}"),
        };
        assert!(named.contains(&id(IFACE).to_string()), "{named}");
        assert!(named.contains(&id(32).to_string()), "{named}");
        assert!(named.contains("`fn  b( )`"), "{named}");
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let text = encode_project(&project()).unwrap();
        assert!(matches!(
            decode_project(CURRENT + 1, &text),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn a_document_that_disagrees_with_its_record_is_refused() {
        let text = encode_project(&project())
            .unwrap()
            .replace("\"schema\":2", "\"schema\":0");
        assert!(matches!(
            decode_project(CURRENT, &text),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A correction of `fields`, by a person, with a reason, at a time.
    fn correction<F: PartialEq + std::fmt::Display>(
        fields: Vec<F>,
    ) -> crate::domain::vo::correction::Correction<F> {
        crate::domain::vo::correction::Correction::new(
            fields,
            crate::domain::vo::prose::Prose::try_from("a token was pasted").unwrap(),
            crate::domain::vo::answer::DecisionMaker::person("carol").unwrap(),
            crate::domain::vo::timestamp::Timestamp::from_millis(50),
        )
        .unwrap()
    }

    #[test]
    fn a_corrected_change_request_comes_back_as_it_was_written_and_its_step_holds_no_text() {
        use crate::domain::entity::change_request::ChangeRequestField as Field;
        use crate::domain::vo::correction::{Correctable, SoughtText};
        use crate::domain::vo::resolution::Transition;

        let p = project();
        let mut cr = change_requests(&p).remove(0);
        for (token, field) in [
            ("caller", Field::StepReason { step: 1 }),
            ("the other", Field::StepReason { step: 2 }),
        ] {
            cr.correct(
                &SoughtText::try_from(token).unwrap(),
                correction(vec![field]),
                &|_, _| false,
            )
            .unwrap();
        }
        assert_eq!(cr.history().len(), 5);
        let text = encode_change_request(&cr).unwrap();
        let back = decode_change_request(CURRENT_CHANGE_REQUEST, &text).unwrap();
        assert_eq!(back, cr);
        assert!(matches!(back.history()[4], Transition::Corrected(_)));
        // the step is written by the words of the fields, and the text is nowhere in the document
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            doc["history"][3],
            serde_json::json!({"corrected": {
                "fields": [{"step_reason": 1}],
                "reason": "a token was pasted",
                "by": {"person": "carol"},
                "at": 50
            }})
        );
        assert!(
            !text.contains("caller") && !text.contains("the other"),
            "{text}"
        );
    }

    #[test]
    fn a_correction_between_a_resolution_and_a_reopening_is_read_back_where_it_stands() {
        use crate::domain::vo::resolution::Transition;

        let p = project();
        let cr = change_requests(&p).remove(0);
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&cr).unwrap()).unwrap();
        let step = serde_json::json!({"corrected": {
            "fields": [
                "title",
                {"assumption_reason": 0},
                {"assumption_evidence": [0, 1]},
                {"step_by": 2}
            ],
            "reason": "a token was pasted",
            "by": {"role": "orchestrator"},
            "at": 77
        }});
        doc["history"].as_array_mut().unwrap().insert(1, step);
        let back = decode_change_request(CURRENT_CHANGE_REQUEST, &doc.to_string()).unwrap();
        assert_eq!(back.history().len(), 4);
        let Transition::Corrected(c) = &back.history()[1] else {
            panic!("the correction stands between the resolution and the reopening")
        };
        assert_eq!(c.fields().len(), 4);
        assert_eq!(c.by().to_string(), "role orchestrator");
        // it changes no state: the request is resolved by the last resolution, as it was
        assert!(back.is_resolved());
        let again: serde_json::Value =
            serde_json::from_str(&encode_change_request(&back).unwrap()).unwrap();
        assert_eq!(again, doc);
        // a correction that names no field, or a field twice, is corrupt
        for odd in [
            serde_json::json!([]),
            serde_json::json!(["title", "title"]),
            serde_json::json!(["no_such_field"]),
        ] {
            let mut bad = doc.clone();
            bad["history"][1]["corrected"]["fields"] = odd;
            assert!(matches!(
                decode_change_request(CURRENT_CHANGE_REQUEST, &bad.to_string()),
                Err(StoreError::Corrupt(_))
            ));
        }
    }

    #[test]
    fn a_change_request_of_schema_3_is_read_as_it_is_and_has_no_corrected_step() {
        let p = project();
        for cr in change_requests(&p) {
            let text = encode_change_request(&cr).unwrap();
            assert!(text.contains("\"schema\":4"), "{text}");
            let three = text.replace("\"schema\":4", "\"schema\":3");
            assert_eq!(decode_change_request(3, &three).unwrap(), cr);
            // the number of the record and the document's own must agree
            assert!(matches!(
                decode_change_request(4, &three),
                Err(StoreError::Corrupt(_))
            ));
            assert!(matches!(
                decode_change_request(5, &text),
                Err(StoreError::Corrupt(_))
            ));
        }
    }

    #[test]
    fn a_change_request_of_schema_3_with_a_corrected_step_is_corrupt() {
        let p = project();
        let cr = change_requests(&p).remove(0);
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&cr).unwrap()).unwrap();
        let step = serde_json::json!({"corrected": {
            "fields": ["title"],
            "reason": "a token was pasted",
            "by": {"person": "carol"},
            "at": 77
        }});
        doc["history"].as_array_mut().unwrap().push(step);
        // as schema 4 it is read, as schema 3 it is refused: schema 3 never had the step
        assert!(decode_change_request(4, &doc.to_string()).is_ok());
        doc["schema"] = serde_json::json!(3);
        assert!(matches!(
            decode_change_request(3, &doc.to_string()),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn a_corrected_crux_comes_back_as_it_was_written_and_a_crux_of_schema_2_is_read_as_it_is() {
        use crate::domain::entity::crux::CruxField as Field;
        use crate::domain::vo::correction::{Correctable, SoughtText};

        let p = project();
        let mut asked = cruxes(&p).remove(0);
        let plain = encode_crux(&asked).unwrap();
        // a Crux never corrected writes no `corrections`, so it is the document of schema 2 in all but the number
        let mut doc: serde_json::Value = serde_json::from_str(&plain).unwrap();
        assert!(doc.get("corrections").is_none());
        doc["schema"] = serde_json::json!(2);
        assert_eq!(decode_crux(2, &doc.to_string()).unwrap(), asked);
        assert!(decode_crux(CURRENT_CRUX, &doc.to_string()).is_err());

        let option = option_id(62, 0);
        asked
            .correct(
                &SoughtText::try_from("callers").unwrap(),
                correction(vec![
                    Field::OptionConsequence { option },
                    Field::RecommendationReason,
                ]),
                &|_, _| false,
            )
            .unwrap();
        let text = encode_crux(&asked).unwrap();
        assert_eq!(decode_crux(CURRENT_CRUX, &text).unwrap(), asked);
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            doc["corrections"],
            serde_json::json!([{
                "fields": [
                    {"option_consequence": option.as_uuid().to_string()},
                    "recommendation_reason"
                ],
                "reason": "a token was pasted",
                "by": {"person": "carol"},
                "at": 50
            }])
        );
        assert!(!text.contains("callers"), "{text}");
        // an option id that is not one is corrupt
        let mut odd = doc.clone();
        odd["corrections"][0]["fields"][0] = serde_json::json!({"option_choice": "not an id"});
        assert!(matches!(
            decode_crux(CURRENT_CRUX, &odd.to_string()),
            Err(StoreError::Corrupt(_))
        ));
        // schema 1 and 2 never had corrections: a document of either that has them is corrupt
        for older in [1, 2] {
            let mut old = doc.clone();
            old["schema"] = serde_json::json!(older);
            assert!(
                matches!(
                    decode_crux(older, &old.to_string()),
                    Err(StoreError::Corrupt(_))
                ),
                "schema {older}"
            );
        }
    }

    #[test]
    fn a_document_the_domain_refuses_is_corrupt() {
        // The Context `top` rests on its Contract: take the Contract away and the rule no longer holds.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_project(&project()).unwrap()).unwrap();
        for c in doc["contexts"].as_array_mut().unwrap() {
            c["contracts"] = serde_json::json!([]);
        }
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            decode_project(CURRENT, "not json"),
            Err(StoreError::Corrupt(_))
        ));
    }
}
