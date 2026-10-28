//! Renders suggestions from the domain into CLI operations and provides diagnostic wire output.

use std::borrow::Cow;
use std::collections::BTreeMap;

use chidev_core::app::{
    Beyond, DiagSubject, DiagValue, Diagnostic, Found, Mend, SuggestedOp, Suggestion, TargetInput,
};
use serde::Serialize;

use super::{
    BasisDto, BeyondDto, CaseDto, ClauseDto, DecideDto, IntentDto, ItemRefDto, KindDto, MakerDto,
    Op, OpenDto, OptionDto, PolicyLineDto, RaiseDto, RefDto, ResolutionDto,
};

/// A diagnostic as written to the wire.
#[derive(Debug, Serialize)]
pub struct DiagnosticOut<'a> {
    pub code: Cow<'a, str>,
    pub level: &'static str,
    pub message: Cow<'a, str>,
    pub subjects: Cow<'a, [DiagSubject]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<Cow<'a, str>>,
    pub values: Cow<'a, BTreeMap<String, DiagValue>>,
    pub help: Cow<'a, str>,
    pub suggestions: Vec<SuggestionOut>,
    pub did_you_mean: Cow<'a, [String]>,
}

impl<'a> From<&'a Diagnostic> for DiagnosticOut<'a> {
    fn from(d: &'a Diagnostic) -> Self {
        DiagnosticOut {
            code: Cow::Borrowed(&d.code),
            level: d.level,
            message: Cow::Borrowed(&d.message),
            subjects: Cow::Borrowed(&d.subjects),
            field: d.field.as_deref().map(Cow::Borrowed),
            values: Cow::Borrowed(&d.values),
            help: Cow::Borrowed(&d.help),
            suggestions: d.suggestions.iter().map(SuggestionOut::from).collect(),
            did_you_mean: Cow::Borrowed(&d.did_you_mean),
        }
    }
}

impl<'a> From<&'a Found> for DiagnosticOut<'a> {
    fn from(f: &'a Found) -> Self {
        let mut values = BTreeMap::new();
        values.insert(
            "state".to_string(),
            DiagValue::Text(f.state.word().to_string()),
        );
        values.insert("stale".to_string(), DiagValue::Flag(f.stale));
        let subjects = match (&f.about, f.subject) {
            (Some(name), Some(s)) => vec![DiagSubject {
                kind: s.kind().word(),
                name: name.clone(),
            }],
            (Some(name), None) => vec![DiagSubject {
                kind: "project",
                name: name.clone(),
            }],
            (None, _) => Vec::new(),
        };
        DiagnosticOut {
            code: Cow::Borrowed(f.rule.as_str()),
            level: f.level.word(),
            message: Cow::Borrowed(&f.message),
            subjects: Cow::Owned(subjects),
            field: None,
            values: Cow::Owned(values),
            help: Cow::Borrowed(&f.help),
            suggestions: Vec::new(),
            did_you_mean: Cow::Borrowed(&[]),
        }
    }
}

/// One suggestion that mends a diagnostic, on the wire.
#[derive(Debug, Serialize)]
pub struct SuggestionOut {
    pub op: SuggestionOpOut,
    pub applicability: &'static str,
}

/// The operation of a suggestion, or a sentence of advice when no operation does it.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SuggestionOpOut {
    Op(Box<Op>),
    Advice(String),
}

impl SuggestionOut {
    pub fn of(s: &Suggestion) -> Self {
        let op = match &s.mend {
            Mend::Op(suggested) => SuggestionOpOut::Op(Box::new(op_of(suggested))),
            Mend::Advice(advice) => SuggestionOpOut::Advice(advice.clone()),
        };
        SuggestionOut {
            op,
            applicability: s.applicability.word(),
        }
    }
}

impl From<&Suggestion> for SuggestionOut {
    fn from(s: &Suggestion) -> Self {
        Self::of(s)
    }
}

impl From<TargetInput> for RefDto {
    fn from(t: TargetInput) -> Self {
        match t {
            TargetInput::Project => RefDto::Project(true),
            TargetInput::Context(name) => RefDto::Context(name),
            TargetInput::Contract { context, contract } => {
                RefDto::Contract(super::ContractRefDto { context, contract })
            }
            TargetInput::Interface { context, interface } => {
                RefDto::Interface(super::InterfaceRefDto { context, interface })
            }
            TargetInput::Harness { context, harness } => {
                RefDto::Harness(super::HarnessRefDto { context, harness })
            }
            TargetInput::Item(r) => RefDto::Item(super::ItemRefDto {
                context: r.context,
                interface: r.interface,
                key: r.key,
            }),
            TargetInput::Clause(r) => RefDto::Clause(super::ClauseRefDto {
                context: r.context,
                contract: r.contract,
                label: r.label,
            }),
            TargetInput::Case {
                context,
                harness,
                name,
            } => RefDto::Case(super::CaseRefDto {
                context,
                harness,
                name,
            }),
        }
    }
}

/// The operation that mends a violation, as the CLI reads it.
///
/// Hole strings write what the application leaves to whoever acts on the suggestion:
/// - reasons: `"<why>"`, `"<why the work goes on>"`, `"<why they are not done>"`
/// - decision makers: `{"person": "<who>"}`
/// - options and questions: `"<one of its option ids>"`, `"<the question>"`, `"<the new choice>"`
/// - titles and names: `"<title>"`, `"<title of the request for them>"`, `"<context>"`, `"<the context>"`, `"<name>"`, `"<harness>"`
/// - code and tests: `"<code>"`, `"<its code>"`, `"<tests>"`
/// - structure: `"<label>"`, `"<text>"`, `"<case>"`, `"<key>"`, `"<the line>"`
///
/// Defaults applied that the application does not specify:
/// - change request intent: `change_behavior`
/// - crux kind: `ambiguity`
/// - case lines: `[1, 1]`
/// - change request resolution: `done`
pub fn op_of(op: &SuggestedOp) -> Op {
    match op {
        SuggestedOp::Init => Op::Init,
        SuggestedOp::AddContext { name } => Op::ContextAdd {
            name: name.clone().unwrap_or_else(|| "<name>".to_string()),
            uses: Vec::new(),
        },
        SuggestedOp::SetUsesWithout { context, without } => {
            let without_text = without.as_deref().unwrap_or("<the context>");
            Op::ContextUses {
                context: context.clone(),
                uses: vec![format!("<its uses without {without_text}>")],
            }
        }
        SuggestedOp::MakeContract { context, name } => Op::ContractPut {
            context: context.clone().unwrap_or_else(|| "<context>".to_string()),
            name: name.clone(),
            title: "<title>".to_string(),
            clauses: vec![ClauseDto {
                label: "<label>".to_string(),
                text: "<text>".to_string(),
                replaces: None,
            }],
        },
        SuggestedOp::MakeInterface { context, name } => Op::InterfacePut {
            context: context.clone().unwrap_or_else(|| "<context>".to_string()),
            name: name.clone(),
            source: Some("<code>".to_string()),
            source_file: None,
            items: None,
            renames: Vec::new(),
        },
        SuggestedOp::MakeHarness { context, name } => Op::HarnessPut {
            context: context.clone().unwrap_or_else(|| "<context>".to_string()),
            name: name.clone(),
            kind: None,
            builds: Vec::new(),
            source: Some("<code>".to_string()),
            source_file: None,
            cases: vec![CaseDto {
                name: "<case>".to_string(),
                lines: [1, 1],
                items: vec![ItemRefDto {
                    context: None,
                    interface: "<interface>".to_string(),
                    key: "<key>".to_string(),
                }],
                clauses: Vec::new(),
            }],
        },
        SuggestedOp::PutHarnessWithoutCitation { context, harness } => Op::HarnessPut {
            context: context.clone(),
            name: harness.clone(),
            kind: None,
            builds: Vec::new(),
            source: Some("<its code>".to_string()),
            source_file: None,
            cases: vec![CaseDto {
                name: "<its cases, without the citation>".to_string(),
                lines: [1, 1],
                items: Vec::new(),
                clauses: Vec::new(),
            }],
        },
        SuggestedOp::CheckInterface { context, interface } => Op::HarnessPut {
            context: context.clone(),
            name: "<harness>".to_string(),
            kind: None,
            builds: Vec::new(),
            source: Some("<tests>".to_string()),
            source_file: None,
            cases: vec![CaseDto {
                name: "<case>".to_string(),
                lines: [1, 1],
                items: vec![ItemRefDto {
                    context: None,
                    interface: interface.clone(),
                    key: "<key>".to_string(),
                }],
                clauses: Vec::new(),
            }],
        },
        SuggestedOp::RetireHarness { context, harness } => Op::HarnessRetire {
            context: context.clone(),
            harness: harness.clone(),
        },
        SuggestedOp::OpenChangeRequest { title } => Op::CrOpen(Box::new(OpenDto {
            title: title.clone().unwrap_or_else(|| "<title>".to_string()),
            situation: None,
            intent: IntentDto::ChangeBehavior,
            parent: None,
            issuer: None,
            targets: Vec::new(),
            keep: Vec::new(),
            assumptions: Vec::new(),
            addresses: Vec::new(),
        })),
        SuggestedOp::OpenRequestFor { targets } => {
            let targets = match targets {
                Some(ts) => ts.iter().cloned().map(RefDto::from).collect(),
                None => vec![RefDto::Context("<the targets outside>".to_string())],
            };
            Op::CrOpen(Box::new(OpenDto {
                title: "<title of the request for them>".to_string(),
                situation: None,
                intent: IntentDto::ChangeBehavior,
                parent: None,
                issuer: None,
                targets,
                keep: Vec::new(),
                assumptions: Vec::new(),
                addresses: Vec::new(),
            }))
        }
        SuggestedOp::ResolveDone { title } => Op::CrResolve {
            title: title.clone(),
            resolution: ResolutionDto::Done,
            reason: "<why>".to_string(),
            by: MakerDto::Person("<who>".to_string()),
        },
        SuggestedOp::ReopenChangeRequest { title } => Op::CrReopen {
            title: title.clone(),
            reason: "<why the work goes on>".to_string(),
            by: MakerDto::Person("<who>".to_string()),
        },
        SuggestedOp::DecideCrux { crux } => Op::CrCruxDecide(Box::new(DecideDto {
            crux: crux.clone(),
            option: Some("<one of its option ids>".to_string()),
            wording: None,
            unspecified: None,
            withdrawn: None,
            reason: None,
            beyond: None,
            basis: BasisDto::Arbitrary("<why>".to_string()),
            by: MakerDto::Person("<who>".to_string()),
        })),
        SuggestedOp::DecideCruxBeyond { crux, beyond } => {
            let beyond_dto = match beyond {
                Beyond::RequestFor => {
                    BeyondDto::Requests(vec!["<title of the request for them>".to_string()])
                }
                Beyond::Declined => BeyondDto::Declined("<why they are not done>".to_string()),
            };
            Op::CrCruxDecide(Box::new(DecideDto {
                crux: crux.clone(),
                option: None,
                wording: None,
                unspecified: None,
                withdrawn: None,
                reason: None,
                beyond: Some(beyond_dto),
                basis: BasisDto::Arbitrary("<why>".to_string()),
                by: MakerDto::Person("<who>".to_string()),
            }))
        }
        SuggestedOp::RaiseSuperseding {
            change_request,
            supersedes,
        } => Op::CrCruxRaise(Box::new(RaiseDto {
            cr: change_request.clone(),
            question: "<the question>".to_string(),
            kind: KindDto::Ambiguity,
            findings: Vec::new(),
            options: vec![OptionDto {
                choice: "<the new choice>".to_string(),
                wording: None,
                targets: Vec::new(),
                keep: Vec::new(),
                consequence: None,
                recommended: None,
            }],
            importance: None,
            supersedes: Some(supersedes.clone()),
        })),
        SuggestedOp::PutPolicy { lines, adding } => {
            let mut policy_lines: Vec<PolicyLineDto> = lines
                .iter()
                .map(|(id, text)| PolicyLineDto {
                    id: id.clone(),
                    text: text.clone(),
                })
                .collect();
            policy_lines.push(PolicyLineDto {
                id: adding.clone(),
                text: "<the line>".to_string(),
            });
            Op::PolicyPut {
                lines: policy_lines,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chidev_core::app::{Applicability, Beyond, ClauseRef, ItemRef, SuggestedOp, TargetInput};
    use serde_json::json;

    use super::op_of;
    use crate::ops::Op;

    fn covered(op: &SuggestedOp) -> usize {
        match op {
            SuggestedOp::Init => 0,
            SuggestedOp::AddContext { .. } => 1,
            SuggestedOp::SetUsesWithout { .. } => 2,
            SuggestedOp::MakeContract { .. } => 3,
            SuggestedOp::MakeInterface { .. } => 4,
            SuggestedOp::MakeHarness { .. } => 5,
            SuggestedOp::PutHarnessWithoutCitation { .. } => 6,
            SuggestedOp::CheckInterface { .. } => 7,
            SuggestedOp::RetireHarness { .. } => 8,
            SuggestedOp::OpenChangeRequest { .. } => 9,
            SuggestedOp::OpenRequestFor { .. } => 10,
            SuggestedOp::ResolveDone { .. } => 11,
            SuggestedOp::ReopenChangeRequest { .. } => 12,
            SuggestedOp::DecideCrux { .. } => 13,
            SuggestedOp::DecideCruxBeyond { .. } => 14,
            SuggestedOp::RaiseSuperseding { .. } => 15,
            SuggestedOp::PutPolicy { .. } => 16,
        }
    }

    fn samples() -> Vec<SuggestedOp> {
        vec![
            SuggestedOp::Init,
            SuggestedOp::AddContext { name: None },
            SuggestedOp::AddContext {
                name: Some("cor".to_string()),
            },
            SuggestedOp::SetUsesWithout {
                context: "core".to_string(),
                without: None,
            },
            SuggestedOp::SetUsesWithout {
                context: "core".to_string(),
                without: Some("pricing".to_string()),
            },
            SuggestedOp::MakeContract {
                context: None,
                name: "rounding".to_string(),
            },
            SuggestedOp::MakeContract {
                context: Some("core".to_string()),
                name: "rounding".to_string(),
            },
            SuggestedOp::MakeInterface {
                context: None,
                name: "api".to_string(),
            },
            SuggestedOp::MakeInterface {
                context: Some("core".to_string()),
                name: "api".to_string(),
            },
            SuggestedOp::MakeHarness {
                context: None,
                name: "tests".to_string(),
            },
            SuggestedOp::MakeHarness {
                context: Some("core".to_string()),
                name: "tests".to_string(),
            },
            SuggestedOp::PutHarnessWithoutCitation {
                context: "core".to_string(),
                harness: "tests".to_string(),
            },
            SuggestedOp::CheckInterface {
                context: "core".to_string(),
                interface: "api".to_string(),
            },
            SuggestedOp::RetireHarness {
                context: "core".to_string(),
                harness: "tests".to_string(),
            },
            SuggestedOp::OpenChangeRequest { title: None },
            SuggestedOp::OpenChangeRequest {
                title: Some("T".to_string()),
            },
            SuggestedOp::OpenRequestFor { targets: None },
            SuggestedOp::OpenRequestFor {
                targets: Some(vec![
                    TargetInput::Project,
                    TargetInput::Context("core".to_string()),
                    TargetInput::Contract {
                        context: "core".to_string(),
                        contract: "rounding".to_string(),
                    },
                    TargetInput::Interface {
                        context: "core".to_string(),
                        interface: "api".to_string(),
                    },
                    TargetInput::Harness {
                        context: "core".to_string(),
                        harness: "tests".to_string(),
                    },
                    TargetInput::Item(ItemRef {
                        context: Some("core".to_string()),
                        interface: "api".to_string(),
                        key: "k".to_string(),
                    }),
                    TargetInput::Clause(ClauseRef {
                        context: None,
                        contract: "rounding".to_string(),
                        label: "C1".to_string(),
                    }),
                    TargetInput::Case {
                        context: "core".to_string(),
                        harness: "tests".to_string(),
                        name: "c1".to_string(),
                    },
                ]),
            },
            SuggestedOp::ResolveDone {
                title: "T".to_string(),
            },
            SuggestedOp::ReopenChangeRequest {
                title: "T".to_string(),
            },
            SuggestedOp::DecideCrux {
                crux: "cx".to_string(),
            },
            SuggestedOp::DecideCruxBeyond {
                crux: "cx".to_string(),
                beyond: Beyond::RequestFor,
            },
            SuggestedOp::DecideCruxBeyond {
                crux: "cx".to_string(),
                beyond: Beyond::Declined,
            },
            SuggestedOp::RaiseSuperseding {
                change_request: "cr".to_string(),
                supersedes: "old".to_string(),
            },
            SuggestedOp::PutPolicy {
                lines: vec![("P1".to_string(), "first line".to_string())],
                adding: "P2".to_string(),
            },
        ]
    }

    fn contains_hole(val: &serde_json::Value) -> bool {
        match val {
            serde_json::Value::String(s) => s.contains('<') && s.contains('>'),
            serde_json::Value::Array(arr) => arr.iter().any(contains_hole),
            serde_json::Value::Object(map) => map.values().any(contains_hole),
            _ => false,
        }
    }

    #[test]
    fn every_suggested_op_is_an_operation_the_cli_reads() {
        let all_samples = samples();
        let covered_indices: BTreeSet<usize> = all_samples.iter().map(covered).collect();
        assert_eq!(
            covered_indices.len(),
            17,
            "samples must cover every variant 0..=16"
        );
        for i in 0..=16 {
            assert!(covered_indices.contains(&i), "variant {i} not covered");
        }

        for s in all_samples {
            let op = op_of(&s);
            let val = serde_json::to_value(&op).expect("serialize suggested op");
            let _parsed: Op = serde_json::from_value(val.clone()).unwrap_or_else(|e| {
                panic!("cannot parse back into Op: {e}\njson was: {val:#?}");
            });
            assert_eq!(
                contains_hole(&val),
                s.has_hole(),
                "mismatch on hole detection for {s:?}: json was {val}"
            );
        }
    }

    #[test]
    fn the_written_operations_keep_their_shape() {
        // AddContext{Some("cor")} == {"op":"context_add","name":"cor"}
        let op = op_of(&SuggestedOp::AddContext {
            name: Some("cor".to_string()),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({"op":"context_add","name":"cor"})
        );

        // OpenChangeRequest{Some("T")} == {"op":"cr_open","title":"T","intent":"change_behavior"}
        let op = op_of(&SuggestedOp::OpenChangeRequest {
            title: Some("T".to_string()),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({"op":"cr_open","title":"T","intent":"change_behavior"})
        );

        // ResolveDone
        let op = op_of(&SuggestedOp::ResolveDone {
            title: "T".to_string(),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"cr_resolve",
                "title":"T",
                "resolution":"done",
                "reason":"<why>",
                "by":{"person":"<who>"}
            })
        );

        // DecideCruxBeyond(Declined)
        let op = op_of(&SuggestedOp::DecideCruxBeyond {
            crux: "c".to_string(),
            beyond: Beyond::Declined,
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"cr_crux_decide",
                "crux":"c",
                "beyond":{"declined":"<why they are not done>"},
                "basis":{"arbitrary":"<why>"},
                "by":{"person":"<who>"}
            })
        );

        // RaiseSuperseding
        let op = op_of(&SuggestedOp::RaiseSuperseding {
            change_request: "cr".to_string(),
            supersedes: "old".to_string(),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"cr_crux_raise",
                "in":"cr",
                "supersedes":"old",
                "kind":"ambiguity",
                "question":"<the question>",
                "options":[{"choice":"<the new choice>"}]
            })
        );

        // MakeContract
        let op = op_of(&SuggestedOp::MakeContract {
            context: Some("ctx".to_string()),
            name: "cnt".to_string(),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"contract_put",
                "context":"ctx",
                "name":"cnt",
                "title":"<title>",
                "clauses":[{"label":"<label>","text":"<text>"}]
            })
        );

        // MakeInterface
        let op = op_of(&SuggestedOp::MakeInterface {
            context: Some("ctx".to_string()),
            name: "iface".to_string(),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"interface_put",
                "context":"ctx",
                "name":"iface",
                "source":"<code>"
            })
        );

        // MakeHarness
        let op = op_of(&SuggestedOp::MakeHarness {
            context: Some("ctx".to_string()),
            name: "h".to_string(),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"harness_put",
                "context":"ctx",
                "name":"h",
                "source":"<code>",
                "cases":[{"name":"<case>","lines":[1,1],"items":[{"interface":"<interface>","key":"<key>"}]}]
            })
        );

        // PutHarnessWithoutCitation
        let op = op_of(&SuggestedOp::PutHarnessWithoutCitation {
            context: "ctx".to_string(),
            harness: "h".to_string(),
        });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"harness_put",
                "context":"ctx",
                "name":"h",
                "source":"<its code>",
                "cases":[{"name":"<its cases, without the citation>","lines":[1,1]}]
            })
        );

        // OpenRequestFor{None}
        let op = op_of(&SuggestedOp::OpenRequestFor { targets: None });
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({
                "op":"cr_open",
                "title":"<title of the request for them>",
                "intent":"change_behavior",
                "targets":[{"context":"<the targets outside>"}]
            })
        );
    }

    #[test]
    fn a_suggestion_with_a_hole_is_a_placeholder_whatever_it_was_given() {
        assert!(!SuggestedOp::Init.has_hole());
        assert!(
            !SuggestedOp::AddContext {
                name: Some("a".to_string())
            }
            .has_hole()
        );
        assert!(SuggestedOp::AddContext { name: None }.has_hole());
        assert!(
            !SuggestedOp::OpenChangeRequest {
                title: Some("T".to_string())
            }
            .has_hole()
        );
        assert!(SuggestedOp::OpenChangeRequest { title: None }.has_hole());
        assert!(
            SuggestedOp::MakeContract {
                context: Some("c".to_string()),
                name: "n".to_string()
            }
            .has_hole()
        );

        let s1 = chidev_core::app::suggest(
            chidev_core::app::Mend::Op(SuggestedOp::Init),
            Applicability::Machine,
        );
        assert_eq!(s1.applicability, Applicability::Machine);

        let s2 = chidev_core::app::suggest(
            chidev_core::app::Mend::Op(SuggestedOp::AddContext { name: None }),
            Applicability::Machine,
        );
        assert_eq!(s2.applicability, Applicability::Placeholder);
    }
}
