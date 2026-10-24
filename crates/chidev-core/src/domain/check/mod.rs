//! Checks: rules about the model that find something worth a person's attention (a citation behind its target, a
//! Clause no Case checks, a change request resolved before its parts). A check is a pure function of a
//! [`CheckView`]; the level it is held to and its parameters come from the [`policy`], which the layers of a project
//! and a person set; what it finds is a [`Finding`], which a change request can address.
//!
//! The same descriptor, finding, level and policy serve every way a check can be run, and there are three
//! [`Executor`]s:
//!
//! - **builtin** ([`builtin`]): Rust code of this crate, a pure function of the view. Done.
//! - **judged** ([`judged`]): a rule written as data, which the agent driving chidev judges (chidev keeps the rules
//!   and the results; it never judges). Done. It runs at no hook and cannot be mandatory.
//! - **external**: a program run as a process. Designed, not implemented; the design is on [`Executor::External`]
//!   (manifest in `checks.toml`, a versioned JSON request and response on stdin and stdout, advisory unless
//!   promoted and then fail-closed, the command's checksum recorded, not sandboxed).
//!
//! What any of them finds is the same [`Finding`], which a change request can address.

pub mod builtin;
pub mod descriptor;
pub mod finding;
pub mod judged;
pub mod level;
pub mod policy;
pub mod view;

pub use self::descriptor::{Descriptor, Executor, ParamDefault, ParamKind, ParamSpec};
pub use self::finding::{Finding, FindingState, Mention};
pub use self::judged::{JudgedDraft, JudgedRule, JudgedRuleError};
pub use self::level::{Hook, Level};
pub use self::policy::{Layer, Origin, ParamValue, Params, Policy, PolicyError, Selector, Setting};
pub use self::view::CheckView;

/// A check that runs in this process.
pub trait Check: Send + Sync {
    /// What the check says about itself.
    fn descriptor(&self) -> &'static Descriptor;

    /// What the check finds in `view`, with `params` set by the policy. It names no level of its own choosing: the
    /// runner sets the level the policy gives.
    fn run(&self, view: &CheckView<'_>, params: &Params) -> Vec<Finding>;
}
