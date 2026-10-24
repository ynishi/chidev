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

    use std::collections::BTreeMap;

    use uuid::Uuid;

    use crate::domain::entity::change_request::{ChangeRequest, ChangeRequestDraft, Crux};
    use crate::domain::entity::context::Context;
    use crate::domain::entity::pin::Pin;
    use crate::domain::entity::project::Project;
    use crate::domain::service::change_request::{decide, expectations_now};
    use crate::domain::service::pin::record;
    use crate::domain::vo::answer::{Answer, Basis, DecisionMaker};
    use crate::domain::vo::assumption::{Assumption, Evidence, KnowledgeRef, RepoPath};
    use crate::domain::vo::case::{CaseCites, CaseName};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::{ClauseContent, ClauseLabel, ClauseText};
    use crate::domain::vo::confidence::Confidence;
    use crate::domain::vo::crux::{
        CruxKind, CruxOption, Decision, Importance, Reasoned, Recommended,
    };
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::finding::{FindingRef, RuleId, Subject};
    use crate::domain::vo::id::{
        CaseId, ChangeRequestId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId,
        PinId, ProjectId,
    };
    use crate::domain::vo::intent::{Intent, Issuer};
    use crate::domain::vo::line_range::LineRange;
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::policy::PolicyLineId;
    use crate::domain::vo::prose::Prose;
    use crate::domain::vo::revision::Revision;
    use crate::domain::vo::signature::Signature;
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

    fn answer(decision: Decision, basis: Basis, by: DecisionMaker, at: u64) -> Answer {
        Answer::new(decision, basis, by, Timestamp::from_millis(at))
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

    /// A change request without a Crux that expects something of a target of every kind, and one with a Crux, a
    /// parent, an Assumption and a decision on a Policy line that asks a change.
    pub(crate) fn change_requests(p: &Project) -> Vec<ChangeRequest> {
        let targets = [
            Target::Item(ItemId::from_uuid(id(31))),
            Target::Clause(ClauseId::from_uuid(id(21))),
        ];
        let of_every_kind = [
            (Target::Project, ExpectKind::Change),
            (
                Target::Context(ContextId::from_uuid(id(11))),
                ExpectKind::Change,
            ),
            (
                Target::Contract(ContractId::from_uuid(id(20))),
                ExpectKind::Keep,
            ),
            (
                Target::Interface(InterfaceId::from_uuid(id(IFACE))),
                ExpectKind::Change,
            ),
            (
                Target::Harness(HarnessId::from_uuid(id(40))),
                ExpectKind::Keep,
            ),
            (
                Target::Clause(ClauseId::from_uuid(id(22))),
                ExpectKind::Remove,
            ),
            (Target::Item(ItemId::from_uuid(id(31))), ExpectKind::Change),
            (Target::Case(CaseId::from_uuid(id(41))), ExpectKind::Change),
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
                crux: None,
                assumptions: Vec::new(),
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
                    BTreeMap::new(),
                    Some(prose("callers check")),
                ),
                CruxOption::new(
                    Choice::try_from("zero").unwrap(),
                    None,
                    BTreeMap::from([
                        (targets[1], ExpectKind::Change),
                        (Target::Item(ItemId::from_uuid(id(32))), ExpectKind::Remove),
                    ]),
                    None,
                ),
            ],
            Some(Recommended::new(
                Choice::try_from("zero").unwrap(),
                Some(prose("callers already handle zero")),
                Some(Confidence::try_from(85).unwrap()),
            )),
            Some(Importance::High),
            None,
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
                expects: expectations_now(p, targets.map(|t| (t, ExpectKind::Change))),
                crux: Some(crux),
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
        let basis = p
            .policy()
            .basis_on(PolicyLineId::try_from("P1").unwrap())
            .unwrap();
        let by = DecisionMaker::role("orchestrator").unwrap();
        decide(
            &mut second,
            answer(
                Decision::Chosen(reasoned("zero")),
                basis,
                by,
                1_700_000_000_123,
            ),
            p,
        )
        .unwrap();
        vec![first, second]
    }

    /// The other kinds of decision, on an arbitrary basis by a person, each superseding the Crux of the change
    /// request `pop of empty` (the second of `change_requests`).
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
                    expects: expectations_now(
                        p,
                        [(Target::Item(ItemId::from_uuid(id(31))), ExpectKind::Change)],
                    ),
                    crux: Some(
                        Crux::new(
                            kind,
                            vec![CruxOption::new(
                                Choice::try_from("a").unwrap(),
                                None,
                                BTreeMap::new(),
                                None,
                            )],
                            None,
                            Some(Importance::Low),
                            Some(ChangeRequestId::from_uuid(id(61))),
                        )
                        .unwrap(),
                    ),
                    assumptions: Vec::new(),
                    addresses: Vec::new(),
                },
            )
        };
        let arbitrary = |reason: &str| Basis::Arbitrary {
            reason: prose(reason),
        };
        let alice = || DecisionMaker::person("alice").unwrap();
        let mut worded = open(70, "worded", CruxKind::Conflict);
        decide(
            &mut worded,
            answer(
                Decision::Worded {
                    wording: prose("in my words"),
                    reason: None,
                },
                arbitrary("the product leans this way"),
                alice(),
                1_700_000_000_456,
            ),
            p,
        )
        .unwrap();
        let mut free = open(71, "free", CruxKind::Disagreement);
        decide(
            &mut free,
            answer(
                Decision::Unspecified {
                    keep: vec![prose("it terminates")],
                },
                arbitrary("either is fine here"),
                alice(),
                1_700_000_000_789,
            ),
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
    fn an_answer_comes_back_with_its_basis_its_maker_and_its_time() {
        use crate::domain::vo::answer::Basis;
        use crate::domain::vo::crux::Importance;
        let p = project();
        let second = &change_requests(&p)[1];
        let text = encode_change_request(second).unwrap();
        let read = decode_change_request(CURRENT, &text).unwrap();
        let answer = read.crux().and_then(|q| q.answer()).unwrap();
        assert!(matches!(
            answer.basis(),
            Basis::Policy { line, policy_revision }
                if line.to_string() == "P1" && policy_revision.number() == p.policy().revision().number()
        ));
        assert_eq!(answer.by().to_string(), "role orchestrator");
        assert_eq!(answer.at().millis(), 1_700_000_000_123);
        assert_eq!(read.assumptions().len(), 1);
        assert_eq!(read.assumptions()[0].evidence().len(), 3);
        let q = read.crux().unwrap();
        assert_eq!(
            q.recommended()
                .and_then(|r| r.confidence())
                .map(|c| c.percent()),
            Some(85)
        );
        assert_eq!(q.importance(), Some(Importance::High));
        let superseding = &other_decisions(&p)[0];
        let text = encode_change_request(superseding).unwrap();
        let read = decode_change_request(CURRENT, &text).unwrap();
        assert_eq!(read.crux().and_then(|q| q.supersedes()), Some(second.id()));
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
        // Without the Assumptions, or the Policy, a document does not load: there is no migration.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc.as_object_mut().unwrap().remove("assumptions");
        assert!(matches!(
            decode_change_request(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("assumptions")
        ));
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_project(&p).unwrap()).unwrap();
        doc.as_object_mut().unwrap().remove("policy");
        assert!(matches!(
            decode_project(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("policy")
        ));
        // A Crux that still has a `default`, and a request that still has a `decision`, are refused by name.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc["crux"]["default"] = serde_json::json!(null);
        assert!(matches!(
            decode_change_request(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("default")
        ));
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc["decision"] = serde_json::json!(null);
        assert!(matches!(
            decode_change_request(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("decision")
        ));
        // A request without `addresses` does not load either.
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[0]).unwrap()).unwrap();
        doc.as_object_mut().unwrap().remove("addresses");
        assert!(matches!(
            decode_change_request(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("addresses")
        ));
    }

    #[test]
    fn an_invariant_the_domain_holds_is_checked_on_restore() {
        let p = project();
        let second = encode_change_request(&change_requests(&p)[1]).unwrap();
        let corrupt = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut doc: serde_json::Value = serde_json::from_str(&second).unwrap();
            edit(&mut doc);
            decode_change_request(CURRENT, &doc.to_string())
        };
        // A confidence above 100, a blank maker, an answer to a choice that is not an option, a bad line id.
        assert!(matches!(
            corrupt(&|d| d["crux"]["recommended"]["confidence"] = serde_json::json!(101)),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["crux"]["answer"]["by"] = serde_json::json!({ "role": " " })),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["crux"]["answer"]["decision"] =
                serde_json::json!({ "chosen": { "choice": "nope", "reason": null } })),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            corrupt(&|d| d["crux"]["answer"]["basis"] =
                serde_json::json!({ "policy": { "line": "P0", "policy_revision": 3 } })),
            Err(StoreError::Corrupt(_))
        ));
        // Evidence that leaves the repository.
        assert!(matches!(
            corrupt(&|d| d["assumptions"][0]["evidence"][0]["repo"]["path"] =
                serde_json::json!("../secret")),
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
        assert_eq!(second.decided_expects().len(), 2);
        // A document of the layout before expectations names no `decided_expects`: it does not load, and says why.
        for old in ["decided_expects", "expects"] {
            let mut doc: serde_json::Value =
                serde_json::from_str(&encode_change_request(second).unwrap()).unwrap();
            doc.as_object_mut().unwrap().remove(old);
            let refused = decode_change_request(CURRENT, &doc.to_string());
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
        assert_eq!(&decode_change_request(CURRENT, &text).unwrap(), first);
    }

    #[test]
    fn a_target_expected_twice_is_corrupt() {
        let p = project();
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[0]).unwrap()).unwrap();
        let again = doc["expects"][0].clone();
        doc["expects"].as_array_mut().unwrap().push(again);
        assert!(matches!(
            decode_change_request(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(why)) if why.contains("twice")
        ));
    }

    #[test]
    fn a_decided_expectation_the_decision_does_not_ask_is_corrupt() {
        let p = project();
        let mut doc: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        doc["crux"]["answer"]["decision"] =
            serde_json::json!({ "chosen": { "choice": "fail", "reason": null } });
        assert!(matches!(
            decode_change_request(CURRENT, &doc.to_string()),
            Err(StoreError::Corrupt(_))
        ));
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
