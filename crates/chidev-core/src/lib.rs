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
//!
//! # Where the truth is
//!
//! chidev computes over Contracts; it does not manage them. The two halves of
//! what it handles have their source of truth in different places.
//!
//! - **The text of an asset** (a Contract and its Clauses, an Interface's
//!   declarations, a Harness) is a file in the repository the code lives in.
//!   Review, branches, sharing and permissions are Git's and the Git host's, not
//!   chidev's.
//! - **The state** (ids, revisions, the revision a citation recorded, Pins,
//!   measurements, judgments, change requests and their decisions) is chidev's
//!   store. It is state across time: a measurement cannot be rebuilt from files,
//!   and kept in files it would make every branch a merge of results.
//!
//! What follows from this:
//!
//! - chidev reads an asset's file into its model and keeps the version it read;
//!   a file that changes is a new revision. Reading the assets from the
//!   repository is to be the main way in, so that a project can be rebuilt from
//!   Git. The puts that take the text inline stay until then, and are today the
//!   only way in.
//! - Reading back is split the same way: the text of an asset is read from its
//!   file, and chidev answers for the state and for the model it read.
//! - How a file is tied to an asset's id (by its path and name, or by an id
//!   written in the file) is not decided yet. A tie that people must remember to
//!   write, such as a key in a commit message, is not one.
//! - chidev is a local command over a local store. A hosted service, when one is
//!   needed, is another adapter of the same [`Store`](app::ports::Store) port over
//!   this core, adding what Git cannot give: an index across repositories, the
//!   state shared with its history, permissions. Where the truth is lives in the
//!   adapters and the import; [`domain`] does not change with it.
//! - Prompts are not chidev's. chidev gives data and the schema of that data;
//!   how an agent is instructed lives outside it.
#![warn(missing_docs)]

pub mod app;
pub mod domain;
pub mod infra;
