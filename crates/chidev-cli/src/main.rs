//! The `chidev` command: parses arguments, calls
//! `chidev-core`, prints the result.
//!
//! The argument parser's definition (clap's derive types, once added) is the
//! only description of the commands: help, the command list and validation
//! come from it, not from a second hand-kept table. Structured input is read
//! from a file argument, not pasted into the command line.
//!
//! A library with no command deletes this crate.

fn main() {}
