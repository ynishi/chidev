//! What the checks of the `check` moment read besides the model: the measurements with the settings now, and the
//! copies of the placed assets in the working tree. Two paths build that view, `check` and `status` ([`App::check`],
//! [`App::status`]) and the question that keeps what the checks said ([`App::check_run`]); both ask here, so that
//! what one of them reads the others read, and a reading added to the moment is added once.

use std::collections::BTreeMap;

use super::measure::Measured;
use super::ports::external::ExternalExecutor;
use super::ports::tree::TreeTarget;
use super::ports::{IdSource, Store};
use super::{App, AppError};
use crate::domain::check::Moment;
use crate::domain::check::builtin::TREE_FAMILY;
use crate::domain::entity::pin::Pin;
use crate::domain::entity::project::Project;
use crate::domain::vo::digest::Digest;
use crate::domain::vo::id::{HarnessId, InterfaceId};
use crate::domain::vo::tree::{CodeNow, CopyRead, DeclaredNow, FileNow, Observed, TreeNow};

/// What the checks of the `check` moment read besides the model.
#[derive(Debug)]
pub(super) struct CheckInputs {
    pub(super) measured: Measured,
    /// The copies in the working tree; none when no check that reads them runs, and then nothing was looked at.
    pub(super) tree: Option<TreeNow>,
}

/// What is read of no copy: every asset is unplaced. What the moments that read no copy give the checks.
pub(super) static NO_TREE: TreeNow = TreeNow::empty();

impl CheckInputs {
    /// The copies the checks are given: none looked at is every asset unplaced, which no check of the tree says
    /// anything about.
    pub(super) fn tree(&self) -> &TreeNow {
        self.tree.as_ref().unwrap_or(&NO_TREE)
    }
}

/// What the model holds of an asset that has a copy, to compare the copy with.
enum Held {
    /// The digest of the code the Pin of the Interface accepted; none for an Interface with no Pin.
    Code(InterfaceId, Option<Digest>),
    /// The digest of the source of the Harness.
    Source(HarnessId, Digest),
}

/// An asset of the Project, by the names a person knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeAsset {
    /// The Context's name.
    pub context: String,
    /// The asset's name.
    pub name: String,
}

/// The assets that have no copy to look at, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeUnplaced {
    /// The Interfaces no layout places.
    pub interfaces: Vec<TreeAsset>,
    /// The Harnesses that hold a source and that no layout places.
    pub harnesses: Vec<TreeAsset>,
}

/// An Interface whose copy is other text than the code its Pin accepted, in the result of `status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeMoved {
    /// The Context's name.
    pub context: String,
    /// The Interface's name.
    pub interface: String,
    /// What is known of the Items the copy declares.
    pub declared: DeclaredNow,
}

/// Where the copies in the working tree stand, in the result of `status`: what has no copy to look at, what has a
/// copy and no Pin to compare it with, and what moved. A copy that is missing or cannot be read is said by
/// `tree.unread`, and a Harness whose copy moved by `tree.harness-moved`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeStatus {
    /// What no layout places.
    pub unplaced: TreeUnplaced,
    /// The Interfaces whose copy is there and that have no Pin.
    pub unpinned: Vec<TreeAsset>,
    /// The Interfaces whose copy moved.
    pub moved: Vec<TreeMoved>,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Everything the checks of the `check` moment read besides the model, for the Project `p`: the one place that
    /// collects it, for `check`, for `status` and for the question that keeps what the checks said.
    ///
    /// The working tree is looked at only when a check that reads it runs, and then once, whatever the number of
    /// assets: a copy is read by the port for all of them together. `pins` are the Pins of the state the checks
    /// run on, which the caller has read already; the copies of the code are compared with them. The settings and
    /// the copies are read at once, for neither waits on the other.
    pub(super) async fn check_inputs(
        &self,
        p: &Project,
        pins: &[Pin],
    ) -> Result<CheckInputs, AppError> {
        let tree = async {
            if self.registry.runs(TREE_FAMILY, Moment::Check) {
                self.tree_now(p, pins).await.map(Some)
            } else {
                Ok(None)
            }
        };
        let (measured, tree) = tokio::try_join!(self.measured(p), tree)?;
        Ok(CheckInputs { measured, tree })
    }

    /// The copies of the placed assets of `p`, whose Pins are `pins`: the working tree is asked about every
    /// Interface and every Harness that holds a source, once, and each answer is set against what the model holds
    /// of the asset. An asset the answer leaves out has no copy among the result, which reads as unplaced.
    async fn tree_now(&self, p: &Project, pins: &[Pin]) -> Result<TreeNow, AppError> {
        let pins: BTreeMap<InterfaceId, Digest> = pins
            .iter()
            .map(|pin| (pin.interface(), pin.code()))
            .collect();
        let mut asked: Vec<(TreeTarget, Held)> = Vec::new();
        for c in p.contexts() {
            for i in c.interfaces() {
                let name = self.name_of(i.id().as_uuid()).await?;
                asked.push((
                    TreeTarget::Interface {
                        context: c.name().clone(),
                        name,
                    },
                    Held::Code(i.id(), pins.get(&i.id()).copied()),
                ));
            }
            for h in c.harnesses() {
                let Some(source) = h.source() else {
                    continue;
                };
                let name = self.name_of(h.id().as_uuid()).await?;
                asked.push((
                    TreeTarget::Harness {
                        context: c.name().clone(),
                        name,
                    },
                    Held::Source(h.id(), Digest::of(source.as_str().as_bytes())),
                ));
            }
        }
        if asked.is_empty() {
            return Ok(TreeNow::empty());
        }
        let targets: Vec<TreeTarget> = asked.iter().map(|(target, _)| target.clone()).collect();
        let mut said: BTreeMap<TreeTarget, CopyRead> =
            self.tree.digests_of(&targets).await.into_iter().collect();
        let mut interfaces = BTreeMap::new();
        let mut harnesses = BTreeMap::new();
        for (target, held) in asked {
            let Some(copy) = said.remove(&target) else {
                continue;
            };
            match held {
                Held::Code(id, pin) => {
                    let now = match CodeNow::observe(pin, copy) {
                        Observed::Settled(now) => now,
                        Observed::Moved(moved) => moved.without_items(),
                    };
                    interfaces.insert(id, now);
                }
                Held::Source(id, source) => {
                    harnesses.insert(id, FileNow::observe(source, copy));
                }
            }
        }
        Ok(TreeNow::new(interfaces, harnesses))
    }

    /// Where the copies `tree` of the Project `p` stand, in the names a person knows the assets by.
    pub(super) async fn tree_status(
        &self,
        p: &Project,
        tree: &TreeNow,
    ) -> Result<TreeStatus, AppError> {
        let mut status = TreeStatus {
            unplaced: TreeUnplaced {
                interfaces: Vec::new(),
                harnesses: Vec::new(),
            },
            unpinned: Vec::new(),
            moved: Vec::new(),
        };
        for c in p.contexts() {
            for i in c.interfaces() {
                let asset = |name: String| TreeAsset {
                    context: c.name().to_string(),
                    name,
                };
                match tree.interface(i.id()) {
                    CodeNow::Unplaced => {
                        let name = self.name_of(i.id().as_uuid()).await?.to_string();
                        status.unplaced.interfaces.push(asset(name));
                    }
                    CodeNow::Unpinned => {
                        let name = self.name_of(i.id().as_uuid()).await?.to_string();
                        status.unpinned.push(asset(name));
                    }
                    CodeNow::Moved { declared } => {
                        status.moved.push(TreeMoved {
                            context: c.name().to_string(),
                            interface: self.name_of(i.id().as_uuid()).await?.to_string(),
                            declared,
                        });
                    }
                    CodeNow::Missing | CodeNow::Unreadable(_) | CodeNow::Same => {}
                }
            }
            for h in c.harnesses().filter(|h| h.source().is_some()) {
                if tree.harness(h.id()) == FileNow::Unplaced {
                    status.unplaced.harnesses.push(TreeAsset {
                        context: c.name().to_string(),
                        name: self.name_of(h.id().as_uuid()).await?.to_string(),
                    });
                }
            }
        }
        Ok(status)
    }
}
