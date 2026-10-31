//! Checks on the copies of the placed assets in the working tree: whether the file that holds the code of an
//! Interface, or the source of a Harness, is still the text the model holds, and whether it can be looked at at all.
//! They read the copies the view gives ([`CheckView::tree`]) and look at no file: the use case that builds the view
//! reads them, once, and an application that reads none (or whose project lays nothing out) gives a view in which
//! every asset is unplaced, which these checks say nothing about.
//!
//! They run at the `check` moment only. A copy is not something an edit or a confirmation claims (a Pin accepts code,
//! it does not accept a file), and reading the files at every pin would cost the files of the whole project each
//! time; see ADR 0010.

use std::borrow::Cow;

use crate::domain::check::descriptor::{Descriptor, Executor};
use crate::domain::check::level::{Level, Moment};
use crate::domain::check::policy::Params;
use crate::domain::check::view::CheckView;
use crate::domain::check::{Check, Finding};
use crate::domain::entity::context::Context;
use crate::domain::entity::harness::Harness;
use crate::domain::vo::finding::{RuleId, Subject};
use crate::domain::vo::tree::{CodeNow, FileNow};

/// The family of the ids of these checks, as a policy writes it (`tree.`).
pub const FAMILY: &str = "tree.";

/// The subject of a finding about a Harness as a whole: its first Case (the Context when it has none), as the
/// checks of the examples and of the simulations do, for a finding has no Harness to be about.
fn subject_of(context: &Context, harness: &Harness) -> Subject {
    harness
        .cases()
        .first()
        .map_or(Subject::Context(context.id()), |c| Subject::Case(c.id()))
}

/// The Harnesses of the Project that hold a source (so a file can hold a copy of it), with the Context of each.
fn sourced<'a>(view: &'a CheckView<'a>) -> impl Iterator<Item = (&'a Context, &'a Harness)> + 'a {
    view.project
        .contexts()
        .flat_map(|c| c.harnesses().map(move |h| (c, h)))
        .filter(|(_, h)| h.source().is_some())
}

/// An Interface whose copy in the working tree is other text than the code its Pin accepted.
#[derive(Debug, Clone, Copy)]
pub struct CodeMoved;

static CODE_MOVED: Descriptor = Descriptor {
    id: RuleId::from_static("tree.code-moved"),
    summary: Cow::Borrowed(
        "an interface whose file in the working tree holds other code than the code its pin accepted",
    ),
    help: Cow::Borrowed(
        "The file the project's layout places the interface's code in was changed after the code was pinned, or the \
           code was pinned from somewhere else. The pin, and every run that stands on it, describe code that is no \
           longer in the file. Pin the file's code again once the harness passes against it, or restore the file.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for CodeMoved {
    fn descriptor(&self) -> &'static Descriptor {
        &CODE_MOVED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        view.project
            .contexts()
            .flat_map(|c| c.interfaces())
            .filter(|i| matches!(view.tree.interface(i.id()), CodeNow::Moved { .. }))
            .map(|i| {
                Finding::new(
                    &CODE_MOVED,
                    Some(Subject::Interface(i.id())),
                    "the file that holds this interface's code in the working tree differs from the code pinned for it",
                )
            })
            .collect()
    }
}

/// A Harness whose copy in the working tree is other text than the source the model holds.
#[derive(Debug, Clone, Copy)]
pub struct HarnessMoved;

static HARNESS_MOVED: Descriptor = Descriptor {
    id: RuleId::from_static("tree.harness-moved"),
    summary: Cow::Borrowed(
        "a harness (about its first case) whose file in the working tree holds other source than the harness does",
    ),
    help: Cow::Borrowed(
        "The file the project's layout places the harness's source in differs from the source the model holds, so \
           the tests the project runs are not the ones the cases were written against. Put the harness again from \
           the file if the file is right, or restore the file.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

impl Check for HarnessMoved {
    fn descriptor(&self) -> &'static Descriptor {
        &HARNESS_MOVED
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        sourced(view)
            .filter(|(_, h)| view.tree.harness(h.id()) == FileNow::Moved)
            .map(|(c, h)| {
                Finding::new(
                    &HARNESS_MOVED,
                    Some(subject_of(c, h)),
                    "the file that holds the source of this case's harness in the working tree differs from the source the model holds",
                )
            })
            .collect()
    }
}

/// A placed Interface or Harness whose file in the working tree is not there or cannot be read as text.
#[derive(Debug, Clone, Copy)]
pub struct Unread;

static UNREAD: Descriptor = Descriptor {
    id: RuleId::from_static("tree.unread"),
    summary: Cow::Borrowed(
        "an interface or a harness (about its first case) whose file in the working tree is missing or cannot be read as text",
    ),
    help: Cow::Borrowed(
        "The project's layout places the asset in a file that cannot be looked at, so nothing is known of whether the \
           file still holds what the model does. Make the file readable, or correct the layout.",
    ),
    moments: Cow::Borrowed(&[Moment::Check]),
    default_level: Level::Warning,
    mandatory: false,
    params: Cow::Borrowed(&[]),
    executor: Executor::Builtin,
};

/// Why a file cannot be looked at.
enum Gap {
    /// It is laid out and not there.
    Missing,
    /// It is there and cannot be read as text, for this reason.
    Unreadable(String),
}

impl Gap {
    /// The gap in a copy of the code of an Interface; none for a copy that can be looked at.
    fn of_code(now: &CodeNow) -> Option<Gap> {
        match now {
            CodeNow::Missing => Some(Gap::Missing),
            CodeNow::Unreadable(why) => Some(Gap::Unreadable(why.clone())),
            CodeNow::Unplaced | CodeNow::Unpinned | CodeNow::Same | CodeNow::Moved { .. } => None,
        }
    }

    /// The gap in a copy of the source of a Harness; none for a copy that can be looked at.
    fn of_source(now: &FileNow) -> Option<Gap> {
        match now {
            FileNow::Missing => Some(Gap::Missing),
            FileNow::Unreadable(why) => Some(Gap::Unreadable(why.clone())),
            FileNow::Unplaced | FileNow::Same | FileNow::Moved => None,
        }
    }

    /// The gap in a sentence about the file that holds `what`.
    fn said(&self, what: &str) -> String {
        match self {
            Gap::Missing => format!("the file that holds {what} in the working tree is missing"),
            Gap::Unreadable(why) => {
                format!("the file that holds {what} in the working tree cannot be read: {why}")
            }
        }
    }
}

impl Check for Unread {
    fn descriptor(&self) -> &'static Descriptor {
        &UNREAD
    }

    fn run(&self, view: &CheckView<'_>, _params: &Params) -> Vec<Finding> {
        let interfaces = view
            .project
            .contexts()
            .flat_map(|c| c.interfaces())
            .filter_map(|i| {
                let gap = Gap::of_code(&view.tree.interface(i.id()))?;
                Some(Finding::new(
                    &UNREAD,
                    Some(Subject::Interface(i.id())),
                    gap.said("this interface's code"),
                ))
            });
        let harnesses = sourced(view).filter_map(|(c, h)| {
            let gap = Gap::of_source(&view.tree.harness(h.id()))?;
            Some(Finding::new(
                &UNREAD,
                Some(subject_of(c, h)),
                gap.said("the source of this case's harness"),
            ))
        });
        interfaces.chain(harnesses).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::fixture::*;
    use super::*;
    use crate::domain::vo::digest::Digest;
    use crate::domain::vo::id::{HarnessId, test_uuid};
    use crate::domain::vo::tree::{CopyRead, DeclaredNow, TreeNow};

    fn harness_id() -> HarnessId {
        HarnessId::from_uuid(test_uuid(400))
    }

    /// What `check` finds in the fixture Project when the working tree is `tree`.
    fn found(check: &dyn Check, tree: &TreeNow) -> Vec<Finding> {
        let p = project();
        check.run(
            &CheckView {
                project: &p,
                pins: &[],
                change_requests: &[],
                cruxes: &[],
                measurements: &[],
                measure_settings: &BTreeMap::new(),
                sim_settings: &BTreeMap::new(),
                examples_settings: &BTreeMap::new(),
                tree,
                resolving: None,
            },
            &Params::default(),
        )
    }

    fn code(now: CodeNow) -> TreeNow {
        TreeNow::new(BTreeMap::from([(iface(), now)]), BTreeMap::new())
    }

    fn source(now: FileNow) -> TreeNow {
        TreeNow::new(BTreeMap::new(), BTreeMap::from([(harness_id(), now)]))
    }

    fn all(tree: &TreeNow) -> Vec<(String, Option<Subject>, String)> {
        [&CodeMoved as &dyn Check, &HarnessMoved, &Unread]
            .into_iter()
            .flat_map(|c| found(c, tree))
            .map(|f| (f.rule().to_string(), f.subject(), f.message().to_string()))
            .collect()
    }

    #[test]
    fn a_copy_that_moved_is_found_by_the_check_of_its_kind_and_by_that_one() {
        let moved = CodeNow::Moved {
            declared: DeclaredNow::Off,
        };
        let found_code = found(&CodeMoved, &code(moved.clone()));
        assert_eq!(found_code.len(), 1);
        assert_eq!(found_code[0].subject(), Some(Subject::Interface(iface())));
        assert_eq!(found_code[0].level(), Level::Warning);
        assert!(found(&HarnessMoved, &code(moved.clone())).is_empty());
        assert!(found(&Unread, &code(moved)).is_empty());

        let found_source = found(&HarnessMoved, &source(FileNow::Moved));
        assert_eq!(found_source.len(), 1);
        // a Harness is not a subject: the finding is about its first Case
        assert_eq!(
            found_source[0].subject(),
            Some(Subject::Case(crate::domain::vo::id::CaseId::from_uuid(
                test_uuid(40)
            )))
        );
        assert!(found(&CodeMoved, &source(FileNow::Moved)).is_empty());
        assert!(found(&Unread, &source(FileNow::Moved)).is_empty());
    }

    #[test]
    fn a_copy_that_is_missing_or_cannot_be_read_is_unread_for_an_interface_and_a_harness() {
        let by_interface = all(&code(CodeNow::Missing));
        assert_eq!(by_interface.len(), 1, "{by_interface:?}");
        assert_eq!(by_interface[0].0, "tree.unread");
        assert_eq!(by_interface[0].1, Some(Subject::Interface(iface())));
        assert!(
            by_interface[0].2.ends_with("is missing"),
            "{by_interface:?}"
        );

        let by_interface = all(&code(CodeNow::Unreadable(
            "src/a.rs: not UTF-8 text".into(),
        )));
        assert_eq!(by_interface.len(), 1, "{by_interface:?}");
        assert!(
            by_interface[0]
                .2
                .ends_with("cannot be read: src/a.rs: not UTF-8 text"),
            "{by_interface:?}"
        );

        let by_harness = all(&source(FileNow::Missing));
        assert_eq!(by_harness.len(), 1, "{by_harness:?}");
        assert_eq!(by_harness[0].0, "tree.unread");
        assert!(matches!(by_harness[0].1, Some(Subject::Case(_))));
        let by_harness = all(&source(FileNow::Unreadable("why".into())));
        assert_eq!(by_harness.len(), 1, "{by_harness:?}");
        assert!(by_harness[0].2.ends_with("cannot be read: why"));
    }

    #[test]
    fn a_copy_that_is_the_same_unpinned_or_unplaced_is_found_by_none() {
        for tree in [
            TreeNow::empty(),
            code(CodeNow::Same),
            code(CodeNow::Unpinned),
            code(CodeNow::Unplaced),
            source(FileNow::Same),
            source(FileNow::Unplaced),
        ] {
            assert!(all(&tree).is_empty(), "{tree:?}");
        }
    }

    #[test]
    fn what_a_port_read_of_a_missing_file_is_found_whether_or_not_the_interface_is_pinned() {
        use crate::domain::vo::tree::Observed;
        for pin in [None, Some(Digest::of(b"code"))] {
            let Observed::Settled(now) = CodeNow::observe(pin, CopyRead::Missing) else {
                panic!("a missing file is settled");
            };
            let found = all(&code(now));
            assert_eq!(found.len(), 1, "{found:?}");
            assert_eq!(found[0].0, "tree.unread");
        }
    }

    #[test]
    fn the_checks_run_at_check_alone_and_are_warnings() {
        for check in [&CodeMoved as &dyn Check, &HarnessMoved, &Unread] {
            let d = check.descriptor();
            assert_eq!(d.moments.as_ref(), [Moment::Check]);
            assert_eq!(d.default_level, Level::Warning);
            assert!(d.id.as_str().starts_with(FAMILY));
        }
    }
}
