//! Hull network management library.
#![allow(missing_docs)]
#![allow(clippy::unused_self)]
#![allow(clippy::needless_pass_by_value)]

pub mod config;
pub mod database;
pub mod ebpf;
pub mod interfaces;
/// Client/daemon protocol types.
pub mod protocol;
pub mod switches;
pub mod utils;
