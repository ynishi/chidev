//! The ChangeSet: one transition of a Project, made of several assets put or retired together.

use std::collections::BTreeSet;

use super::case::CaseName;
use super::clause::{ClauseContent, ClauseLabel};
use super::clause_edit::ClauseEdit;
use super::example::Example;
use super::frame::Frame;
use super::harness_kind::CodeKind;
use super::id::{CaseId, ClauseId, ContextId, ContractId, HarnessId, InterfaceId, ItemId};
use super::line_range::LineRange;
use super::source::Source;
use super::title::Title;
use crate::domain::entity::context::Context;
use crate::domain::entity::interface::ItemFound;

/// A Case to put, as a Harness is written: what it cites is given by id alone. The revision a citation is
/// recorded at is the one its target has once the Contracts and Interfaces of the same change set are in, so a Case
/// can cite a Clause or an Item that the change set itself makes or revises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseDraft {
    id: CaseId,
    name: CaseName,
    lines: LineRange,
    items: BTreeSet<ItemId>,
    clauses: BTreeSet<ClauseId>,
}

impl CaseDraft {
    /// A Case citing `items` and `clauses`. Whether the Harness takes a Case with these (a Case of tests cites an
    /// Item, a Case of a sim none) is the Harness's rule, found when the change set is applied.
    pub fn new(
        id: CaseId,
        name: CaseName,
        lines: LineRange,
        items: BTreeSet<ItemId>,
        clauses: BTreeSet<ClauseId>,
    ) -> CaseDraft {
        CaseDraft {
            id,
            name,
            lines,
            items,
            clauses,
        }
    }

    /// Its id.
    pub fn id(&self) -> CaseId {
        self.id
    }

    /// Its name.
    pub fn name(&self) -> &CaseName {
        &self.name
    }

    /// The lines of the code it is on.
    pub fn lines(&self) -> LineRange {
        self.lines
    }

    /// The Items it cites.
    pub fn items(&self) -> &BTreeSet<ItemId> {
        &self.items
    }

    /// The Clauses it cites.
    pub fn clauses(&self) -> &BTreeSet<ClauseId> {
        &self.clauses
    }
}

/// An example Case to put, as a Harness of examples is written: its example, and the Clauses it cites by id alone
/// (an example cites no Item, so it cannot even be drafted with one). The revision a citation is recorded at is
/// the one its target has once the Contracts of the same change set are in, as for a [`CaseDraft`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleDraft {
    id: CaseId,
    name: CaseName,
    example: Example,
    clauses: BTreeSet<ClauseId>,
}

impl ExampleDraft {
    /// A Case that holds `example` and cites `clauses`. Whether the Harness takes the Case (it cites a Clause, its
    /// name is one an example has) is the Harness's rule, found when the change set is applied.
    pub fn new(
        id: CaseId,
        name: CaseName,
        example: Example,
        clauses: BTreeSet<ClauseId>,
    ) -> ExampleDraft {
        ExampleDraft {
            id,
            name,
            example,
            clauses,
        }
    }

    /// Its id.
    pub fn id(&self) -> CaseId {
        self.id
    }

    /// Its name.
    pub fn name(&self) -> &CaseName {
        &self.name
    }

    /// The example it holds.
    pub fn example(&self) -> &Example {
        &self.example
    }

    /// The Clauses it cites.
    pub fn clauses(&self) -> &BTreeSet<ClauseId> {
        &self.clauses
    }
}

/// What a put of a Harness says it holds. The kind is inside the content, so the input cannot say examples and
/// give code, or the other way round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessDraft {
    /// Code, and the Cases in it.
    Code {
        /// The kind of the code; none: the kind the Harness has, or tests for a new one. A Harness that holds
        /// examples must be told.
        kind: Option<CodeKind>,
        /// Its code.
        source: Source,
        /// Its Cases.
        cases: Vec<CaseDraft>,
    },
    /// Examples, one for each Case.
    Examples(Vec<ExampleDraft>),
}

/// Something a change set puts: a setting of a Context, or the whole new state of an asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetPut {
    /// A Context is added (the Project goes to its next revision). It is applied first, so the same change set can
    /// put assets in it and have them cite into it.
    Context(Context),
    /// The Contexts a Context uses.
    Uses {
        /// The Context.
        context: ContextId,
        /// What it uses.
        uses: BTreeSet<ContextId>,
    },
    /// A Context is based on its Contracts (it has one at least once the Contracts of the change set are in).
    BaseOnContract {
        /// The Context.
        context: ContextId,
    },
    /// A Contract is made or revised.
    Contract {
        /// The Context.
        context: ContextId,
        /// The Contract.
        id: ContractId,
        /// Its title.
        title: Title,
        /// Its Clauses.
        clauses: Vec<(ClauseId, ClauseContent)>,
    },
    /// Some Clauses of a Contract that is: the Clauses not named stay in their order, and so does the title. A
    /// Clause an edit names (by its label, or the one it replaces) is set in its place, one in `remove` is gone, and
    /// a new one comes after the others, in the order given. The Contract composes the list
    /// ([`Contract::clauses_partial`](crate::domain::entity::contract::Contract::clauses_partial)) and the Project
    /// puts it as it puts a Contract: this is not a second way into the Contract. It is a put of the Contract, so
    /// the set names it once: with a `Contract` put of the same id, or another `Clauses` put, it is refused.
    Clauses {
        /// The Context.
        context: ContextId,
        /// The Contract, which the Project has before the set.
        id: ContractId,
        /// The Clauses to set. The id of an edit is the one a new Clause takes.
        edits: Vec<ClauseEdit>,
        /// The labels of the Clauses to remove.
        remove: Vec<ClauseLabel>,
    },
    /// An Interface is made or revised.
    Interface {
        /// The Context.
        context: ContextId,
        /// The Interface.
        id: InterfaceId,
        /// Its code.
        source: Source,
        /// Its Items.
        items: Vec<ItemFound>,
    },
    /// A Harness is made or revised.
    Harness {
        /// The Context.
        context: ContextId,
        /// The Harness.
        id: HarnessId,
        /// The Interfaces whose pinned code a sim Harness or a Harness of examples builds (none for tests).
        builds: BTreeSet<InterfaceId>,
        /// What it holds: code or examples, and its Cases.
        content: HarnessDraft,
    },
}

/// Something a change set retires, with what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRetire {
    /// A Context and everything it owns.
    Context(ContextId),
    /// A Contract of a Context.
    Contract {
        /// The Context.
        context: ContextId,
        /// The Contract.
        id: ContractId,
    },
    /// An Interface of a Context.
    Interface {
        /// The Context.
        context: ContextId,
        /// The Interface.
        id: InterfaceId,
    },
    /// A Harness of a Context.
    Harness {
        /// The Context.
        context: ContextId,
        /// The Harness.
        id: HarnessId,
    },
}

/// A transition of the Project that is coherent only as a whole: the rules of the Project are checked once, on
/// the state it ends in, so a Case can stop citing a Clause in the same change that removes the Clause, and a new
/// Harness can cite what the same change makes. The order in which the entries are listed does not matter; they are
/// applied in the order the rules need (settings, Contracts and Interfaces, Harnesses, then retirements).
/// A thing is named once in a change set: putting it twice, or putting and retiring it, is refused.
///
/// A set applied for a change request carries that request's [`Frame`], which bounds which citations a Harness put
/// brings up: a Case is claimed current only for what the request is about. A set that belongs to no request carries
/// none, and every citation of a Harness it puts is brought up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// What is put.
    pub puts: Vec<AssetPut>,
    /// What is retired.
    pub retires: Vec<AssetRetire>,
    frame: Option<Frame>,
}

impl ChangeSet {
    /// A change set of `puts` and `retires`, for no change request.
    pub fn new(puts: Vec<AssetPut>, retires: Vec<AssetRetire>) -> ChangeSet {
        ChangeSet {
            puts,
            retires,
            frame: None,
        }
    }

    /// This change set as applied for a change request whose frame is `frame`.
    pub fn for_frame(mut self, frame: Frame) -> ChangeSet {
        self.frame = Some(frame);
        self
    }

    /// The frame of the change request it is applied for; none when it is applied for none.
    pub fn frame(&self) -> Option<&Frame> {
        self.frame.as_ref()
    }

    /// Whether it changes nothing because it names nothing.
    pub fn is_empty(&self) -> bool {
        self.puts.is_empty() && self.retires.is_empty()
    }
}
