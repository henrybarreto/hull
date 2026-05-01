//! Hull network management library.

/// Runtime configuration and path resolution.
pub mod config;
/// SQLite-backed persistence for Hull state.
pub mod database;
/// System TAP interface management.
pub mod interfaces;
/// `OpenFlow` helper wrapper.
pub mod of;
/// Local `OpenFlow` implementation vendored into this crate.
mod openflow;
/// OVSDB helper wrapper.
pub mod ovs;
/// Client/daemon protocol types.
pub mod protocol;
/// Router operations and flow programming.
pub mod routers;
/// Switch operations and flow programming.
pub mod switches;
/// Shared utility helpers.
pub mod utils;
