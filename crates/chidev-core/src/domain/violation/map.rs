//! The violation each error variant of the domain gives. A variant that is tied to a rule gives a violation of that
//! rule with the things it is about and the remedies that mend it; every other variant gives
//! [`UNMAPPED`](crate::domain::rule::UNMAPPED) with its debug text, and is listed by the test at the end of this file,
//! so that the list can only shrink. Every `match` here names every variant (no `_` arm), so a new variant cannot be
//! added without choosing which of the two it is.

use super::{Applicability, Party, Remedy, Value, Violation};
use crate::domain::entity::change_request::ChangeRequestError;
use crate::domain::entity::context::ContextError;
use crate::domain::entity::contract::ContractError;
use crate::domain::entity::harness::{HarnessError, SimShapeProblem};
use crate::domain::entity::interface::InterfaceError;
use crate::domain::entity::project::ProjectError;
use crate::domain::rule::{
    CHANGE_SET_NAMED_TWICE, CONTEXT_ALREADY_BASED, CONTEXT_LAST_CONTRACT, CONTEXT_NO_SUCH_ASSET,
    CR_ALREADY_RESOLVED, CR_CRUX_ANSWERED, CR_CRUX_UNANSWERED, CR_CRUX_UNKNOWN_OPTION,
    CR_PARENT_OPEN, CR_POLICY_LINE_UNKNOWN, CR_REFERENCE_ITSELF, CR_REFERENCE_UNKNOWN,
    CR_SUPERSEDES_UNANSWERED, CR_SUPERSEDES_UNKNOWN, HARNESS_LINES_OUTSIDE, INTERFACE_BLANK_ITEM,
    INTERFACE_KEY_TAKEN, INTERFACE_LINES_OUTSIDE, PIN_UNCHECKED, PROJECT_BUILD_UNREACHABLE,
    PROJECT_CITE_UNREACHABLE, PROJECT_RETIRE_CITED, PROJECT_RETIRE_USED, PROJECT_UNKNOWN_CONTEXT,
    SIM_BUILDS_NONE, SIM_BUILDS_OF_TESTS, SIM_CASE_CITES_ITEM, SIM_CASE_CITES_NONE, SIM_CASE_NAME,
    SIM_CLAUSE_FOREIGN, SIM_ONE_PER_CONTEXT, SIM_SWEEP_ONCE,
};
use crate::domain::service::pin::PinError;
use crate::domain::vo::id::{CaseId, ChangeRequestId, HarnessId};
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
            ChangeRequestError::SupersedesUnanswered(_) => {
                Violation::of(CR_SUPERSEDES_UNANSWERED).on(here)
            }
            ChangeRequestError::NotAnOption(_) => Violation::of(CR_CRUX_UNKNOWN_OPTION).on(here),
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
            | ChangeRequestError::UnspecifiedNotAllowed(_)
            | ChangeRequestError::PolicyRevisionMismatch { .. }
            | ChangeRequestError::NotResolved
            | ChangeRequestError::HistoryOutOfOrder
            | ChangeRequestError::UnaskedExpectation(_)
            | ChangeRequestError::ExpectationConflict(_)
            | ChangeRequestError::RaisedInUnknown(_) => Violation::unmapped(self).on(here),
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
            ProjectError::SimShape(problem) => vec![problem.violation()],
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
            ContextError::AlreadyOnContract => Violation::of(CONTEXT_ALREADY_BASED),
            ContextError::UsesItself | ContextError::NoContract => Violation::unmapped(self),
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
    /// The violation this refusal is. The key is the application's to name: the violation holds the id of the Item
    /// that has it.
    pub fn violation(&self) -> Violation {
        match self {
            InterfaceError::KeyTaken { id, .. } => {
                Violation::of(INTERFACE_KEY_TAKEN).about(Party::Target(Target::Item(*id)))
            }
            InterfaceError::BlankItem { id } => {
                Violation::of(INTERFACE_BLANK_ITEM).about(Party::Target(Target::Item(*id)))
            }
            InterfaceError::RangeOutside {
                id,
                first,
                last,
                total,
            } => Violation::of(INTERFACE_LINES_OUTSIDE)
                .about(Party::Target(Target::Item(*id)))
                .with_value("first", Value::Count(*first))
                .with_value("last", Value::Count(*last))
                .with_value("total", Value::Count(*total)),
            InterfaceError::DuplicateId(_) => Violation::unmapped(self),
        }
    }
}

impl HarnessError {
    /// The violation this refusal is.
    pub fn violation(&self) -> Violation {
        match self {
            HarnessError::SimCitesItem(case) => {
                Violation::of(SIM_CASE_CITES_ITEM).about(Party::Target(Target::Case(*case)))
            }
            HarnessError::BuildsOfTests(harness) => {
                Violation::of(SIM_BUILDS_OF_TESTS).about(Party::Target(Target::Harness(*harness)))
            }
            HarnessError::RangeOutside {
                id,
                first,
                last,
                total,
            } => Violation::of(HARNESS_LINES_OUTSIDE)
                .about(Party::Target(Target::Case(*id)))
                .with_value("first", Value::Count(*first))
                .with_value("last", Value::Count(*last))
                .with_value("total", Value::Count(*total)),
            HarnessError::DuplicateId(_) | HarnessError::NoItem(_) => Violation::unmapped(self),
        }
    }
}

impl SimShapeProblem {
    /// The violation this problem is. The names of the Harness, the Case and the Clause are the application's: the
    /// violation holds their ids.
    pub fn violation(&self) -> Violation {
        let harness = |h: &HarnessId| Party::Target(Target::Harness(*h));
        let case = |c: &CaseId| Party::Target(Target::Case(*c));
        match self {
            SimShapeProblem::OnePerContext { context, harnesses } => {
                Violation::of(SIM_ONE_PER_CONTEXT)
                    .about(Party::Target(Target::Context(*context)))
                    .subjects_of(harnesses.iter().map(harness))
            }
            SimShapeProblem::SweepOnce {
                harness: h,
                count,
                extra,
            } => {
                let v = Violation::of(SIM_SWEEP_ONCE)
                    .about(harness(h))
                    .with_value("count", Value::Count(*count));
                match extra {
                    Some(c) => v.about(case(c)),
                    None => v,
                }
            }
            SimShapeProblem::CaseName {
                harness: h,
                case: c,
                fault,
            } => Violation::of(SIM_CASE_NAME)
                .about(case(c))
                .about(harness(h))
                .with_value("reason", Value::Word(fault.word())),
            SimShapeProblem::CaseCitesNone {
                harness: h,
                case: c,
            } => Violation::of(SIM_CASE_CITES_NONE)
                .about(case(c))
                .about(harness(h)),
            SimShapeProblem::ClauseForeign {
                harness: _,
                case: c,
                clause,
            } => {
                let cited = Target::Clause(*clause);
                Violation::of(SIM_CLAUSE_FOREIGN)
                    .about(case(c))
                    .about(Party::Target(cited))
                    .with_remedy(
                        Remedy::PutAgainWithoutCitation {
                            case: Target::Case(*c),
                            cited,
                        },
                        Applicability::Review,
                    )
            }
            SimShapeProblem::BuildsNone { harness: h } => {
                Violation::of(SIM_BUILDS_NONE).about(harness(h))
            }
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
        ClauseId, ContextId, CruxId, CruxOptionId, InterfaceId, ItemId, test_uuid,
    };
    use crate::domain::vo::item_key::ItemKey;
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
    fn crux(n: u128) -> CruxId {
        CruxId::from_uuid(test_uuid(n))
    }

    /// The name of a variant; exhaustive, so a new variant is not forgotten here.
    fn cr_name(e: &ChangeRequestError) -> &'static str {
        match e {
            ChangeRequestError::NoOption => "NoOption",
            ChangeRequestError::DuplicateOption(_) => "DuplicateOption",
            ChangeRequestError::NotAnOption(_) => "NotAnOption",
            ChangeRequestError::UnspecifiedNotAllowed(_) => "UnspecifiedNotAllowed",
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
            ChangeRequestError::ExpectationConflict(_) => "ExpectationConflict",
            ChangeRequestError::RaisedInUnknown(_) => "RaisedInUnknown",
        }
    }

    fn cr_samples() -> Vec<ChangeRequestError> {
        vec![
            ChangeRequestError::NoOption,
            ChangeRequestError::DuplicateOption(choice()),
            ChangeRequestError::NotAnOption(CruxOptionId::from_uuid(test_uuid(8))),
            ChangeRequestError::UnspecifiedNotAllowed("model"),
            ChangeRequestError::AlreadyResolved,
            ChangeRequestError::Undecided,
            ChangeRequestError::AlreadyAnswered,
            ChangeRequestError::SupersedesUnknown(crux(2)),
            ChangeRequestError::SupersedesUnanswered(crux(2)),
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
            ChangeRequestError::ExpectationConflict(Target::Project),
            ChangeRequestError::RaisedInUnknown(cr(2)),
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
            ProjectError::SimShape(_) => "SimShape",
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
            ProjectError::SimShape(SimShapeProblem::BuildsNone {
                harness: HarnessId::from_uuid(test_uuid(400)),
            }),
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
            ContextError::Harness(HarnessError::BuildsOfTests(HarnessId::from_uuid(
                test_uuid(400),
            ))),
        ]
    }

    /// The variants of the domain's error enums that still give `domain.unmapped`, by `Enum::Variant`, with those
    /// of the errors they wrap that are themselves listed. The count can only go down: a variant that is mapped
    /// leaves this list, and a new variant that is not mapped has to be added to it.
    const STILL_UNMAPPED: &[&str] = &[
        "ChangeRequestError::DuplicateOption",
        "ChangeRequestError::ExpectationConflict",
        "ChangeRequestError::HistoryOutOfOrder",
        "ChangeRequestError::NoOption",
        "ChangeRequestError::NotResolved",
        "ChangeRequestError::PolicyRevisionMismatch",
        "ChangeRequestError::RaisedInUnknown",
        "ChangeRequestError::UnaskedExpectation",
        "ChangeRequestError::UnspecifiedNotAllowed",
        "ContextError::NoContract",
        "ContextError::UsesItself",
        "ContractError::DuplicateId",
        "ContractError::DuplicateLabel",
        "HarnessError::DuplicateId",
        "HarnessError::NoItem",
        "InterfaceError::DuplicateId",
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
                "InterfaceError::KeyTaken",
                InterfaceError::KeyTaken {
                    key: ItemKey::try_from("push").unwrap(),
                    id: ItemId::from_uuid(test_uuid(1)),
                }
                .violation(),
            ),
            (
                "InterfaceError::BlankItem",
                InterfaceError::BlankItem {
                    id: ItemId::from_uuid(test_uuid(1)),
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
                HarnessError::BuildsOfTests(HarnessId::from_uuid(test_uuid(1))).violation(),
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
        // The count only goes down: 18 + 16 + 8 + 2 + 2 + 5 + 4 variants of the enums with a mapping, of which
        // the wrappers (`Many`, `Context`, `Policy`, `Contract`, `Interface`, `Harness`) give their wrapped
        // error's.
        assert_eq!(gives().len(), 19 + 13 + 5 + 2 + 2 + 5 + 4 + 2 + 2);
        assert_eq!(STILL_UNMAPPED.len(), 26);
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
        let v = ChangeRequestError::NoOption.violation(cr(1));
        assert_eq!(*v.rule(), UNMAPPED);
        assert_eq!(
            v.values().get("error"),
            Some(&Value::Text("NoOption".to_string()))
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
            ChangeRequestError::SupersedesUnknown(crux(9))
                .violation(cr(1))
                .rule()
                .as_str(),
            "cr.supersedes-unknown"
        );
        let unanswered = ChangeRequestError::SupersedesUnanswered(crux(9)).violation(cr(1));
        assert_eq!(unanswered.rule().as_str(), "cr.supersedes-unanswered");
        assert_eq!(unanswered.subjects(), &[Party::ChangeRequest(cr(1))]);
        assert_eq!(
            ChangeRequestError::NotAnOption(CruxOptionId::from_uuid(test_uuid(8)))
                .violation(cr(1))
                .rule()
                .as_str(),
            "cr.crux-unknown-option"
        );
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
    fn each_problem_of_the_shape_of_a_sim_harness_is_a_violation_of_its_rule_about_what_it_names() {
        use crate::domain::entity::harness::CaseNameFault;

        let h = |n: u128| HarnessId::from_uuid(test_uuid(n));
        let c = |n: u128| CaseId::from_uuid(test_uuid(n));
        let k = ClauseId::from_uuid(test_uuid(30));
        let target = |t: Target| Party::Target(t);
        let problems = [
            SimShapeProblem::OnePerContext {
                context: ctx(1),
                harnesses: vec![h(400), h(401)],
            },
            SimShapeProblem::SweepOnce {
                harness: h(400),
                count: 2,
                extra: Some(c(41)),
            },
            SimShapeProblem::CaseName {
                harness: h(400),
                case: c(41),
                fault: CaseNameFault::RegressNamedTwice,
            },
            SimShapeProblem::CaseCitesNone {
                harness: h(400),
                case: c(41),
            },
            SimShapeProblem::ClauseForeign {
                harness: h(400),
                case: c(41),
                clause: k,
            },
            SimShapeProblem::BuildsNone { harness: h(400) },
        ];
        let seen: Vec<(String, Vec<Party>)> = problems
            .iter()
            .map(|p| {
                let v = p.violation();
                (v.rule().to_string(), v.subjects().to_vec())
            })
            .collect();
        let want = |rule: &str, subjects: Vec<Party>| (rule.to_string(), subjects);
        assert_eq!(
            seen,
            [
                want(
                    "sim.one-per-context",
                    vec![
                        target(Target::Context(ctx(1))),
                        target(Target::Harness(h(400))),
                        target(Target::Harness(h(401)))
                    ]
                ),
                want(
                    "sim.sweep-once",
                    vec![target(Target::Harness(h(400))), target(Target::Case(c(41)))]
                ),
                want(
                    "sim.case-name",
                    vec![target(Target::Case(c(41))), target(Target::Harness(h(400)))]
                ),
                want(
                    "sim.case-cites-none",
                    vec![target(Target::Case(c(41))), target(Target::Harness(h(400)))]
                ),
                want(
                    "sim.clause-foreign",
                    vec![target(Target::Case(c(41))), target(Target::Clause(k))]
                ),
                want("sim.builds-none", vec![target(Target::Harness(h(400)))]),
            ]
        );
        assert_eq!(
            problems[1].violation().values().get("count"),
            Some(&Value::Count(2))
        );
        assert_eq!(
            problems[2].violation().values().get("reason"),
            Some(&Value::Word("regress-named-twice"))
        );
        // a Clause outside the Context is mended by putting the Harness again without the citation
        assert_eq!(
            problems[4].violation().remedies()[0].0,
            Remedy::PutAgainWithoutCitation {
                case: Target::Case(c(41)),
                cited: Target::Clause(k)
            }
        );
        // the two refusals of a Harness's own content name the Case or the Harness they are about
        assert_eq!(
            HarnessError::SimCitesItem(c(41)).violation().subjects(),
            &[target(Target::Case(c(41)))]
        );
        assert_eq!(
            HarnessError::BuildsOfTests(h(400))
                .violation()
                .rule()
                .as_str(),
            "sim.builds-of-tests"
        );
        // every rule the problems give carries only the value keys it lists
        for p in &problems {
            let v = p.violation();
            let listed = rule(v.rule().as_str()).expect("registered").values;
            assert!(v.values().keys().all(|key| listed.contains(key)));
        }
    }

    #[test]
    fn the_refusals_of_an_item_name_the_item() {
        let item = ItemId::from_uuid(test_uuid(7));
        let taken = InterfaceError::KeyTaken {
            key: ItemKey::try_from("push").unwrap(),
            id: item,
        }
        .violation();
        assert_eq!(taken.rule().as_str(), "interface.key-taken");
        assert_eq!(taken.subjects(), &[Party::Target(Target::Item(item))]);
        let blank = InterfaceError::BlankItem { id: item }.violation();
        assert_eq!(blank.rule().as_str(), "interface.blank-item");
        assert_eq!(blank.subjects(), &[Party::Target(Target::Item(item))]);
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
