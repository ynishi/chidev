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
//! chidev's store is the source of truth of both halves of what it handles:
//!
//! - **the text of an asset**: a Contract and its Clauses, an Interface's
//!   declarations, a Harness;
//! - **the state**: ids, revisions, the revision a citation recorded, Pins,
//!   measurements, judgments, change requests and their decisions. It is state
//!   across time, and a measurement cannot be rebuilt from anything outside it.
//!
//! Files are a way in and out, not a second home of the assets:
//!
//! - chidev reads the text it is given, inline or from a file, and gives its model
//!   back on request (`view`). Moving text between files and the store is a thing
//!   chidev does; the files are the user's.
//! - chidev does not manage files outside its store. It records no path an asset
//!   came from or went to, follows no file, and gives a file no id. Where the
//!   files and the store are kept, how they are laid out, and whether they are in
//!   Git, is the user's to choose; so is how the code's tree is tied to the
//!   assets (`[[measure.layout]]` is written by the user, and chidev only reads
//!   it). chidev depends on no Git and no Git host.
//! - An asset is known by the names written in it (a Context, a name, a Clause's
//!   label, an Item's key), not by a file. An Item that is renamed says so with
//!   `replaces`.
//! - chidev is a local command over a local store. A hosted service, when one is
//!   needed, is another adapter of the same [`Store`](app::ports::Store) port over
//!   this core; [`domain`] does not change with it.
//! - Prompts are not chidev's. chidev gives data and the schema of that data;
//!   how an agent is instructed lives outside it.
#![warn(missing_docs)]

pub mod app;
pub mod domain;
pub mod infra;
