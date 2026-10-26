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
//!   and the results; it never judges). Done. It runs at no moment and cannot be mandatory.
//! - **external** ([`external`]): a project-registered program run through an application port. Its findings
//!   share the policy and change gates; execution failures are explicit findings. See [`Executor::External`].
//!
//! What any of them finds is the same [`Finding`], which a change request can address.

pub mod builtin;
pub mod descriptor;
pub mod external;
pub mod finding;
pub mod introduced;
pub mod judged;
pub mod level;
pub mod policy;
pub mod view;

pub use self::descriptor::{CheckKind, Descriptor, Executor, ParamDefault, ParamKind, ParamSpec};
pub use self::finding::{Finding, FindingState, Ground, LineState, Mention};
pub use self::introduced::introduced;
pub use self::judged::{JudgedDraft, JudgedRule, JudgedRuleError};
pub use self::level::{DIGEST_MOMENTS_KEY, Level, Moment, MomentWordError, renamed_key};
pub use self::policy::{Layer, Origin, ParamValue, Params, Policy, PolicyError, Selector, Setting};
pub use self::view::{Change, CheckView};

/// A check that runs in this process.
pub trait Check: Send + Sync {
    /// What the check says about itself.
    fn descriptor(&self) -> &'static Descriptor;

    /// What the check finds in `view`, with `params` set by the policy. It names no level of its own choosing: the
    /// runner sets the level the policy gives.
    fn run(&self, view: &CheckView<'_>, params: &Params) -> Vec<Finding>;
}
