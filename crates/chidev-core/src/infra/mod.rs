//! Adapters: the ports of [`crate::app`] implemented with real I/O (files,
//! processes, git, the network, the clock).
//!
//! The outside world's formats and errors are translated here, at the edge,
//! so that nothing past this layer sees them. An adapter that grows large
//! moves to a crate of its own.

pub mod ids;
mod persist;
pub mod policy;
pub mod sqlite;
