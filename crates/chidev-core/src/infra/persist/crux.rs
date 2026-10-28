//! The stored document of a Crux, schema 2: the aggregate on its own, with its options, its recommendation and its
//! answer. It refers to the change request it was raised in, to the Crux it supersedes, and to its own options by
//! id, and reaches the domain only through [`Crux::restore`], which re-checks every rule.
//!
//! Schema 2 differs from schema 1 in the answer alone: it may say where what the decision added outside its change
//! request went (`beyond`). A document of schema 1 has none and is read as it is.

use serde::{Deserialize, Serialize};

use super::v1::{
    AddressDoc, AnswerBasisDoc, Bad, CruxKindDoc, ImportanceDoc, MakerDoc, OptionExpectDoc, bad,
    option_expects_of, prose, revision, target_doc, uuid,
};
use crate::domain::entity::crux::{Crux, CruxDraft, CruxState};
use crate::domain::vo::answer::{Answer, Basis as AnswerBasis, Beyond, DecisionMaker};
use crate::domain::vo::choice::Choice;
use crate::domain::vo::confidence::Confidence;
use crate::domain::vo::crux::{CruxKind, CruxOption, Decision, Importance, Recommended};
use crate::domain::vo::finding::FindingRef;
use crate::domain::vo::id::{AnswerId, ChangeRequestId, CruxId, CruxOptionId};
use crate::domain::vo::policy::PolicyLineId;
use crate::domain::vo::prose::Prose;
use crate::domain::vo::timestamp::Timestamp;

/// The schema number a Crux document is written with.
pub(crate) const SCHEMA: u32 = 2;

/// A Crux, with its answer once it has one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CruxDoc {
    pub(super) schema: u32,
    pub(super) id: String,
    /// The id of the change request it stops.
    pub(super) raised_in: String,
    /// The findings it was raised from.
    pub(super) findings: Vec<AddressDoc>,
    pub(super) kind: CruxKindDoc,
    pub(super) question: String,
    pub(super) options: Vec<OptionDoc>,
    pub(super) recommended: Option<RecommendedDoc>,
    pub(super) importance: Option<ImportanceDoc>,
    /// The id of the Crux whose decision this one replaces.
    pub(super) supersedes: Option<String>,
    pub(super) state: StateDoc,
}

/// Whether the Crux is answered, and the answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StateDoc {
    Open,
    Answered(AnswerDoc),
}

/// An option of a Crux, with the id a recommendation and a decision point at.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OptionDoc {
    pub(super) id: String,
    pub(super) choice: String,
    pub(super) change: Option<String>,
    pub(super) expects: Vec<OptionExpectDoc>,
    pub(super) consequence: Option<String>,
}

/// The option recommended, by id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecommendedDoc {
    pub(super) option: String,
    pub(super) reason: Option<String>,
    pub(super) confidence: Option<u64>,
}

/// The answer to a Crux: its id, the decision, what it rests on, who gave it, and when (milliseconds since the
/// epoch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnswerDoc {
    pub(super) id: String,
    pub(super) decision: DecisionDoc,
    /// Where what the decision added outside its change request went; schema 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) beyond: Option<BeyondDoc>,
    pub(super) basis: AnswerBasisDoc,
    pub(super) by: MakerDoc,
    pub(super) at: u64,
}

/// The change requests opened for what an answer added outside its change request, by id, or why it is not done.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BeyondDoc {
    Requests(Vec<String>),
    Declined(String),
}

impl From<&Beyond> for BeyondDoc {
    fn from(b: &Beyond) -> BeyondDoc {
        match b {
            Beyond::TakenBy(requests) => {
                BeyondDoc::Requests(requests.iter().map(|r| r.as_uuid().to_string()).collect())
            }
            Beyond::Declined { reason } => BeyondDoc::Declined(reason.to_string()),
        }
    }
}

impl TryFrom<&BeyondDoc> for Beyond {
    type Error = Bad;

    fn try_from(d: &BeyondDoc) -> Result<Beyond, Bad> {
        match d {
            BeyondDoc::Requests(requests) => Beyond::taken_by(
                requests
                    .iter()
                    .map(|r| Ok::<_, Bad>(ChangeRequestId::from_uuid(uuid(r)?)))
                    .collect::<Result<_, _>>()?,
            )
            .map_err(bad),
            BeyondDoc::Declined(reason) => Ok(Beyond::declined(
                Prose::try_from(reason.as_str()).map_err(bad)?,
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecisionDoc {
    Chosen {
        option: String,
        reason: Option<String>,
    },
    Worded {
        wording: String,
        reason: Option<String>,
    },
    Unspecified {
        keep: Vec<String>,
    },
    Withdrawn,
}

impl From<&Decision> for DecisionDoc {
    fn from(d: &Decision) -> DecisionDoc {
        match d {
            Decision::Chosen { option, reason } => DecisionDoc::Chosen {
                option: option.as_uuid().to_string(),
                reason: reason.as_ref().map(|p| p.to_string()),
            },
            Decision::Worded { wording, reason } => DecisionDoc::Worded {
                wording: wording.to_string(),
                reason: reason.as_ref().map(|p| p.to_string()),
            },
            Decision::Unspecified { keep } => DecisionDoc::Unspecified {
                keep: keep.iter().map(|p| p.to_string()).collect(),
            },
            Decision::Withdrawn => DecisionDoc::Withdrawn,
        }
    }
}

impl TryFrom<&DecisionDoc> for Decision {
    type Error = Bad;

    fn try_from(d: &DecisionDoc) -> Result<Decision, Bad> {
        Ok(match d {
            DecisionDoc::Chosen { option, reason } => Decision::Chosen {
                option: CruxOptionId::from_uuid(uuid(option)?),
                reason: prose(reason)?,
            },
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
            DecisionDoc::Withdrawn => Decision::Withdrawn,
        })
    }
}

impl From<&Answer> for AnswerDoc {
    fn from(a: &Answer) -> AnswerDoc {
        AnswerDoc {
            id: a.id().as_uuid().to_string(),
            decision: DecisionDoc::from(a.decision()),
            beyond: a.beyond().map(BeyondDoc::from),
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
        let answer = Answer::new(
            AnswerId::from_uuid(uuid(&d.id)?),
            Decision::try_from(&d.decision)?,
            basis,
            by,
            Timestamp::from_millis(d.at),
        );
        Ok(match &d.beyond {
            Some(beyond) => answer.with_beyond(Beyond::try_from(beyond)?).map_err(bad)?,
            None => answer,
        })
    }
}

impl From<&Crux> for CruxDoc {
    fn from(q: &Crux) -> CruxDoc {
        CruxDoc {
            schema: SCHEMA,
            id: q.id().as_uuid().to_string(),
            raised_in: q.raised_in().as_uuid().to_string(),
            findings: q.findings().iter().map(AddressDoc::from).collect(),
            kind: match q.kind() {
                CruxKind::Ambiguity => CruxKindDoc::Ambiguity,
                CruxKind::Conflict => CruxKindDoc::Conflict,
                CruxKind::Model => CruxKindDoc::Model,
                CruxKind::Disagreement => CruxKindDoc::Disagreement,
            },
            question: q.question().to_string(),
            options: q
                .options()
                .iter()
                .map(|o| OptionDoc {
                    id: o.id().as_uuid().to_string(),
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
                option: r.option().as_uuid().to_string(),
                reason: r.reason().map(|p| p.to_string()),
                confidence: r.confidence().map(|c| u64::from(c.percent())),
            }),
            importance: q.importance().map(|i| match i {
                Importance::Low => ImportanceDoc::Low,
                Importance::Normal => ImportanceDoc::Normal,
                Importance::High => ImportanceDoc::High,
            }),
            supersedes: q.supersedes().map(|s| s.as_uuid().to_string()),
            state: match q.state() {
                CruxState::Open => StateDoc::Open,
                CruxState::Answered(a) => StateDoc::Answered(AnswerDoc::from(a)),
            },
        }
    }
}

impl TryFrom<CruxDoc> for Crux {
    type Error = Bad;

    fn try_from(d: CruxDoc) -> Result<Crux, Bad> {
        let mut options = Vec::with_capacity(d.options.len());
        for o in &d.options {
            options.push(CruxOption::new(
                CruxOptionId::from_uuid(uuid(&o.id)?),
                Choice::try_from(o.choice.as_str()).map_err(bad)?,
                prose(&o.change)?,
                option_expects_of(&o.expects)?,
                prose(&o.consequence)?,
            ));
        }
        let recommended = d
            .recommended
            .as_ref()
            .map(|r| {
                Ok::<_, Bad>(Recommended::new(
                    CruxOptionId::from_uuid(uuid(&r.option)?),
                    prose(&r.reason)?,
                    r.confidence
                        .map(|c| Confidence::try_from(c).map_err(bad))
                        .transpose()?,
                ))
            })
            .transpose()?;
        let state = match &d.state {
            StateDoc::Open => CruxState::Open,
            StateDoc::Answered(a) => CruxState::Answered(Answer::try_from(a)?),
        };
        let draft = CruxDraft {
            raised_in: ChangeRequestId::from_uuid(uuid(&d.raised_in)?),
            findings: d
                .findings
                .iter()
                .map(FindingRef::try_from)
                .collect::<Result<_, _>>()?,
            kind: match d.kind {
                CruxKindDoc::Ambiguity => CruxKind::Ambiguity,
                CruxKindDoc::Conflict => CruxKind::Conflict,
                CruxKindDoc::Model => CruxKind::Model,
                CruxKindDoc::Disagreement => CruxKind::Disagreement,
            },
            question: Prose::try_from(d.question.as_str()).map_err(bad)?,
            options,
            recommended,
            importance: d.importance.map(|i| match i {
                ImportanceDoc::Low => Importance::Low,
                ImportanceDoc::Normal => Importance::Normal,
                ImportanceDoc::High => Importance::High,
            }),
            supersedes: d
                .supersedes
                .as_deref()
                .map(|s| Ok::<_, Bad>(CruxId::from_uuid(uuid(s)?)))
                .transpose()?,
        };
        Crux::restore(CruxId::from_uuid(uuid(&d.id)?), draft, state).map_err(bad)
    }
}
