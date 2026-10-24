//! How the aggregates are written to the store and read back: one JSON document each, in a versioned schema
//! ([`v1`]). A document that is newer than this code, or that the domain's rules refuse, is `Corrupt`, never
//! quietly repaired.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::StoreError;

pub(crate) mod v1;

use self::v1::{ChangeRequestDoc, PinDoc, ProjectDoc, SCHEMA};

/// The schema number a document is written with.
pub(crate) const CURRENT: u32 = SCHEMA;

fn encode<D: Serialize>(doc: &D) -> Result<String, StoreError> {
    serde_json::to_string(doc)
        .map_err(|e| StoreError::Failed(format!("cannot write a document: {e}")))
}

/// Reads a document of `stored` schema (the column beside it); the document's own `schema` must agree.
fn decode<D, T>(stored: u32, text: &str, own: impl Fn(&D) -> u32) -> Result<T, StoreError>
where
    D: DeserializeOwned,
    T: TryFrom<D, Error = String>,
{
    if stored > CURRENT {
        return Err(StoreError::Corrupt(format!(
            "schema {stored} is newer than this version reads ({CURRENT})"
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
    decode::<ProjectDoc, Project>(stored, text, |d| d.schema)
}

pub(crate) fn encode_pin(p: &Pin) -> Result<String, StoreError> {
    encode(&PinDoc::from(p))
}

pub(crate) fn decode_pin(stored: u32, text: &str) -> Result<Pin, StoreError> {
    decode::<PinDoc, Pin>(stored, text, |d| d.schema)
}

pub(crate) fn encode_change_request(c: &ChangeRequest) -> Result<String, StoreError> {
    encode(&ChangeRequestDoc::from(c))
}

pub(crate) fn decode_change_request(stored: u32, text: &str) -> Result<ChangeRequest, StoreError> {
    decode::<ChangeRequestDoc, ChangeRequest>(stored, text, |d| d.schema)
}

#[cfg(test)]
pub(crate) mod fixture {
    //! One Project, Pin and change requests that use every part of the stored format.

    use std::collections::{BTreeMap, BTreeSet};

    use uuid::Uuid;

    use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft, Crux};
    use crate::domain::entity::context::Context;
    use crate::domain::entity::pin::Pin;
    use crate::domain::entity::project::Project;
    use crate::domain::service::change_request::{decide, revisions_now};
    use crate::domain::service::pin::record;
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Reasoned};
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
    use crate::domain::vo::id::{
        CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
        PinId, ProjectId,
    };
    use crate::domain::vo::intent::{Intent, Issuer};
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::signature::Signature;
    use crate::domain::vo::source::Source;
    use crate::domain::vo::target::Target;
    use crate::domain::vo::title::Title;

    pub(crate) fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    pub(crate) const IFACE: u128 = 30;

    /// Two Contexts (the second uses the first and rests on its Contract), with a Contract of two Clauses, an
    /// Interface revised once, and a Harness citing an Item and a Clause.
    pub(crate) fn project() -> Project {
        let mut p = Project::new(ProjectId::from_uuid(id(1)));
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
        let items = |sig: &str| {
            vec![
                (
                    ItemId::from_uuid(id(31)),
                    Signature::try_from(sig).unwrap(),
                    LineRange::new(1, 1).unwrap(),
                ),
                (
                    ItemId::from_uuid(id(32)),
                    Signature::try_from("pub fn pop() -> u8").unwrap(),
                    LineRange::new(2, 2).unwrap(),
                ),
            ]
        };
        p.put_interface(
            top,
            InterfaceId::from_uuid(id(IFACE)),
            Source::new(src.to_string()),
            items("pub fn push(v: u8)"),
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

    fn reasoned(c: &str) -> Reasoned {
        Reasoned::new(Choice::try_from(c).unwrap(), Some(prose("because")))
    }

    /// A change request without a Crux, and one with a Crux, a parent and a decision that asks a change.
    pub(crate) fn change_requests(p: &Project) -> Vec<ChangeRequest> {
        let targets = [
            Target::Item(ItemId::from_uuid(id(31))),
            Target::Clause(ClauseId::from_uuid(id(21))),
        ];
        let mut first = ChangeRequest::open(
            ChangeRequestId::from_uuid(id(60)),
            ChangeRequestDraft {
                title: Title::try_from("rename push").unwrap(),
                situation: None,
                intent: Intent::KeepBehavior,
                parent: None,
                issuer: None,
                lines: revisions_now(p, targets),
                crux: None,
                addresses: Vec::new(),
            },
        );
        first.resolve().unwrap();
        let crux = Crux::new(
            CruxKind::Ambiguity,
            vec![
                CruxOption::new(
                    Choice::try_from("fail").unwrap(),
                    Some(prose("C2 says fail")),
                    BTreeSet::new(),
                    Some(prose("callers check")),
                ),
                CruxOption::new(
                    Choice::try_from("zero").unwrap(),
                    None,
                    BTreeSet::from([targets[1]]),
                    None,
                ),
            ],
            Some(reasoned("fail")),
            Some(reasoned("zero")),
        )
        .unwrap();
        let mut second = ChangeRequest::open(
            ChangeRequestId::from_uuid(id(61)),
            ChangeRequestDraft {
                title: Title::try_from("pop of empty").unwrap(),
                situation: Some(prose("it is not said")),
                intent: Intent::ChangeBehavior,
                parent: Some(ChangeRequestId::from_uuid(id(60))),
                issuer: Some(Issuer::try_from("yutaka").unwrap()),
                lines: revisions_now(p, targets),
                crux: Some(crux),
                addresses: vec![
                    FindingRef::new(
                        RuleId::from_static("cite.uncited-clause"),
                        Some(Subject::Clause(ClauseId::from_uuid(id(22)))),
                    ),
                    FindingRef::new(RuleId::from_static("context.no-harness"), None),
                ],
            },
        );
        decide(&mut second, Decision::Chosen(reasoned("zero")), p).unwrap();
        vec![first, second]
    }

    /// The other kinds of decision, on Crux of kinds that allow them.
    pub(crate) fn other_decisions(p: &Project) -> Vec<ChangeRequest> {
        let open = |n: u128, title: &str, kind: CruxKind| {
            ChangeRequest::open(
                ChangeRequestId::from_uuid(id(n)),
                ChangeRequestDraft {
                    title: Title::try_from(title).unwrap(),
                    situation: None,
                    intent: Intent::ChangeBehavior,
                    parent: None,
                    issuer: None,
                    lines: revisions_now(p, [Target::Item(ItemId::from_uuid(id(31)))]),
                    crux: Some(
                        Crux::new(
                            kind,
                            vec![CruxOption::new(
                                Choice::try_from("a").unwrap(),
                                None,
                                BTreeSet::new(),
                                None,
                            )],
                            None,
                            None,
                        )
                        .unwrap(),
                    ),
                    addresses: Vec::new(),
                },
            )
        };
        let mut worded = open(70, "worded", CruxKind::Conflict);
        decide(
            &mut worded,
            Decision::Worded {
                wording: prose("in my words"),
                reason: None,
            },
            p,
        )
        .unwrap();
        let mut free = open(71, "free", CruxKind::Disagreement);
        decide(
            &mut free,
            Decision::Unspecified {
                keep: vec![prose("it terminates")],
            },
            p,
        )
        .unwrap();
        vec![worded, free]
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
    fn a_pin_comes_back_as_it_was_written() {
        let pin = pin(&project());
        let text = encode_pin(&pin).unwrap();
        assert_eq!(decode_pin(CURRENT, &text).unwrap(), pin);
    }

    #[test]
    fn change_requests_come_back_as_they_were_written() {
        let p = project();
        for cr in change_requests(&p).into_iter().chain(other_decisions(&p)) {
            let text = encode_change_request(&cr).unwrap();
            assert_eq!(decode_change_request(CURRENT, &text).unwrap(), cr);
        }
    }

    #[test]
    fn a_change_request_stored_before_it_could_address_findings_still_reads() {
        let p = project();
        for cr in change_requests(&p) {
            let mut doc: serde_json::Value =
                serde_json::from_str(&encode_change_request(&cr).unwrap()).unwrap();
            // A document of the first layout of this schema has no `addresses` at all.
            doc.as_object_mut().unwrap().remove("addresses");
            let read = decode_change_request(CURRENT, &doc.to_string()).unwrap();
            assert!(read.addresses().is_empty());
            assert_eq!(read.title(), cr.title());
        }
    }

    #[test]
    fn a_change_request_that_asks_a_change_keeps_what_it_asked_and_an_old_document_is_refused() {
        let p = project();
        let second = &change_requests(&p)[1];
        assert_eq!(second.decided_changes().len(), 1);
        // A document of the layout before decided changes names no `decided_changes`: it does not load.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(second).unwrap()).unwrap();
        doc.as_object_mut().unwrap().remove("decided_changes");
        let refused = decode_change_request(CURRENT, &doc.to_string());
        assert!(
            matches!(&refused, Err(StoreError::Corrupt(why)) if why.contains("decided_changes"))
        );
    }

    #[test]
    fn a_decided_change_the_decision_does_not_ask_is_corrupt() {
        let p = project();
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc["decision"] = serde_json::json!({ "chosen": { "choice": "fail", "reason": null } });
        assert!(matches!(
            decode_change_request(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(_))
        ));
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
            .replace("\"schema\":1", "\"schema\":0");
        assert!(matches!(
            decode_project(CURRENT, &text),
            Err(StoreError::Corrupt(_))
        ));
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
