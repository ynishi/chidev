//! Core (SDK) of chidev: Contract, Interface and Harness as the assets of a codebase; the code is a cache pinned to them
//!
//! Three layers, inner to outer. `arch-lint.toml` at the workspace root states
//! them, and `cargo test` (`tests/architecture.rs`) fails when code breaks them.
//!
//! - [`domain`]: the rules and types of the problem. Pure: no file system,
//!   process, environment, network, clock or randomness. Those come in as values.
//! - [`app`]: the use cases, and the ports (traits) they need from the outside
//!   world. Also free of direct I/O.
//! - [`infra`]: adapters that implement the ports with real I/O.
//!
//! A layer that does not fit this crate (a device driver has no domain worth
//! the name, for example) is deleted, module and `arch-lint.toml` scope
//! together. The rest stays.
#![warn(missing_docs)]

pub mod app;
pub mod domain;
pub mod infra;
