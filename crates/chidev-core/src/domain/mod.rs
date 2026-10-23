//! The rules and types of the problem, in its own words.
//!
//! Pure: nothing here touches the file system, a process, the environment, the
//! network, the clock or a random source. A value that comes from outside
//! (a file's contents, the current time) is passed in by the caller.
//! A value with rules is a type, not a `String`: an enum for a closed set,
//! a newtype that checks its input for anything else.
//!
//! The model is split by what a type is: [`vo`] holds the values (ids, names,
//! digests, texts), compared by what they hold.

pub mod vo;
