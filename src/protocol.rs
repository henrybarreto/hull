use serde::{Deserialize, Serialize};
/// A client request sent to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// The requested operation.
    pub command: Command,
}

/// Top-level daemon commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Initialize Hull state.
    Init,
    /// Remove Hull state.
    Deinit,
    /// Switch operations.
    Switch(SwitchCommand),
    /// Router operations.
    Router(RouterCommand),
    /// Reconcile daemon state.
    Sync,
}

/// Switch subcommands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwitchCommand {
    /// List switches.
    Ls,
    /// Create a switch.
    Create {
        /// Switch name.
        name: String,
        /// Switch subnet IPv4 address.
        ip: String,
        /// Switch subnet mask.
        mask: u8,
    },
    /// Remove a switch.
    Rm {
        /// Switch name.
        name: String,
    },
    /// Switch port operations.
    Port(SwitchPortCommand),
}

/// Switch port subcommands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SwitchPortCommand {
    /// List switch ports.
    Ls,
    /// Create a switch port.
    Create {
        /// Switch name.
        switch: String,
        /// Port name.
        name: String,
        /// Optional static IPv4 for the switch port.
        ip: Option<String>,
        /// Optional static MAC for the switch port.
        mac: Option<String>,
    },
    /// Remove a switch port.
    Rm {
        /// Switch name.
        switch: String,
        /// Port name.
        name: String,
    },
}

/// Router subcommands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterCommand {
    /// List routers.
    Ls,
    /// Show a router with full details.
    Show {
        /// Router name.
        name: String,
    },
    /// Create a router.
    Create {
        /// Router name.
        name: String,
    },
    /// Remove a router.
    Rm {
        /// Router name.
        name: String,
    },
    /// Attach a router to a switch.
    Attach {
        /// Router name.
        router: String,
        /// Switch name.
        switch: String,
    },
    /// Detach a router from a switch.
    Detach {
        /// Router name.
        router: String,
        /// Switch name.
        switch: String,
    },
    /// Router route operations.
    Route(RouterRouteCommand),
}

/// Router route subcommands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterRouteCommand {
    /// Add a route.
    Add {
        /// Router name.
        router: String,
        /// Source subnet.
        source: String,
        /// Destination subnet.
        destination: String,
        /// Optional next hop IP.
        next_hop: Option<String>,
        /// Optional next hop MAC.
        next_hop_mac: Option<String>,
        /// Route metric.
        metric: u32,
    },
    /// Remove a route.
    Rm {
        /// Router name.
        router: String,
        /// Source subnet.
        source: String,
        /// Destination subnet.
        destination: String,
    },
    /// List routes.
    Ls {
        /// Router name.
        router: String,
    },
}

/// Encode an error response payload.
pub fn error_response(message: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "status": "error",
        "message": message.into(),
    })
}
