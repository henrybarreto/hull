//! Hull network management library.
#![allow(missing_docs)]
#![allow(clippy::unused_self)]
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};

pub mod cidr;
pub mod database;
pub mod ebpf;
pub mod interfaces;
/// Client/daemon protocol types.
pub mod protocol;
pub mod switches;
pub mod utils;

pub fn get_root_path() -> PathBuf {
    std::env::var("HULL_PATH").map_or_else(
        |_| {
            std::env::var("XDG_DATA_HOME").map_or_else(
                |_| PathBuf::from("/var/lib/hull"),
                |dir| PathBuf::from(dir).join("hull"),
            )
        },
        PathBuf::from,
    )
}

pub fn get_db_path(root: &Path) -> PathBuf {
    root.join("hull.db")
}

pub fn get_socket_path(root: &Path) -> PathBuf {
    std::env::var("HULL_SOCKET").map_or_else(|_| root.join("hulld.sock"), PathBuf::from)
}
