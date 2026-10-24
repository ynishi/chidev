//! Checks: rules about the model that find something worth a person's attention (a citation behind its target, a
//! Clause no Case checks, a change request resolved before its parts). A check is a pure function of a
//! [`CheckView`]; the level it is held to and its parameters come from the [`policy`], which the layers of a project
//! and a person set; what it finds is a [`Finding`], which a change request can address.
//!
//! The same descriptor, finding, level and policy serve every way a check can be run. Only the checks that come with
//! the code ([`builtin`]) are here; a check run as a process and a rule an agent judges are other executors that
//! would produce the same [`Finding`].

pub mod builtin;
pub mod descriptor;
pub mod finding;
pub mod level;
pub mod policy;
pub mod view;

pub use self::descriptor::{Descriptor, Executor, ParamDefault, ParamKind, ParamSpec};
pub use self::finding::{Finding, FindingState};
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
