use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub config: Option<PathBuf>,
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Init,
    Deinit,
    Interface(InterfaceCommand),
    Switch(SwitchCommand),
    Router(RouterCommand),
    Sync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterfaceCommand {
    Ls,
    Create { name: String },
    Rm { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwitchCommand {
    Ls,
    Create { name: String, ip: String, mask: u8 },
    Rm { name: String },
    Port(SwitchPortCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwitchPortCommand {
    Ls,
    Create {
        switch: String,
        name: String,
        interface: String,
    },
    Rm {
        switch: String,
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterCommand {
    Ls,
    Create { name: String },
    Rm { name: String },
    Attach { router: String, switch: String },
    Detach { router: String, switch: String },
    Link(RouterLinkCommand),
    Route(RouterRouteCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterLinkCommand {
    Set {
        router: String,
        link: String,
        ip: String,
        mac: String,
    },
    Unset {
        router: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterRouteCommand {
    Add {
        router: String,
        source: String,
        destination: String,
        next_hop: Option<String>,
        metric: u32,
    },
    Rm {
        router: String,
        source: String,
        destination: String,
    },
    Ls {
        router: String,
    },
}

pub fn error_response(message: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "status": "error",
        "message": message.into(),
    })
}
