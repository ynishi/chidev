//! A violation: an operation refused, or a state found, as structure. The domain says which rule was broken
//! ([`RuleId`]), what it was broken about (by id), the values that tell how (typed, never a sentence), and what
//! would mend it ([`Remedy`], each with how far a machine may trust it). The sentence, the names and the
//! operations a remedy comes to are the application's: the domain holds no name the store keeps, and no prose
//! beyond the registry's.
//!
//! Each error enum of the domain keeps its variants and gives the violation of a variant (see [`map`]); a variant
//! that has no rule yet gives [`UNMAPPED`], carrying its debug text, so that a
//! refusal is never lost and the ones still to be mapped are counted by a test.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::domain::rule::UNMAPPED;
use crate::domain::vo::choice::Choice;
use crate::domain::vo::finding::RuleId;
use crate::domain::vo::id::{ChangeRequestId, ContextId, CruxId, MeasurementId};
use crate::domain::vo::policy::PolicyLineId;
use crate::domain::vo::revision::Revision;
use crate::domain::vo::target::Target;

pub mod map;

/// How far a machine may trust a remedy (as rustc's `Applicability`): an agent that mends its own input reads this
/// to decide whether to apply a suggestion as it stands or to look first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Applicability {
    /// It mends the violation and changes nothing else: apply it as it stands.
    Machine,
    /// It is likely what is meant, but it changes the model: look at it before applying.
    Review,
    /// It has a hole (`<..>`) that a person or an agent fills in.
    Placeholder,
}

impl Applicability {
    /// The word for it, as the output writes it.
    pub fn word(self) -> &'static str {
        match self {
            Applicability::Machine => "machine",
            Applicability::Review => "review",
            Applicability::Placeholder => "placeholder",
        }
    }
}

/// Something a violation is about, by id. The domain has no names; the application looks them up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Party {
    /// A thing of the model: a Context, an asset, or a definition inside one.
    Target(Target),
    /// A change request.
    ChangeRequest(ChangeRequestId),
    /// A Crux, named by its id: the id is how an operation addresses it.
    Crux(CruxId),
    /// A measurement, which has no name: it is shown by its id.
    Measurement(MeasurementId),
    /// A line of the decision Policy.
    PolicyLine(PolicyLineId),
    /// An asset or Context by the uuid alone, when the error holds no more (the application finds its name).
    Id(Uuid),
}

/// A value of a violation, typed. No sentence: a count, a word of a closed set, a revision, the choice of an
/// option, the things it lists.
///
/// A violation says what it is about in two ways, and each value is for one of them. A subject
/// ([`Violation::subjects`]) is what the violation is about, in order, and is what the sentence of the rule names
/// (`{s0}`, `{s1}`). A value is a typed attribute that a program reads by its key: a [`Value::Party`] is one thing
/// that plays a part the key names (the target an option asks twice), and [`Value::Parties`] is a list of them.
/// Both are looked up by the application, which names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// How many.
    Count(usize),
    /// A word of a closed set (`item`, `clause`).
    Word(&'static str),
    /// A revision.
    Revision(Revision),
    /// The choice of an option, as it was given: a short label, a value of the Crux, not a sentence.
    Choice(Choice),
    /// One thing, by id.
    Party(Party),
    /// Things, by id.
    Parties(Vec<Party>),
    /// A text the rule's value is: the debug text of an error that has no rule yet ([`UNMAPPED`] carries it), or a
    /// word of a record that a rule names (the field a correction was refused for), never a text of a record.
    Text(String),
}

/// What would mend a violation, in the domain's words. The application turns it into the operation to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    /// Open the change request the operation names but does not exist yet.
    OpenChangeRequest,
    /// Resolve these parts first.
    ResolveParts {
        /// The change requests whose parent it is and that are open.
        parts: Vec<ChangeRequestId>,
    },
    /// Answer this crux of this change request.
    AnswerCrux {
        /// The change request.
        change_request: ChangeRequestId,
        /// The crux that has no decision.
        crux: CruxId,
    },
    /// Reopen this change request, if the work goes on.
    ReopenChangeRequest {
        /// The change request.
        change_request: ChangeRequestId,
    },
    /// Raise a new crux that supersedes the decision that stands for this one.
    SupersedeCrux {
        /// The change request the refused operation was on. The domain cannot know which crux stands for the
        /// question, so this is not where the new crux is raised: the application raises it in the change request
        /// the crux that stands was raised in, which it may reopen first.
        change_request: ChangeRequestId,
        /// The crux whose decision is to be replaced.
        crux: CruxId,
    },
    /// Open a change request for the targets the answer adds outside its change request, and decide the crux again
    /// naming it, or decide it declining them.
    TakeOutside {
        /// The crux being decided.
        crux: CruxId,
        /// The targets outside the change request, in order.
        targets: Vec<Target>,
    },
    /// Put this line in the decision Policy.
    PutPolicyLine {
        /// The line.
        line: PolicyLineId,
    },
    /// Retire what refers to the target first (the Harnesses of the Cases named), in the same change set.
    RetireReferrersFirst {
        /// The things that refer, in order: Cases, which are retired with their Harness.
        referrers: Vec<Target>,
    },
    /// Change the uses of these Contexts, or retire them, so that they no longer use the Context.
    ChangeUsesOf {
        /// The Contexts that use it.
        contexts: Vec<ContextId>,
    },
    /// Put the Harness of the Case again without the citation.
    PutAgainWithoutCitation {
        /// The Case.
        case: Target,
        /// What it cites.
        cited: Target,
    },
    /// Keep what is cited (put it in the same change set), so that the citation reaches.
    KeepCited {
        /// The Item or Clause.
        cited: Target,
    },
    /// Name the thing once in the change set.
    NameOnce {
        /// The thing named twice.
        id: Uuid,
    },
    /// Put another Contract in the same change set.
    PutAnotherContract,
    /// Put a Harness whose Case cites an Item of the Interface, then pin again.
    CheckTheInterface {
        /// The Interface.
        interface: Target,
    },
    /// Run `init`.
    InitProject,
    /// Add the Context.
    AddContext,
}

/// A rule broken, as structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    rule: RuleId,
    subjects: Vec<Party>,
    values: BTreeMap<&'static str, Value>,
    remedies: Vec<(Remedy, Applicability)>,
}

impl Violation {
    /// A violation of `rule` about nothing yet.
    pub fn of(rule: RuleId) -> Violation {
        Violation {
            rule,
            subjects: Vec::new(),
            values: BTreeMap::new(),
            remedies: Vec::new(),
        }
    }

    /// A refusal that no rule is tied to yet: [`UNMAPPED`], carrying the debug text of `error` under `error` and
    /// its display text (kept for debugging, and so that the refusal is still readable) under `text`.
    pub fn unmapped(error: &(impl std::fmt::Debug + std::fmt::Display)) -> Violation {
        Violation::of(UNMAPPED)
            .with_value("error", Value::Text(format!("{error:?}")))
            .with_value("text", Value::Text(error.to_string()))
    }

    /// The same, about `subject` (the next subject, after the ones it has).
    pub fn about(mut self, subject: Party) -> Violation {
        self.subjects.push(subject);
        self
    }

    /// The same, with a subject put first: the thing the operation was on, which the error did not hold.
    pub fn on(mut self, subject: Party) -> Violation {
        self.subjects.insert(0, subject);
        self
    }

    /// The same, with a value.
    pub fn with_value(mut self, key: &'static str, value: Value) -> Violation {
        self.values.insert(key, value);
        self
    }

    /// The same, with a remedy.
    pub fn with_remedy(mut self, remedy: Remedy, how: Applicability) -> Violation {
        self.remedies.push((remedy, how));
        self
    }

    /// The rule broken.
    pub fn rule(&self) -> &RuleId {
        &self.rule
    }

    /// What it is about, by id, in order.
    pub fn subjects(&self) -> &[Party] {
        &self.subjects
    }

    /// The values, by key.
    pub fn values(&self) -> &BTreeMap<&'static str, Value> {
        &self.values
    }

    /// What would mend it, most likely first.
    pub fn remedies(&self) -> &[(Remedy, Applicability)] {
        &self.remedies
    }

    /// Everything it names by id: the subjects, the things its values list, and the things its remedies act on,
    /// so that the application can look every one of them up once.
    pub fn parties(&self) -> Vec<Party> {
        let mut out = self.subjects.clone();
        for v in self.values.values() {
            match v {
                Value::Party(party) => out.push(*party),
                Value::Parties(parties) => out.extend(parties.iter().copied()),
                Value::Count(_)
                | Value::Word(_)
                | Value::Revision(_)
                | Value::Choice(_)
                | Value::Text(_) => {}
            }
        }
        for (r, _) in &self.remedies {
            match r {
                Remedy::ResolveParts { parts } => {
                    out.extend(parts.iter().map(|c| Party::ChangeRequest(*c)));
                }
                Remedy::AnswerCrux { change_request, .. }
                | Remedy::ReopenChangeRequest { change_request }
                | Remedy::SupersedeCrux { change_request, .. } => {
                    out.push(Party::ChangeRequest(*change_request));
                }
                Remedy::TakeOutside { crux, targets } => {
                    out.push(Party::Crux(*crux));
                    out.extend(targets.iter().map(|t| Party::Target(*t)));
                }
                Remedy::PutPolicyLine { line } => out.push(Party::PolicyLine(*line)),
                Remedy::RetireReferrersFirst { referrers } => {
                    out.extend(referrers.iter().map(|t| Party::Target(*t)));
                }
                Remedy::ChangeUsesOf { contexts } => {
                    out.extend(contexts.iter().map(|c| Party::Target(Target::Context(*c))));
                }
                Remedy::PutAgainWithoutCitation { case, cited } => {
                    out.push(Party::Target(*case));
                    out.push(Party::Target(*cited));
                }
                Remedy::KeepCited { cited } => out.push(Party::Target(*cited)),
                Remedy::CheckTheInterface { interface } => out.push(Party::Target(*interface)),
                Remedy::NameOnce { id } => out.push(Party::Id(*id)),
                Remedy::OpenChangeRequest
                | Remedy::PutAnotherContract
                | Remedy::InitProject
                | Remedy::AddContext => {}
            }
        }
        out.sort();
        out.dedup();
        out
    }
}
