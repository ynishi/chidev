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
//! The model is the asset, and chidev's store is its source of truth. The model
//! is everything [`domain`] holds: Contracts and their Clauses, Interfaces and
//! their Items, Harnesses and their Cases, Pins, change requests and their
//! decisions, measurements and judgments, with their ids and revisions. The text
//! of a Contract or an Interface is part of it, not a half that could be kept
//! somewhere else; a measurement cannot be rebuilt from anything outside it.
//!
//! Files are outside the model. They are the user's, handled with whatever the
//! user works with, and chidev does not manage them:
//!
//! - chidev reads the text it is given, inline or from a file named relative to
//!   the directory it runs in, and gives its model back on request (`view`). The
//!   file is read once, when the operation runs.
//! - It records no path an asset came from or went to, follows no file, and gives
//!   a file no id. (The path an assumption cites as its evidence is kept as a
//!   citation, and chidev does not open it.) Where the files and the store are
//!   kept, how they are laid out, and whether they are in Git, is the user's to
//!   choose. chidev depends on no Git and no Git host.
//! - The project's settings (`checks.toml`) say where the adapters work: which
//!   checkers run, how a measurement builds and tests, where an Interface's code
//!   is written in the copy a measurement makes (`[[measure.layout]]`). The user
//!   writes them and chidev only reads them. They hold no asset's identity: the
//!   model means the same without them. chidev never writes them, uses a path in
//!   them only as a place in that copy, and never takes an asset's text from one.
//! - An asset is known by the names it is given when it is put (a Context, a name,
//!   a Clause's label, an Item's key), not by a file. Only an Item keeps its id
//!   when it is renamed (`replaces`); renaming anything else makes a new asset,
//!   and what cited the old one still cites the old one.
//! - chidev is a local command over a local store. A hosted service, when one is
//!   needed, is another adapter of the same [`Store`](app::ports::Store) port over
//!   this core; [`domain`] does not change with it.
//! - Prompts are not chidev's. chidev gives data and the schema of that data;
//!   how an agent is instructed lives outside it.
#![warn(missing_docs)]

pub mod app;
pub mod domain;
pub mod infra;
