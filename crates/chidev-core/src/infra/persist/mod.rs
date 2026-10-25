//! How the aggregates are written to the store and read back: one JSON document each, in a versioned schema
//! ([`v1`]). A document that is newer than this code, or that the domain's rules refuse, is `Corrupt`, never
//! quietly repaired.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::repository::StoreError;

pub(crate) mod measure;
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

    /// A change request without a Crux that names a target or a keep of every kind, resolved, reopened and resolved
    /// again, and one with a Crux, a parent, an Assumption and a decision on a Policy line that names a target.
    pub(crate) fn change_requests(p: &Project) -> Vec<ChangeRequest> {
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
                crux: None,
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
                        (targets[1], ExpectKind::Target),
                        (Target::Item(ItemId::from_uuid(id(32))), ExpectKind::Target),
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
                expects: expectations_now(p, targets.map(|t| (t, ExpectKind::Target))),
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
                        [(Target::Item(ItemId::from_uuid(id(31))), ExpectKind::Target)],
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
    fn change_requests_come_back_as_they_were_written() {
        let p = project();
        for cr in change_requests(&p).into_iter().chain(other_decisions(&p)) {
            let text = encode_change_request(&cr).unwrap();
            assert_eq!(decode_change_request(CURRENT, &text).unwrap(), cr);
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
            .find(|c| {
                c.state() != crate::domain::entity::change_request::ChangeRequestState::Resolved
            })
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
        cr.record_work(Footprint::new(9, Vec::new())).unwrap();
        let text = encode_change_request(&cr).unwrap();
        let read = decode_change_request(CURRENT, &text).unwrap();
        assert_eq!(read.work(), cr.work());
        assert_eq!(read, cr);
        // a document without it is of before it existed and does not load
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc.as_object_mut().unwrap().remove("work");
        assert!(decode_change_request(CURRENT, &doc.to_string()).is_err());
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
        let read = decode_change_request(CURRENT, &doc.to_string()).unwrap();
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
                decode_change_request(CURRENT, &old.to_string()),
                Err(StoreError::Corrupt(m)) if m.contains(why)
            ));
        }
        // Steps out of order are refused by the domain.
        let mut swapped = doc.clone();
        swapped["history"].as_array_mut().unwrap().swap(0, 1);
        assert!(matches!(
            decode_change_request(CURRENT, &swapped.to_string()),
            Err(StoreError::Corrupt(_))
        ));
        // A resolution as done of a request whose Crux has no answer is refused.
        let mut unanswered: serde_json::Value =
            serde_json::from_str(&encode_change_request(&change_requests(&p)[1]).unwrap()).unwrap();
        unanswered["crux"]["answer"] = serde_json::Value::Null;
        unanswered["decided_expects"] = serde_json::json!([]);
        unanswered["history"] = serde_json::json!([{ "resolved": {
            "kind": "done", "reason": "r", "by": { "person": "alice" }, "at": 1 } }]);
        assert!(matches!(
            decode_change_request(CURRENT, &unanswered.to_string()),
            Err(StoreError::Corrupt(_))
        ));
        // Not done, the question may stay open.
        unanswered["history"][0]["resolved"]["kind"] = serde_json::json!("wont_do");
        assert!(decode_change_request(CURRENT, &unanswered.to_string()).is_ok());
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
    fn a_pin_and_a_change_request_of_schema_1_are_read_as_they_are() {
        let p = project();
        let pin = pin(&p);
        let text = encode_pin(&pin)
            .unwrap()
            .replace("\"schema\":2", "\"schema\":1");
        assert_eq!(decode_pin(1, &text).unwrap(), pin);
        for cr in change_requests(&p) {
            let text = encode_change_request(&cr)
                .unwrap()
                .replace("\"schema\":2", "\"schema\":1");
            assert_eq!(decode_change_request(1, &text).unwrap(), cr);
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
