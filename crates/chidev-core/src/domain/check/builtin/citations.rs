//! Checks on the citations of the Cases: behind their targets, or missing.

use std::borrow::Cow;

use std::collections::{BTreeMap, BTreeSet};

use super::ALL_HOOKS;
use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Hook, Level};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::service::cite::stale_cites;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::id::{ClauseId, ItemId};

/// A Case cites a definition that moved past the revision the Case recorded.
#[derive(Debug, Clone, Copy)]
pub struct StaleCite;

static STALE_CITE: Descriptor = Descriptor {
    id: RuleId::from_static("cite.stale"),
    summary: Cow::Borrowed(
        "a case cites an item or clause that changed after the case was written",
    ),
    help: Cow::Borrowed(
        "The case was written against an older definition and may no longer check what it says. \
           Bring the case up to date (put the harness again) or, if it still holds, put it to record the new revision.",
    ),
    hooks: Cow::Borrowed(&[Hook::Put, Hook::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for StaleCite {
    fn descriptor(&self) -> &'static Descriptor {
        &STALE_CITE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        // One finding per Case, so that the same Case is the same finding however many of its citations moved.
        let mut by_case = BTreeMap::new();
        for s in stale_cites(view.after) {
            let what = s.target.word();
            by_case
                .entry(s.case)
                .or_insert_with(Vec::new)
                .push(format!("{what} {} -> {}", s.cited, s.now));
        }
        by_case
            .into_iter()
            .map(|(case, moved)| {
                Finding::new(
                    &STALE_CITE,
                    Some(Subject::Case(case)),
                    format!("cites a definition that moved on ({})", moved.join(", ")),
                )
            })
            .collect()
    }
}

/// A Clause no Case cites.
#[derive(Debug, Clone, Copy)]
pub struct UncitedClause;

static UNCITED_CLAUSE: Descriptor = Descriptor {
    id: RuleId::from_static("cite.uncited-clause"),
    summary: Cow::Borrowed("a clause of a contract that no case cites"),
    help: Cow::Borrowed(
        "Nothing checks the promise this clause makes. Add a case that cites it, or open a change request that \
           addresses the finding if the clause is meant to go unchecked for now.",
    ),
    hooks: Cow::Borrowed(ALL_HOOKS),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for UncitedClause {
    fn descriptor(&self) -> &'static Descriptor {
        &UNCITED_CLAUSE
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let cited: BTreeSet<ClauseId> = view
            .after
            .contexts()
            .flat_map(|c| c.harnesses())
            .flat_map(|h| h.cases())
            .flat_map(|k| k.cites().clauses().keys().copied())
            .collect();
        view.after
            .contexts()
            .flat_map(|c| c.contracts())
            .flat_map(|k| k.clauses())
            .filter(|cl| !cited.contains(&cl.id()))
            .map(|cl| {
                Finding::new(
                    &UNCITED_CLAUSE,
                    Some(Subject::Clause(cl.id())),
                    format!("no case cites clause {}", cl.content().label()),
                )
            })
            .collect()
    }
}

/// An Item no Case cites.
#[derive(Debug, Clone, Copy)]
pub struct UncitedItem;

static UNCITED_ITEM: Descriptor = Descriptor {
    id: RuleId::from_static("cite.uncited-item"),
    summary: Cow::Borrowed("an item of an interface that no case cites"),
    help: Cow::Borrowed(
        "Nothing checks this declaration. Add a case that cites it, or open a change request that addresses the \
           finding if it is meant to go unchecked for now.",
    ),
    hooks: Cow::Borrowed(ALL_HOOKS),
    default_level: Level::Warning,
    mandatory: false,
    params: std::borrow::Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for UncitedItem {
    fn descriptor(&self) -> &'static Descriptor {
        &UNCITED_ITEM
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let cited: BTreeSet<ItemId> = view
            .after
            .contexts()
            .flat_map(|c| c.harnesses())
            .flat_map(|h| h.cases())
            .flat_map(|k| k.cites().items().keys().copied())
            .collect();
        view.after
            .contexts()
            .flat_map(|c| c.interfaces())
            .flat_map(|i| i.items())
            .filter(|it| !cited.contains(&it.id()))
            .map(|it| {
                Finding::new(
                    &UNCITED_ITEM,
                    Some(Subject::Item(it.id())),
                    format!("no case cites `{}`", it.key()),
                )
            })
            .collect()
    }
}
