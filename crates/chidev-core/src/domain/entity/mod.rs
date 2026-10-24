//! Entities: what is kept over time and known by an id, so that it stays the same thing when its content
//! changes. Each counts its changes in a `Revision`.

pub mod change_request;
pub mod context;
pub mod contract;
pub mod harness;
pub mod interface;
pub mod measurement;
pub mod pin;
pub mod project;
