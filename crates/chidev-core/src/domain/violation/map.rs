//! The violation each error variant of the domain gives. A variant that is tied to a rule gives a violation of that
//! rule with the things it is about and the remedies that mend it; every other variant gives
//! [`UNMAPPED`](crate::domain::rule::UNMAPPED) with its debug text, and is listed by the test at the end of this file,
//! so that the list can only shrink. Every `match` here names every variant (no `_` arm), so a new variant cannot be
//! added without choosing which of the two it is.

use super::{Applicability, Party, Remedy, Value, Violation};
use crate::domain::entity::change_request::ChangeRequestError;
use crate::domain::entity::context::ContextError;
use crate::domain::entity::contract::ContractError;
use crate::domain::entity::harness::HarnessError;
use crate::domain::entity::interface::InterfaceError;
use crate::domain::entity::project::ProjectError;
use crate::domain::rule::{
    CHANGE_SET_NAMED_TWICE, CONTEXT_LAST_CONTRACT, CONTEXT_NO_SUCH_ASSET, CR_ALREADY_RESOLVED,
    CR_CRUX_ANSWERED, CR_CRUX_UNANSWERED, CR_PARENT_OPEN, CR_POLICY_LINE_UNKNOWN,
    CR_REFERENCE_ITSELF, CR_REFERENCE_UNKNOWN, CR_SUPERSEDES_UNANSWERED, CR_SUPERSEDES_UNKNOWN,
    PIN_UNCHECKED, PROJECT_BUILD_UNREACHABLE, PROJECT_CITE_UNREACHABLE, PROJECT_RETIRE_CITED,
    PROJECT_RETIRE_USED, PROJECT_UNKNOWN_CONTEXT,
};
use crate::domain::service::pin::PinError;
use crate::domain::vo::id::ChangeRequestId;
use crate::domain::vo::policy::PolicyError;
use crate::domain::vo::target::Target;

impl ChangeRequestError {
    /// The violation this refusal is. `on` is the change request the operation was on, which the error does not
    /// hold: it is the first subject of the violation.
    pub fn violation(&self, on: ChangeRequestId) -> Violation {
        let here = Party::ChangeRequest(on);
        match self {
            ChangeRequestError::AlreadyResolved => {
                Violation::of(CR_ALREADY_RESOLVED).on(here).with_remedy(
                    Remedy::ReopenChangeRequest { change_request: on },
                    Applicability::Review,
                )
            }
            ChangeRequestError::Undecided => {
                Violation::of(CR_CRUX_UNANSWERED).on(here).with_remedy(
                    Remedy::AnswerCrux { change_request: on },
                    Applicability::Placeholder,
                )
            }
            ChangeRequestError::AlreadyAnswered => {
                Violation::of(CR_CRUX_ANSWERED).on(here).with_remedy(
                    Remedy::SupersedeCrux { change_request: on },
                    Applicability::Review,
                )
            }
            ChangeRequestError::SupersedesUnknown(_) => Violation::of(CR_SUPERSEDES_UNKNOWN)
                .on(here)
                .with_remedy(Remedy::OpenChangeRequest, Applicability::Review),
            ChangeRequestError::SupersedesUnanswered(other) => {
                Violation::of(CR_SUPERSEDES_UNANSWERED)
                    .on(here)
                    .about(Party::ChangeRequest(*other))
                    .with_remedy(
                        Remedy::AnswerCrux {
                            change_request: *other,
                        },
                        Applicability::Placeholder,
                    )
            }
            ChangeRequestError::UnknownPolicyLine(line) => Violation::of(CR_POLICY_LINE_UNKNOWN)
                .on(here)
                .about(Party::PolicyLine(*line))
                .with_remedy(
                    Remedy::PutPolicyLine { line: *line },
                    Applicability::Placeholder,
                ),
            ChangeRequestError::ReferenceUnknown(_) => Violation::of(CR_REFERENCE_UNKNOWN)
                .on(here)
                .with_remedy(Remedy::OpenChangeRequest, Applicability::Review),
            ChangeRequestError::ReferencesItself => Violation::of(CR_REFERENCE_ITSELF).on(here),
            ChangeRequestError::ChildrenNotResolved { children } => {
                let mut v = Violation::of(CR_PARENT_OPEN)
                    .on(here)
                    .with_value("count", Value::Count(children.len()));
                for child in children {
                    v = v.about(Party::ChangeRequest(*child));
                }
                v.with_remedy(
                    Remedy::ResolveParts {
                        parts: children.clone(),
                    },
                    Applicability::Placeholder,
                )
            }
            ChangeRequestError::NoOption
            | ChangeRequestError::DuplicateOption(_)
            | ChangeRequestError::NotAnOption(_)
            | ChangeRequestError::UnspecifiedNotAllowed(_)
            | ChangeRequestError::NoCrux
            | ChangeRequestError::PolicyRevisionMismatch { .. }
            | ChangeRequestError::NotResolved
            | ChangeRequestError::HistoryOutOfOrder
            | ChangeRequestError::UnaskedExpectation(_) => Violation::unmapped(self).on(here),
        }
    }
}

impl ProjectError {
    /// The violations this refusal is: one, or one for each problem of a change set that broke several rules (in
    /// the order found).
    pub fn violations(&self) -> Vec<Violation> {
        match self {
            ProjectError::Many(all) => all.iter().flat_map(ProjectError::violations).collect(),
            ProjectError::Context(e) => vec![e.violation()],
            ProjectError::Policy(e) => vec![e.violation()],
            ProjectError::UnknownContext(id) => vec![
                Violation::of(PROJECT_UNKNOWN_CONTEXT)
                    .about(Party::Target(Target::Context(*id)))
                    .with_remedy(Remedy::AddContext, Applicability::Placeholder),
            ],
            ProjectError::RetireCited { cited, cases } => vec![
                Violation::of(PROJECT_RETIRE_CITED)
                    .with_value(
                        "cited",
                        Value::Parties(cited.iter().map(|t| Party::Target(*t)).collect()),
                    )
                    .subjects_of(cases.iter().map(|c| Party::Target(Target::Case(*c))))
                    .with_remedy(
                        Remedy::RetireReferrersFirst {
                            referrers: cases.iter().map(|c| Target::Case(*c)).collect(),
                        },
                        Applicability::Review,
                    ),
            ],
            ProjectError::RetireUsed { context, by } => vec![
                Violation::of(PROJECT_RETIRE_USED)
                    .about(Party::Target(Target::Context(*context)))
                    .subjects_of(by.iter().map(|c| Party::Target(Target::Context(*c))))
                    .with_remedy(
                        Remedy::ChangeUsesOf {
                            contexts: by.clone(),
                        },
                        Applicability::Review,
                    ),
            ],
            ProjectError::ItemNotReached {
                context,
                case,
                item,
            } => {
                let cited = Target::Item(*item);
                vec![unreachable_cite("item", *context, *case, cited)]
            }
            ProjectError::ClauseNotReached {
                context,
                case,
                clause,
            } => {
                let cited = Target::Clause(*clause);
                vec![unreachable_cite("clause", *context, *case, cited)]
            }
            ProjectError::BuildNotReached {
                context,
                harness,
                interface,
            } => vec![
                Violation::of(PROJECT_BUILD_UNREACHABLE)
                    .about(Party::Target(Target::Harness(*harness)))
                    .about(Party::Target(Target::Interface(*interface)))
                    .about(Party::Target(Target::Context(*context)))
                    .with_remedy(
                        Remedy::KeepCited {
                            cited: Target::Interface(*interface),
                        },
                        Applicability::Review,
                    ),
            ],
            ProjectError::Repeated { what, id } => vec![
                Violation::of(CHANGE_SET_NAMED_TWICE)
                    .about(Party::Id(*id))
                    .with_value("kind", Value::Word(what))
                    .with_remedy(Remedy::NameOnce { id: *id }, Applicability::Review),
            ],
            ProjectError::DuplicateId(_)
            | ProjectError::DuplicateName(_)
            | ProjectError::UnknownUse { .. }
            | ProjectError::Cycle(_)
            | ProjectError::IdTaken(_) => vec![Violation::unmapped(self)],
        }
    }
}

/// A Case that cites an Item or a Clause it cannot reach.
fn unreachable_cite(
    kind: &'static str,
    context: crate::domain::vo::id::ContextId,
    case: crate::domain::vo::id::CaseId,
    cited: Target,
) -> Violation {
    Violation::of(PROJECT_CITE_UNREACHABLE)
        .about(Party::Target(Target::Case(case)))
        .about(Party::Target(cited))
        .about(Party::Target(Target::Context(context)))
        .with_value("kind", Value::Word(kind))
        .with_remedy(
            Remedy::PutAgainWithoutCitation {
                case: Target::Case(case),
                cited,
            },
            Applicability::Review,
        )
        .with_remedy(Remedy::KeepCited { cited }, Applicability::Review)
}

impl Violation {
    /// The same, with these subjects after the ones it has.
    fn subjects_of(mut self, subjects: impl IntoIterator<Item = Party>) -> Violation {
        for s in subjects {
            self = self.about(s);
        }
        self
    }
}

impl ContextError {
    /// The violation this refusal is.
    pub fn violation(&self) -> Violation {
        match self {
            ContextError::LastContract => Violation::of(CONTEXT_LAST_CONTRACT)
                .with_remedy(Remedy::PutAnotherContract, Applicability::Review),
            ContextError::NoSuchAsset { kind, id } => Violation::of(CONTEXT_NO_SUCH_ASSET)
                .about(Party::Id(*id))
                .with_value("kind", Value::Word(kind)),
            ContextError::Contract(e) => e.violation(),
            ContextError::Interface(e) => e.violation(),
            ContextError::Harness(e) => e.violation(),
            ContextError::UsesItself
            | ContextError::AlreadyOnContract
            | ContextError::NoContract => Violation::unmapped(self),
        }
    }
}

impl PinError {
    /// The violation this refusal is.
    pub fn violation(&self) -> Violation {
        match self {
            PinError::Unchecked(interface) => Violation::of(PIN_UNCHECKED)
                .about(Party::Target(Target::Interface(*interface)))
                .with_remedy(
                    Remedy::CheckTheInterface {
                        interface: Target::Interface(*interface),
                    },
                    Applicability::Placeholder,
                ),
            PinError::UnknownInterface(_) => Violation::unmapped(self),
        }
    }
}

impl ContractError {
    /// The violation this refusal is: not yet tied to a rule.
    pub fn violation(&self) -> Violation {
        match self {
            ContractError::DuplicateId(_) | ContractError::DuplicateLabel(_) => {
                Violation::unmapped(self)
            }
        }
    }
}

impl InterfaceError {
    /// The violation this refusal is: not yet tied to a rule.
    pub fn violation(&self) -> Violation {
        match self {
            InterfaceError::DuplicateId(_) | InterfaceError::RangeOutside { .. } => {
                Violation::unmapped(self)
            }
        }
    }
}

impl HarnessError {
    /// The violation this refusal is: not yet tied to a rule.
    pub fn violation(&self) -> Violation {
        match self {
            HarnessError::DuplicateId(_)
            | HarnessError::RangeOutside { .. }
            | HarnessError::NoItem(_)
            | HarnessError::SimCitesItem(_)
            | HarnessError::BuildsOfTests => Violation::unmapped(self),
        }
    }
}

impl PolicyError {
    /// The violation this refusal is: not yet tied to a rule.
    pub fn violation(&self) -> Violation {
        match self {
            PolicyError::BadLineId(_)
            | PolicyError::DuplicateLine(_)
            | PolicyError::LineFromTheFuture { .. }
            | PolicyError::UnknownLine(_) => Violation::unmapped(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::domain::rule::{UNMAPPED, rule};
    use crate::domain::vo::choice::Choice;
    use crate::domain::vo::clause::ClauseLabel;
    use crate::domain::vo::id::{
        CaseId, ClauseId, ContextId, HarnessId, InterfaceId, ItemId, test_uuid,
    };
    use crate::domain::vo::name::ContextName;
    use crate::domain::vo::policy::PolicyLineId;
    use crate::domain::vo::revision::Revision;

    fn cr(n: u128) -> ChangeRequestId {
        ChangeRequestId::from_uuid(test_uuid(n))
    }
    fn line(n: &str) -> PolicyLineId {
        PolicyLineId::try_from(n).unwrap()
    }
    fn choice() -> Choice {
        Choice::try_from("up").unwrap()
    }

    /// The name of a variant; exhaustive, so a new variant is not forgotten here.
    fn cr_name(e: &ChangeRequestError) -> &'static str {
        match e {
            ChangeRequestError::NoOption => "NoOption",
            ChangeRequestError::DuplicateOption(_) => "DuplicateOption",
            ChangeRequestError::NotAnOption(_) => "NotAnOption",
            ChangeRequestError::UnspecifiedNotAllowed(_) => "UnspecifiedNotAllowed",
            ChangeRequestError::NoCrux => "NoCrux",
            ChangeRequestError::AlreadyResolved => "AlreadyResolved",
            ChangeRequestError::Undecided => "Undecided",
            ChangeRequestError::AlreadyAnswered => "AlreadyAnswered",
            ChangeRequestError::SupersedesUnknown(_) => "SupersedesUnknown",
            ChangeRequestError::SupersedesUnanswered(_) => "SupersedesUnanswered",
            ChangeRequestError::UnknownPolicyLine(_) => "UnknownPolicyLine",
            ChangeRequestError::PolicyRevisionMismatch { .. } => "PolicyRevisionMismatch",
            ChangeRequestError::NotResolved => "NotResolved",
            ChangeRequestError::ReferenceUnknown(_) => "ReferenceUnknown",
            ChangeRequestError::ReferencesItself => "ReferencesItself",
            ChangeRequestError::HistoryOutOfOrder => "HistoryOutOfOrder",
            ChangeRequestError::ChildrenNotResolved { .. } => "ChildrenNotResolved",
            ChangeRequestError::UnaskedExpectation(_) => "UnaskedExpectation",
        }
    }

    fn cr_samples() -> Vec<ChangeRequestError> {
        vec![
            ChangeRequestError::NoOption,
            ChangeRequestError::DuplicateOption(choice()),
            ChangeRequestError::NotAnOption(choice()),
            ChangeRequestError::UnspecifiedNotAllowed("model"),
            ChangeRequestError::NoCrux,
            ChangeRequestError::AlreadyResolved,
            ChangeRequestError::Undecided,
            ChangeRequestError::AlreadyAnswered,
            ChangeRequestError::SupersedesUnknown(cr(2)),
            ChangeRequestError::SupersedesUnanswered(cr(2)),
            ChangeRequestError::UnknownPolicyLine(line("P1")),
            ChangeRequestError::PolicyRevisionMismatch {
                recorded: Revision::first(),
                now: Revision::first(),
            },
            ChangeRequestError::NotResolved,
            ChangeRequestError::ReferenceUnknown(cr(2)),
            ChangeRequestError::ReferencesItself,
            ChangeRequestError::HistoryOutOfOrder,
            ChangeRequestError::ChildrenNotResolved {
                children: vec![cr(2), cr(3)],
            },
            ChangeRequestError::UnaskedExpectation(Target::Project),
        ]
    }

    fn project_name(e: &ProjectError) -> &'static str {
        match e {
            ProjectError::UnknownContext(_) => "UnknownContext",
            ProjectError::DuplicateId(_) => "DuplicateId",
            ProjectError::DuplicateName(_) => "DuplicateName",
            ProjectError::UnknownUse { .. } => "UnknownUse",
            ProjectError::Cycle(_) => "Cycle",
            ProjectError::IdTaken(_) => "IdTaken",
            ProjectError::RetireCited { .. } => "RetireCited",
            ProjectError::RetireUsed { .. } => "RetireUsed",
            ProjectError::ItemNotReached { .. } => "ItemNotReached",
            ProjectError::ClauseNotReached { .. } => "ClauseNotReached",
            ProjectError::BuildNotReached { .. } => "BuildNotReached",
            ProjectError::Repeated { .. } => "Repeated",
            ProjectError::Many(_) => "Many",
            ProjectError::Context(_) => "Context",
            ProjectError::Policy(_) => "Policy",
        }
    }

    fn ctx(n: u128) -> ContextId {
        ContextId::from_uuid(test_uuid(n))
    }

    fn project_samples() -> Vec<ProjectError> {
        vec![
            ProjectError::UnknownContext(ctx(1)),
            ProjectError::DuplicateId(ctx(1)),
            ProjectError::DuplicateName(ContextName::try_from("core").unwrap()),
            ProjectError::UnknownUse {
                context: ctx(1),
                used: ctx(2),
            },
            ProjectError::Cycle(vec![ctx(1), ctx(2)]),
            ProjectError::IdTaken(test_uuid(9)),
            ProjectError::RetireCited {
                cited: vec![Target::Clause(ClauseId::from_uuid(test_uuid(30)))],
                cases: vec![CaseId::from_uuid(test_uuid(40))],
            },
            ProjectError::RetireUsed {
                context: ctx(1),
                by: vec![ctx(2)],
            },
            ProjectError::ItemNotReached {
                context: ctx(1),
                case: CaseId::from_uuid(test_uuid(40)),
                item: ItemId::from_uuid(test_uuid(10)),
            },
            ProjectError::ClauseNotReached {
                context: ctx(1),
                case: CaseId::from_uuid(test_uuid(40)),
                clause: ClauseId::from_uuid(test_uuid(30)),
            },
            ProjectError::BuildNotReached {
                context: ctx(1),
                harness: HarnessId::from_uuid(test_uuid(400)),
                interface: InterfaceId::from_uuid(test_uuid(100)),
            },
            ProjectError::Repeated {
                what: "contract",
                id: test_uuid(7),
            },
            ProjectError::Many(vec![ProjectError::UnknownContext(ctx(1))]),
            ProjectError::Context(ContextError::LastContract),
            ProjectError::Policy(PolicyError::UnknownLine(line("P1"))),
        ]
    }

    fn context_name(e: &ContextError) -> &'static str {
        match e {
            ContextError::UsesItself => "UsesItself",
            ContextError::AlreadyOnContract => "AlreadyOnContract",
            ContextError::NoContract => "NoContract",
            ContextError::LastContract => "LastContract",
            ContextError::NoSuchAsset { .. } => "NoSuchAsset",
            ContextError::Contract(_) => "Contract",
            ContextError::Interface(_) => "Interface",
            ContextError::Harness(_) => "Harness",
        }
    }

    fn context_samples() -> Vec<ContextError> {
        vec![
            ContextError::UsesItself,
            ContextError::AlreadyOnContract,
            ContextError::NoContract,
            ContextError::LastContract,
            ContextError::NoSuchAsset {
                kind: "harness",
                id: test_uuid(4),
            },
            ContextError::Contract(ContractError::DuplicateId(ClauseId::from_uuid(test_uuid(
                1,
            )))),
            ContextError::Interface(InterfaceError::DuplicateId(ItemId::from_uuid(test_uuid(1)))),
            ContextError::Harness(HarnessError::BuildsOfTests),
        ]
    }

    /// The variants of the domain's error enums that still give `domain.unmapped`, by `Enum::Variant`, with those
    /// of the errors they wrap that are themselves listed. The count can only go down: a variant that is mapped
    /// leaves this list, and a new variant that is not mapped has to be added to it.
    const STILL_UNMAPPED: &[&str] = &[
        "ChangeRequestError::DuplicateOption",
        "ChangeRequestError::HistoryOutOfOrder",
        "ChangeRequestError::NoCrux",
        "ChangeRequestError::NoOption",
        "ChangeRequestError::NotAnOption",
        "ChangeRequestError::NotResolved",
        "ChangeRequestError::PolicyRevisionMismatch",
        "ChangeRequestError::UnaskedExpectation",
        "ChangeRequestError::UnspecifiedNotAllowed",
        "ContextError::AlreadyOnContract",
        "ContextError::NoContract",
        "ContextError::UsesItself",
        "ContractError::DuplicateId",
        "ContractError::DuplicateLabel",
        "HarnessError::BuildsOfTests",
        "HarnessError::DuplicateId",
        "HarnessError::NoItem",
        "HarnessError::RangeOutside",
        "HarnessError::SimCitesItem",
        "InterfaceError::DuplicateId",
        "InterfaceError::RangeOutside",
        "PinError::UnknownInterface",
        "PolicyError::BadLineId",
        "PolicyError::DuplicateLine",
        "PolicyError::LineFromTheFuture",
        "PolicyError::UnknownLine",
        "ProjectError::Cycle",
        "ProjectError::DuplicateId",
        "ProjectError::DuplicateName",
        "ProjectError::IdTaken",
        "ProjectError::UnknownUse",
    ];

    /// Every variant of the error enums that have a mapping, as `(Enum::Variant, the rule it gives)`.
    fn gives() -> Vec<(String, Vec<crate::domain::vo::finding::RuleId>)> {
        let mut out = Vec::new();
        for e in cr_samples() {
            out.push((
                format!("ChangeRequestError::{}", cr_name(&e)),
                vec![e.violation(cr(1)).rule().clone()],
            ));
        }
        for e in project_samples() {
            let rules = e.violations().iter().map(|v| v.rule().clone()).collect();
            // A wrapper is the variant of what it wraps.
            if !matches!(
                e,
                ProjectError::Many(_) | ProjectError::Context(_) | ProjectError::Policy(_)
            ) {
                out.push((format!("ProjectError::{}", project_name(&e)), rules));
            }
        }
        for e in context_samples() {
            if !matches!(
                e,
                ContextError::Contract(_) | ContextError::Interface(_) | ContextError::Harness(_)
            ) {
                out.push((
                    format!("ContextError::{}", context_name(&e)),
                    vec![e.violation().rule().clone()],
                ));
            }
        }
        for (name, v) in [
            (
                "ContractError::DuplicateId",
                ContractError::DuplicateId(ClauseId::from_uuid(test_uuid(1))).violation(),
            ),
            (
                "ContractError::DuplicateLabel",
                ContractError::DuplicateLabel(ClauseLabel::try_from("C1").unwrap()).violation(),
            ),
            (
                "InterfaceError::DuplicateId",
                InterfaceError::DuplicateId(ItemId::from_uuid(test_uuid(1))).violation(),
            ),
            (
                "InterfaceError::RangeOutside",
                InterfaceError::RangeOutside {
                    id: ItemId::from_uuid(test_uuid(1)),
                    first: 1,
                    last: 2,
                    total: 1,
                }
                .violation(),
            ),
            (
                "HarnessError::DuplicateId",
                HarnessError::DuplicateId(CaseId::from_uuid(test_uuid(1))).violation(),
            ),
            (
                "HarnessError::RangeOutside",
                HarnessError::RangeOutside {
                    id: CaseId::from_uuid(test_uuid(1)),
                    first: 1,
                    last: 2,
                    total: 1,
                }
                .violation(),
            ),
            (
                "HarnessError::NoItem",
                HarnessError::NoItem(CaseId::from_uuid(test_uuid(1))).violation(),
            ),
            (
                "HarnessError::SimCitesItem",
                HarnessError::SimCitesItem(CaseId::from_uuid(test_uuid(1))).violation(),
            ),
            (
                "HarnessError::BuildsOfTests",
                HarnessError::BuildsOfTests.violation(),
            ),
            (
                "PolicyError::BadLineId",
                PolicyError::BadLineId("x".into()).violation(),
            ),
            (
                "PolicyError::DuplicateLine",
                PolicyError::DuplicateLine(line("P1")).violation(),
            ),
            (
                "PolicyError::LineFromTheFuture",
                PolicyError::LineFromTheFuture {
                    line: line("P1"),
                    changed_at: Revision::first(),
                    revision: Revision::first(),
                }
                .violation(),
            ),
            (
                "PolicyError::UnknownLine",
                PolicyError::UnknownLine(line("P1")).violation(),
            ),
            (
                "PinError::UnknownInterface",
                PinError::UnknownInterface(InterfaceId::from_uuid(test_uuid(1))).violation(),
            ),
            (
                "PinError::Unchecked",
                PinError::Unchecked(InterfaceId::from_uuid(test_uuid(1))).violation(),
            ),
        ] {
            out.push((name.to_string(), vec![v.rule().clone()]));
        }
        out
    }

    #[test]
    fn the_variants_that_still_give_unmapped_are_exactly_the_listed_ones() {
        let unmapped: BTreeSet<String> = gives()
            .into_iter()
            .filter(|(_, rules)| rules.iter().all(|r| *r == UNMAPPED))
            .map(|(name, _)| name)
            .collect();
        let listed: BTreeSet<String> = STILL_UNMAPPED.iter().map(ToString::to_string).collect();
        assert_eq!(unmapped, listed, "map a variant, then take it off the list");
    }

    #[test]
    fn every_rule_a_variant_gives_is_registered_and_unmapped_is_the_only_fallback() {
        for (name, rules) in gives() {
            for r in rules {
                assert!(
                    rule(r.as_str()).is_some(),
                    "{name} gives {r}, which is not registered"
                );
            }
        }
        // The count only goes down: 18 + 15 + 8 + 2 + 2 + 5 + 4 variants of the enums with a mapping, of which
        // the wrappers (`Many`, `Context`, `Policy`, `Contract`, `Interface`, `Harness`) give their wrapped
        // error's.
        assert_eq!(gives().len(), 18 + 12 + 5 + 2 + 2 + 5 + 4 + 2);
        assert_eq!(STILL_UNMAPPED.len(), 31);
    }

    #[test]
    fn a_violation_carries_only_the_value_keys_its_rule_lists() {
        let mut all: Vec<Violation> = cr_samples().iter().map(|e| e.violation(cr(1))).collect();
        all.extend(project_samples().iter().flat_map(ProjectError::violations));
        all.extend(context_samples().iter().map(ContextError::violation));
        for v in all {
            let listed = rule(v.rule().as_str()).expect("registered").values;
            for key in v.values().keys() {
                assert!(
                    listed.contains(key),
                    "{} carries {key}, which its rule does not list",
                    v.rule()
                );
            }
        }
    }

    #[test]
    fn an_unmapped_violation_carries_the_debug_text_and_is_tracked_by_the_bug_rule() {
        let v = ChangeRequestError::NoCrux.violation(cr(1));
        assert_eq!(*v.rule(), UNMAPPED);
        assert_eq!(
            v.values().get("error"),
            Some(&Value::Text("NoCrux".to_string()))
        );
        assert_eq!(v.subjects(), &[Party::ChangeRequest(cr(1))]);
    }

    #[test]
    fn a_change_request_refusal_is_about_the_request_and_carries_its_values_and_remedies() {
        let resolved = ChangeRequestError::AlreadyResolved.violation(cr(1));
        assert_eq!(resolved.rule().as_str(), "cr.already-resolved");
        assert_eq!(resolved.subjects(), &[Party::ChangeRequest(cr(1))]);
        assert_eq!(
            resolved.remedies(),
            &[(
                Remedy::ReopenChangeRequest {
                    change_request: cr(1)
                },
                Applicability::Review
            )]
        );

        let undecided = ChangeRequestError::Undecided.violation(cr(1));
        assert_eq!(undecided.rule().as_str(), "cr.crux-unanswered");
        assert_eq!(
            undecided.remedies()[0].0,
            Remedy::AnswerCrux {
                change_request: cr(1)
            }
        );

        let open = ChangeRequestError::ChildrenNotResolved {
            children: vec![cr(2), cr(3)],
        }
        .violation(cr(1));
        assert_eq!(open.rule().as_str(), "cr.parent-open");
        assert_eq!(
            open.subjects(),
            &[
                Party::ChangeRequest(cr(1)),
                Party::ChangeRequest(cr(2)),
                Party::ChangeRequest(cr(3))
            ]
        );
        assert_eq!(open.values().get("count"), Some(&Value::Count(2)));
        assert_eq!(
            open.remedies()[0].0,
            Remedy::ResolveParts {
                parts: vec![cr(2), cr(3)]
            }
        );

        assert_eq!(
            ChangeRequestError::AlreadyAnswered
                .violation(cr(1))
                .rule()
                .as_str(),
            "cr.crux-answered"
        );
        assert_eq!(
            ChangeRequestError::ReferenceUnknown(cr(9))
                .violation(cr(1))
                .rule()
                .as_str(),
            "cr.reference-unknown"
        );
        assert_eq!(
            ChangeRequestError::SupersedesUnknown(cr(9))
                .violation(cr(1))
                .rule()
                .as_str(),
            "cr.supersedes-unknown"
        );
        let unanswered = ChangeRequestError::SupersedesUnanswered(cr(9)).violation(cr(1));
        assert_eq!(unanswered.rule().as_str(), "cr.supersedes-unanswered");
        assert_eq!(unanswered.subjects().len(), 2);
        let line_gone = ChangeRequestError::UnknownPolicyLine(line("P7")).violation(cr(1));
        assert_eq!(line_gone.rule().as_str(), "cr.policy-line-unknown");
        assert_eq!(line_gone.subjects()[1], Party::PolicyLine(line("P7")));
    }

    #[test]
    fn a_project_refusal_names_what_it_is_about_by_id_with_the_values_that_say_how() {
        let case = CaseId::from_uuid(test_uuid(40));
        let clause = ClauseId::from_uuid(test_uuid(30));
        let cited = ProjectError::RetireCited {
            cited: vec![Target::Clause(clause)],
            cases: vec![case],
        }
        .violations();
        assert_eq!(cited.len(), 1);
        assert_eq!(cited[0].rule().as_str(), "project.retire-cited");
        assert_eq!(cited[0].subjects(), &[Party::Target(Target::Case(case))]);
        assert_eq!(
            cited[0].values().get("cited"),
            Some(&Value::Parties(vec![Party::Target(Target::Clause(clause))]))
        );
        assert_eq!(
            cited[0].remedies()[0].0,
            Remedy::RetireReferrersFirst {
                referrers: vec![Target::Case(case)]
            }
        );

        let reach = ProjectError::ClauseNotReached {
            context: ctx(1),
            case,
            clause,
        }
        .violations();
        assert_eq!(reach[0].rule().as_str(), "project.cite-unreachable");
        assert_eq!(reach[0].values().get("kind"), Some(&Value::Word("clause")));
        assert_eq!(reach[0].subjects().len(), 3);
        assert_eq!(reach[0].remedies().len(), 2);

        let used = ProjectError::RetireUsed {
            context: ctx(1),
            by: vec![ctx(2), ctx(3)],
        }
        .violations();
        assert_eq!(used[0].rule().as_str(), "project.retire-used");
        assert_eq!(used[0].subjects().len(), 3);

        let twice = ProjectError::Repeated {
            what: "contract",
            id: test_uuid(7),
        }
        .violations();
        assert_eq!(twice[0].rule().as_str(), "change-set.named-twice");
        assert_eq!(
            twice[0].values().get("kind"),
            Some(&Value::Word("contract"))
        );

        let build = ProjectError::BuildNotReached {
            context: ctx(1),
            harness: HarnessId::from_uuid(test_uuid(400)),
            interface: InterfaceId::from_uuid(test_uuid(100)),
        }
        .violations();
        assert_eq!(build[0].rule().as_str(), "project.build-unreachable");

        let gone = ProjectError::UnknownContext(ctx(1)).violations();
        assert_eq!(gone[0].rule().as_str(), "project.unknown-context");
    }

    #[test]
    fn a_change_set_that_breaks_several_rules_gives_a_violation_for_each_and_wrappers_give_what_they_wrap()
     {
        let many = ProjectError::Many(vec![
            ProjectError::UnknownContext(ctx(1)),
            ProjectError::Repeated {
                what: "harness",
                id: test_uuid(1),
            },
            ProjectError::Context(ContextError::LastContract),
        ])
        .violations();
        let rules: Vec<&str> = many.iter().map(|v| v.rule().as_str()).collect();
        assert_eq!(
            rules,
            [
                "project.unknown-context",
                "change-set.named-twice",
                "context.last-contract"
            ]
        );
        let asset = ContextError::NoSuchAsset {
            kind: "interface",
            id: test_uuid(5),
        }
        .violation();
        assert_eq!(asset.rule().as_str(), "context.no-such-asset");
        assert_eq!(asset.values().get("kind"), Some(&Value::Word("interface")));
    }

    #[test]
    fn the_pin_refusal_names_the_interface_nothing_checks() {
        let i = InterfaceId::from_uuid(test_uuid(100));
        let v = PinError::Unchecked(i).violation();
        assert_eq!(v.rule().as_str(), "pin.unchecked");
        assert_eq!(v.subjects(), &[Party::Target(Target::Interface(i))]);
    }

    #[test]
    fn the_parties_of_a_violation_are_everything_it_names_once() {
        let v = ChangeRequestError::ChildrenNotResolved {
            children: vec![cr(2)],
        }
        .violation(cr(1));
        assert_eq!(
            v.parties(),
            vec![Party::ChangeRequest(cr(1)), Party::ChangeRequest(cr(2))]
        );
    }
}
