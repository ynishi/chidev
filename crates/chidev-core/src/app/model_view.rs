//! The view of the model an agent judges from: the Contexts with what they use and are based on, the Contracts with
//! their Clauses, the Interfaces with their source and Items, the Harnesses with their source and Cases, and the
//! change requests. The types are the output (typed and versioned, so that what a reader may rely on is written
//! once, here, and a later protocol for programs can send the same view): they are serialized as they are.
//!
//! Two types hold it. [`ModelView`] is the whole model, which stamps the entries of a judged finding; a
//! [`ModelSlice`] is the part of it that was asked for (some sections, some Contexts), which is what an agent is
//! given by `view` and what a program is sent by an external check. The slice carries no stamp, so nothing can
//! stamp what only part of the model was read.
//!
//! The same entries decide whether a judged finding still holds. A finding is stamped with the fingerprint of the
//! entry of its subject when it is reported, and is stale once the entry is not what it was, or is gone. One
//! build serves both, so what the agent saw and what makes its finding stale cannot differ.

use std::collections::{BTreeMap, BTreeSet};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use uuid::Uuid;

use crate::domain::entity::change_request::ChangeRequest;
use crate::domain::entity::crux::Crux;
use crate::domain::entity::project::Project;
use crate::domain::service::change_request::state_of;
use crate::domain::vo::answer::{Basis, Beyond};
use crate::domain::vo::correction::Correction;
use crate::domain::vo::crux::{CruxKind, Decision, Importance};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::finding::{FindingRef, Subject, SubjectKind};
use crate::domain::vo::id::ContextId;
use crate::domain::vo::model_section::ModelSection;
use crate::domain::vo::resolution::Transition;

/// The version of the view. It is its own: the stored documents have a schema of their own, and the two change
/// apart. A change that removes or renames a field is a new version; a field added is not. (Version 2: a change
/// request's `lines` became `expects`, each with a kind. Version 3: the kinds of an expectation are `target` and
/// `keep`; `change` and `remove` are gone. Version 4: the Crux is a record of its own, in the section `cruxes`, and a
/// change request no longer has a `crux`. Version 5: a Harness has a `kind`, and one of examples has no `source`; a
/// Case of examples has no `lines` but an `example`, so a reader of 4 would read it as a malformed Case of code.)
pub const VIEW_VERSION: u32 = 5;

/// The whole model as an agent is shown it, read when the store stood at `store_revision`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelView {
    version: u32,
    store_revision: u64,
    contexts: Vec<ContextView>,
    change_requests: Vec<ChangeRequestView>,
    /// Left out when there is none, so that the whole view of a model without a Crux is the bytes it was without the
    /// section.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cruxes: Vec<CruxView>,
}

/// A Context, with its assets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextView {
    id: String,
    name: String,
    /// `interface` or `contract`.
    basis: &'static str,
    /// The names of the Contexts it uses.
    uses: Vec<String>,
    contracts: Vec<ContractView>,
    interfaces: Vec<InterfaceView>,
    harnesses: Vec<HarnessView>,
}

/// A Contract and its Clauses.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ContractView {
    id: String,
    name: String,
    title: String,
    revision: u64,
    clauses: Vec<ClauseView>,
}

/// A Clause.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ClauseView {
    id: String,
    label: String,
    text: String,
    revision: u64,
}

/// An Interface, its source and its Items.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct InterfaceView {
    id: String,
    name: String,
    revision: u64,
    source: String,
    items: Vec<ItemView>,
}

/// An Item.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ItemView {
    id: String,
    /// The name the Item is cited by.
    key: String,
    /// The first and last line in the source.
    lines: [usize; 2],
    revision: u64,
}

/// A Harness, its kind, its source and its Cases.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct HarnessView {
    id: String,
    name: String,
    revision: u64,
    /// `tests`, `sim` or `examples`.
    kind: &'static str,
    /// Its code; absent for a Harness of examples, which has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    cases: Vec<CaseView>,
}

/// A Case: the lines of the code it is on, or the example it holds.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CaseView {
    id: String,
    name: String,
    /// The first and last line of the Case in the code; absent for an example.
    #[serde(skip_serializing_if = "Option::is_none")]
    lines: Option<[usize; 2]>,
    revision: u64,
    cites: CitesView,
    /// The example the Case holds; absent for a Case of code.
    #[serde(skip_serializing_if = "Option::is_none")]
    example: Option<ExampleView>,
}

/// An example as the model holds it, with the words of an operation.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ExampleView {
    args: String,
    stdin: String,
    /// The expected standard output, ending with the line `#exit <code>`.
    expected: String,
    /// The expected standard error; absent when the example does not compare it.
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
}

/// What a Case cites, each with the revision it recorded.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CitesView {
    items: Vec<CiteView>,
    clauses: Vec<CiteView>,
}

/// One citation.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CiteView {
    id: String,
    revision: u64,
}

/// A change request: how it stands among the others and what it asks. Its `situation`, its issuer and its
/// Assumptions are not in it. They are the request in the words of whoever asked and the readings of those words,
/// and what a reader of the view is to rely on of them has been put into the Contracts: a Harness writer who reads
/// only the view writes the tests from the promises, not from the requirements (see the guide, Roles). The questions
/// that stop it are the Cruxes raised in it, in `cruxes`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ChangeRequestView {
    id: String,
    /// What is asked.
    title: String,
    /// `change_behavior` or `keep_behavior`.
    intent: &'static str,
    /// `open`, `decided` (it has Cruxes and every one has a decision) or `resolved`.
    state: &'static str,
    /// The id of the request it is a part of; left out for one that is no part. Left out, not `null`, so that the
    /// entry of a request without a parent is the bytes it was before the field was added. A stamp taken over the
    /// change requests (a judged finding about them or the Project, an external run that reads them) is stale once
    /// when any request has a parent, as it is after any change of what it was taken over.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    /// What it asks of targets (to revise or retire, or to keep), each with the revision the target had when the
    /// request was opened.
    expects: Vec<ExpectView>,
    /// The findings it deals with.
    addresses: Vec<AddressView>,
    /// The corrections made of its texts (the title among them), in the order they were made; left out when there
    /// is none, so that the entry of a request never corrected is the bytes it was before the field was added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    corrections: Vec<CorrectionView>,
}

/// A correction of the texts of a record: which fields had a text replaced by a mark (by their words), why, who said
/// so (a claim, which nothing checks) and when. The text that was replaced is in none of it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CorrectionView {
    /// The fields, by their words (`title`, `question`, `option[<id>].choice`).
    fields: Vec<String>,
    /// Why.
    reason: String,
    by: MakerView,
    /// When, in milliseconds since the epoch.
    at: u64,
}

/// A Crux: a question that stops the change request it was raised in until a decision maker has decided it. It is
/// a record of its own, beside the change requests (`cruxes`), and points at the change request by id.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CruxView {
    id: String,
    /// The id of the change request it stops.
    raised_in: String,
    /// The findings it was raised from; none for a question that arises from the text of the change request.
    findings: Vec<AddressView>,
    /// The kind of question: `ambiguity`, `conflict`, `model` or `disagreement`.
    #[schemars(schema_with = "crux_kind_schema")]
    kind: &'static str,
    /// The question.
    question: String,
    /// `low`, `normal` or `high`; none when it was not given.
    #[schemars(schema_with = "importance_schema")]
    importance: Option<&'static str>,
    /// The options, in the order they were given.
    options: Vec<CruxOptionView>,
    /// The option recommended; none when there is none.
    recommended: Option<RecommendedView>,
    /// The id of the Crux whose decision this one replaces; none when it replaces none.
    supersedes: Option<String>,
    /// `open` (no decision yet) or `answered`.
    #[schemars(schema_with = "crux_state_schema")]
    state: &'static str,
    /// The decision, once taken; none while the Crux is open. Only what was decided: the work it asks for is in the
    /// Contracts and in the change request it was raised in.
    answer: Option<AnswerView>,
    /// The corrections made of its texts, in the order they were made; left out when there is none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    corrections: Vec<CorrectionView>,
}

/// The decision of a Crux: what was decided, what it rests on, who said so (a claim, which nothing checks) and when.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AnswerView {
    id: String,
    decision: DecisionView,
    /// Where what the decision added outside its change request went; left out when the answer says nothing of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    beyond: Option<BeyondView>,
    basis: BasisView,
    by: MakerView,
    /// When, in milliseconds since the epoch.
    at: u64,
}

/// Where what a decision added outside its change request went: `{"requests": [<change request ids>]}`, the change
/// requests opened for it, or `{"declined": <why>}`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BeyondView {
    /// The change requests opened for it, by id, as `change_requests` lists them.
    Requests(Vec<String>),
    /// It is not done, and why.
    Declined(String),
}

/// What was decided about a Crux: `{"chosen": {"option": <option id>, "reason": …}}`, `{"worded": {"wording": …,
/// "reason": …}}`, `{"unspecified": [<what must still hold>]}` or `"withdrawn"`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionView {
    /// One of the options.
    Chosen {
        /// The id of the option.
        option: String,
        /// Why.
        reason: Option<String>,
    },
    /// The behaviour in the decision maker's own words.
    Worded {
        /// The behaviour.
        wording: String,
        /// Why.
        reason: Option<String>,
    },
    /// The behaviour left free, as long as these hold.
    Unspecified(Vec<String>),
    /// The question is not needed any more.
    Withdrawn,
}

/// What a decision rests on.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BasisView {
    /// A line of the decision Policy, and the revision of the Policy it was applied at.
    Policy {
        /// The line (`P1`).
        line: String,
        /// The revision of the Policy.
        policy_revision: u64,
    },
    /// Nothing derives it; the reason it went this way.
    Arbitrary {
        /// Why.
        reason: String,
    },
}

/// Who decided: a role or a person, as they said.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct MakerView {
    /// `role` or `person`.
    kind: &'static str,
    name: String,
}

/// The schema of a Crux's `kind`: the word of one of the kinds.
fn crux_kind_schema(_: &mut SchemaGenerator) -> Schema {
    let words: Vec<&str> = CruxKind::ALL.iter().map(|k| k.word()).collect();
    json_schema!({ "type": "string", "enum": words })
}

/// The schema of a Crux's `importance`: the word of one of them, or none.
fn importance_schema(_: &mut SchemaGenerator) -> Schema {
    let words: Vec<serde_json::Value> = Importance::ALL
        .iter()
        .map(|i| serde_json::Value::from(i.word()))
        .chain([serde_json::Value::Null])
        .collect();
    json_schema!({ "enum": words })
}

/// The schema of a Crux's `state`: `open` or `answered`.
fn crux_state_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "enum": ["open", "answered"] })
}

/// An option of a Crux.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CruxOptionView {
    /// Its id, which a recommendation and a decision point at.
    id: String,
    choice: String,
    /// The wording the option would give; none when it gives none.
    wording: Option<String>,
    /// What choosing it would change beyond what the request names.
    expects: Vec<OptionExpectView>,
    /// What else choosing it affects.
    consequence: Option<String>,
}

/// What an option of a Crux would ask of one target.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct OptionExpectView {
    /// The kind of the target, as in [`ExpectView`].
    target: &'static str,
    /// Its id; none for the Project.
    id: Option<String>,
    /// `target` or `keep`.
    kind: &'static str,
}

/// The option a Crux recommends, why, and how sure (0 to 100).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct RecommendedView {
    /// The id of the option recommended.
    option: String,
    reason: Option<String>,
    confidence: Option<u8>,
}

/// What a change request asks of a target.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ExpectView {
    /// The kind of the target: `project`, `context`, `contract`, `interface`, `harness`, `clause`, `item` or
    /// `case`.
    target: &'static str,
    /// Its id; none for the Project.
    id: Option<String>,
    /// `target` (to be revised or retired) or `keep`.
    kind: &'static str,
    /// The revision the target had when the expectation was recorded.
    recorded: u64,
}

/// A finding a change request addresses.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AddressView {
    rule: String,
    /// `kind` and `id` of what it is about; none: the Project.
    subject: Option<SubjectView>,
}

/// What a finding is about, by id.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SubjectView {
    kind: &'static str,
    id: String,
}

/// The part of the model that was asked for: the Contexts asked for, each with its metadata and the sections asked
/// for, and the change requests and the Cruxes when they were. A section that was not asked for is left out, not empty: an absent
/// key says "not requested", `[]` says "none".
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ModelSlice {
    /// The version of the view format (`VIEW_VERSION`), its own and independent of the protocols that carry it.
    version: u32,
    /// The Contexts asked for.
    contexts: Vec<ContextSlice>,
    /// The change requests; present only when they were asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<ChangeRequestView>")]
    change_requests: Option<Vec<ChangeRequestView>>,
    /// The Cruxes; present only when they were asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<CruxView>")]
    cruxes: Option<Vec<CruxView>>,
}

/// A Context in a [`ModelSlice`]: what names it, and the sections that were asked for.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ContextSlice {
    id: String,
    name: String,
    /// `interface` or `contract`.
    basis: &'static str,
    /// The names of the Contexts it uses.
    uses: Vec<String>,
    /// Its Contracts; present only when they were asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<ContractView>")]
    contracts: Option<Vec<ContractView>>,
    /// Its Interfaces; present only when they were asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<InterfaceView>")]
    interfaces: Option<Vec<InterfaceView>>,
    /// Its Harnesses; present only when they were asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<HarnessView>")]
    harnesses: Option<Vec<HarnessView>>,
}

/// Which part of the model to take: the Contexts (all when none), and the sections. The names a person wrote are
/// resolved by the use case that builds it, so a selection holds ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewSelection {
    /// The Contexts to take; none: every Context.
    pub(crate) contexts: Option<BTreeSet<ContextId>>,
    /// The sections to take in each Context, and whether to take the change requests and the Cruxes.
    pub(crate) sections: BTreeSet<ModelSection>,
}

/// The fingerprint of anything the view holds. Serializing these types cannot fail; were it to, the fingerprint of
/// nothing would make every entry look unchanged, which is why there is no way for it to.
fn stamp<T: Serialize + ?Sized>(entry: &T) -> Digest {
    Digest::of(&serde_json::to_vec(entry).unwrap_or_default())
}

/// A finding a change request addresses or a Crux was raised from.
fn address_view(a: &FindingRef) -> AddressView {
    AddressView {
        rule: a.rule().to_string(),
        subject: a.subject().map(|s| SubjectView {
            kind: s.kind().word(),
            id: s.as_uuid().to_string(),
        }),
    }
}

/// A Crux as the view shows it. Every id is the id the record has; a reader that follows `raised_in` finds the change
/// request in `change_requests`, and the option a decision `chosen` in the `options` of the Crux.
fn crux_view(x: &Crux) -> CruxView {
    CruxView {
        id: x.id().to_string(),
        raised_in: x.raised_in().to_string(),
        findings: x.findings().iter().map(address_view).collect(),
        kind: x.kind().word(),
        question: x.question().as_str().to_string(),
        importance: x.importance().map(|i| i.word()),
        options: x
            .options()
            .iter()
            .map(|o| CruxOptionView {
                id: o.id().to_string(),
                choice: o.choice().as_str().to_string(),
                wording: o.change().map(|w| w.as_str().to_string()),
                expects: o
                    .expects()
                    .iter()
                    .map(|(t, k)| OptionExpectView {
                        target: t.word(),
                        id: t.as_uuid().map(|u| u.to_string()),
                        kind: k.word(),
                    })
                    .collect(),
                consequence: o.consequence().map(|w| w.as_str().to_string()),
            })
            .collect(),
        recommended: x.recommended().map(|r| RecommendedView {
            option: r.option().to_string(),
            reason: r.reason().map(|w| w.as_str().to_string()),
            confidence: r.confidence().map(|c| c.percent()),
        }),
        supersedes: x.supersedes().map(|s| s.to_string()),
        state: x.state().word(),
        answer: x.answer_given().map(|a| AnswerView {
            id: a.id().to_string(),
            decision: match a.decision() {
                Decision::Chosen { option, reason } => DecisionView::Chosen {
                    option: option.to_string(),
                    reason: reason.as_ref().map(|r| r.as_str().to_string()),
                },
                Decision::Worded { wording, reason } => DecisionView::Worded {
                    wording: wording.as_str().to_string(),
                    reason: reason.as_ref().map(|r| r.as_str().to_string()),
                },
                Decision::Unspecified { keep } => {
                    DecisionView::Unspecified(keep.iter().map(|k| k.as_str().to_string()).collect())
                }
                Decision::Withdrawn => DecisionView::Withdrawn,
            },
            beyond: a.beyond().map(|b| match b {
                Beyond::TakenBy(requests) => {
                    BeyondView::Requests(requests.iter().map(ToString::to_string).collect())
                }
                Beyond::Declined { reason } => BeyondView::Declined(reason.as_str().to_string()),
            }),
            basis: match a.basis() {
                Basis::Policy {
                    line,
                    policy_revision,
                } => BasisView::Policy {
                    line: line.to_string(),
                    policy_revision: policy_revision.number(),
                },
                Basis::Arbitrary { reason } => BasisView::Arbitrary {
                    reason: reason.as_str().to_string(),
                },
            },
            by: MakerView {
                kind: a.by().word(),
                name: a.by().name().to_string(),
            },
            at: a.at().millis(),
        }),
        corrections: x.corrections().iter().map(correction_view).collect(),
    }
}

/// A correction as the view shows it: the words of the fields, and no text.
fn correction_view<F: std::fmt::Display>(c: &Correction<F>) -> CorrectionView {
    CorrectionView {
        fields: c.fields().iter().map(ToString::to_string).collect(),
        reason: c.reason().as_str().to_string(),
        by: MakerView {
            kind: c.by().word(),
            name: c.by().name().to_string(),
        },
        at: c.at().millis(),
    }
}

impl ContextView {
    /// What stamps a Context: the Contexts' assets are stamped by their own entries.
    fn head(&self) -> (&str, &str, &str, &[String]) {
        (&self.id, &self.name, self.basis, &self.uses)
    }
}

impl ModelView {
    /// The view of `project`, `crs` and the Cruxes `cruxes` raised in them, read when the store stood at
    /// `store_revision`. `names` are the names a person gave the assets, by id; an asset with none is shown by its
    /// id.
    pub fn build(
        store_revision: u64,
        project: &Project,
        crs: &[ChangeRequest],
        cruxes: &[Crux],
        names: &BTreeMap<Uuid, String>,
    ) -> ModelView {
        let name = |id: Uuid| names.get(&id).cloned().unwrap_or_else(|| id.to_string());
        let contexts = project
            .contexts()
            .map(|c| ContextView {
                id: c.id().to_string(),
                name: c.name().to_string(),
                basis: c.basis().word(),
                uses: c
                    .uses()
                    .iter()
                    .map(|u| {
                        project
                            .context(*u)
                            .map_or_else(|| u.to_string(), |x| x.name().to_string())
                    })
                    .collect(),
                contracts: c
                    .contracts()
                    .map(|k| ContractView {
                        id: k.id().to_string(),
                        name: name(k.id().as_uuid()),
                        title: k.title().to_string(),
                        revision: k.revision().number(),
                        clauses: k
                            .clauses()
                            .iter()
                            .map(|cl| ClauseView {
                                id: cl.id().to_string(),
                                label: cl.label().to_string(),
                                text: cl.text().as_str().to_string(),
                                revision: cl.revision().number(),
                            })
                            .collect(),
                    })
                    .collect(),
                interfaces: c
                    .interfaces()
                    .map(|i| InterfaceView {
                        id: i.id().to_string(),
                        name: name(i.id().as_uuid()),
                        revision: i.revision().number(),
                        source: i.source().as_str().to_string(),
                        items: i
                            .items()
                            .iter()
                            .map(|it| ItemView {
                                id: it.id().to_string(),
                                key: it.key().to_string(),
                                lines: [it.lines().first(), it.lines().last()],
                                revision: it.revision().number(),
                            })
                            .collect(),
                    })
                    .collect(),
                harnesses: c
                    .harnesses()
                    .map(|h| HarnessView {
                        id: h.id().to_string(),
                        name: name(h.id().as_uuid()),
                        revision: h.revision().number(),
                        kind: h.kind().word(),
                        source: h.source().map(|s| s.as_str().to_string()),
                        cases: h
                            .cases()
                            .iter()
                            .map(|k| CaseView {
                                id: k.id().to_string(),
                                name: k.name().to_string(),
                                lines: k.lines().map(|l| [l.first(), l.last()]),
                                example: k.example().map(|e| ExampleView {
                                    args: e.arguments().to_string(),
                                    stdin: e.input().to_string(),
                                    expected: e.expected().to_string(),
                                    stderr: e.errors().map(str::to_string),
                                }),
                                revision: k.revision().number(),
                                cites: CitesView {
                                    items: k
                                        .cites()
                                        .items()
                                        .iter()
                                        .map(|(id, r)| CiteView {
                                            id: id.to_string(),
                                            revision: r.number(),
                                        })
                                        .collect(),
                                    clauses: k
                                        .cites()
                                        .clauses()
                                        .iter()
                                        .map(|(id, r)| CiteView {
                                            id: id.to_string(),
                                            revision: r.number(),
                                        })
                                        .collect(),
                                },
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let change_requests = crs
            .iter()
            .map(|c| ChangeRequestView {
                id: c.id().to_string(),
                title: c.title().to_string(),
                intent: c.intent().word(),
                state: state_of(c, cruxes).word(),
                expects: c
                    .expects()
                    .iter()
                    .map(|(t, e)| ExpectView {
                        target: t.word(),
                        id: t.as_uuid().map(|u| u.to_string()),
                        kind: e.kind.word(),
                        recorded: e.recorded.number(),
                    })
                    .collect(),
                addresses: c.addresses().iter().map(address_view).collect(),
                parent: c.parent().map(|p| p.to_string()),
                corrections: c
                    .history()
                    .iter()
                    .filter_map(|step| match step {
                        Transition::Corrected(k) => Some(correction_view(k)),
                        Transition::Resolved(_) | Transition::Reopened(_) => None,
                    })
                    .collect(),
            })
            .collect();
        ModelView {
            version: VIEW_VERSION,
            store_revision,
            contexts,
            change_requests,
            cruxes: cruxes.iter().map(crux_view).collect(),
        }
    }

    /// The revision of the store the view was read at.
    pub fn store_revision(&self) -> u64 {
        self.store_revision
    }

    /// The part of the model `selection` takes. The Contexts always keep their metadata; only the sections taken
    /// are kept in them. The cut is made on the typed view, so what a reader is given is what a judged rule would
    /// have been given, entry for entry.
    pub(crate) fn select(self, selection: &ViewSelection) -> ModelSlice {
        let takes = |section| selection.sections.contains(&section);
        let wanted: Option<BTreeSet<String>> = selection
            .contexts
            .as_ref()
            .map(|ids| ids.iter().map(ToString::to_string).collect());
        let contexts = self
            .contexts
            .into_iter()
            .filter(|c| wanted.as_ref().is_none_or(|ids| ids.contains(&c.id)))
            .map(|c| ContextSlice {
                id: c.id,
                name: c.name,
                basis: c.basis,
                uses: c.uses,
                contracts: takes(ModelSection::Contracts).then_some(c.contracts),
                interfaces: takes(ModelSection::Interfaces).then_some(c.interfaces),
                harnesses: takes(ModelSection::Harnesses).then_some(c.harnesses),
            })
            .collect();
        ModelSlice {
            version: self.version,
            contexts,
            change_requests: takes(ModelSection::ChangeRequests).then_some(self.change_requests),
            cruxes: takes(ModelSection::Cruxes).then_some(self.cruxes),
        }
    }

    /// The fingerprint of the entry of `subject` (of the whole model for a finding about the Project); none when
    /// the entry is gone.
    pub fn stamp_of(&self, subject: Option<Subject>) -> Option<Digest> {
        let Some(subject) = subject else {
            return Some(self.scope_stamp(SubjectKind::Project));
        };
        let id = subject.as_uuid().to_string();
        match subject {
            Subject::Context(_) => self
                .contexts
                .iter()
                .find(|c| c.id == id)
                .map(|c| stamp(&c.head())),
            Subject::Interface(_) => self.interfaces().find(|i| i.id == id).map(stamp),
            Subject::Item(_) => self.items().find(|i| i.id == id).map(stamp),
            Subject::Clause(_) => self.clauses().find(|c| c.id == id).map(stamp),
            Subject::Case(_) => self.cases().find(|c| c.id == id).map(stamp),
            Subject::ChangeRequest(_) => {
                self.change_requests.iter().find(|c| c.id == id).map(|c| {
                    let raised: Vec<&CruxView> =
                        self.cruxes.iter().filter(|q| q.raised_in == id).collect();
                    // The Cruxes raised in a request are a part of what it is: it is stamped with them. One without
                    // any is stamped as it was before there were Cruxes, so that the saved findings about it hold.
                    if raised.is_empty() {
                        stamp(c)
                    } else {
                        stamp(&(c, raised))
                    }
                })
            }
        }
    }

    /// The fingerprint of all the entries of one kind: the part of the model a rule about `kind` judges.
    pub fn scope_stamp(&self, kind: SubjectKind) -> Digest {
        match kind {
            SubjectKind::Context => {
                stamp(&self.contexts.iter().map(|c| c.head()).collect::<Vec<_>>())
            }
            SubjectKind::Interface => stamp(&self.interfaces().collect::<Vec<_>>()),
            SubjectKind::Item => stamp(&self.items().collect::<Vec<_>>()),
            SubjectKind::Clause => stamp(&self.clauses().collect::<Vec<_>>()),
            SubjectKind::Case => stamp(&self.cases().collect::<Vec<_>>()),
            // The Cruxes belong to the change requests they are raised in. A model without a Crux is stamped as it
            // was before there were Cruxes.
            SubjectKind::ChangeRequest if self.cruxes.is_empty() => stamp(&self.change_requests),
            SubjectKind::ChangeRequest => stamp(&(&self.change_requests, &self.cruxes)),
            SubjectKind::Project if self.cruxes.is_empty() => {
                stamp(&(&self.contexts, &self.change_requests))
            }
            SubjectKind::Project => stamp(&(&self.contexts, &self.change_requests, &self.cruxes)),
        }
    }

    fn interfaces(&self) -> impl Iterator<Item = &InterfaceView> {
        self.contexts.iter().flat_map(|c| c.interfaces.iter())
    }

    fn items(&self) -> impl Iterator<Item = &ItemView> {
        self.interfaces().flat_map(|i| i.items.iter())
    }

    fn clauses(&self) -> impl Iterator<Item = &ClauseView> {
        self.contexts
            .iter()
            .flat_map(|c| c.contracts.iter())
            .flat_map(|k| k.clauses.iter())
    }

    fn cases(&self) -> impl Iterator<Item = &CaseView> {
        self.contexts
            .iter()
            .flat_map(|c| c.harnesses.iter())
            .flat_map(|h| h.cases.iter())
    }
}

impl ModelSlice {
    /// A slice of no Context, no change request and no Crux, for a test that needs a value to carry.
    #[cfg(test)]
    pub(super) fn empty(version: u32) -> ModelSlice {
        ModelSlice {
            version,
            contexts: Vec::new(),
            change_requests: None,
            cruxes: None,
        }
    }

    /// The version of the view format.
    pub(super) fn version(&self) -> u32 {
        self.version
    }

    /// The Contexts taken.
    pub(super) fn contexts(&self) -> &[ContextSlice] {
        &self.contexts
    }

    /// The change requests; none when they were not taken.
    pub(super) fn change_requests(&self) -> Option<&[ChangeRequestView]> {
        self.change_requests.as_deref()
    }

    /// The Cruxes; none when they were not taken.
    pub(super) fn cruxes(&self) -> Option<&[CruxView]> {
        self.cruxes.as_deref()
    }

    /// Whether a Context with this id is in the slice.
    pub(super) fn has_context(&self, id: &str) -> bool {
        self.contexts.iter().any(|c| c.id == id)
    }

    /// Whether an Interface with this id is in the slice.
    pub(super) fn has_interface(&self, id: &str) -> bool {
        self.interfaces().any(|i| i.id == id)
    }

    /// Whether an Item with this id is in an Interface of the slice.
    pub(super) fn has_item(&self, id: &str) -> bool {
        self.interfaces()
            .flat_map(|i| i.items.iter())
            .any(|i| i.id == id)
    }

    /// Whether a Clause with this id is in a Contract of the slice.
    pub(super) fn has_clause(&self, id: &str) -> bool {
        self.contexts
            .iter()
            .flat_map(|c| c.contracts.iter().flatten())
            .flat_map(|k| k.clauses.iter())
            .any(|c| c.id == id)
    }

    /// Whether a Case with this id is in a Harness of the slice.
    pub(super) fn has_case(&self, id: &str) -> bool {
        self.contexts
            .iter()
            .flat_map(|c| c.harnesses.iter().flatten())
            .flat_map(|h| h.cases.iter())
            .any(|c| c.id == id)
    }

    /// Whether a change request with this id is in the slice.
    pub(super) fn has_change_request(&self, id: &str) -> bool {
        self.change_requests()
            .is_some_and(|xs| xs.iter().any(|c| c.id == id))
    }

    fn interfaces(&self) -> impl Iterator<Item = &InterfaceView> {
        self.contexts
            .iter()
            .flat_map(|c| c.interfaces.iter().flatten())
    }
}
