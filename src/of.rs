use anyhow::Result;

use tracing::{debug, trace};

use crate::openflow::client::Connection;
use crate::openflow::protocol::rule::Rule;

/// Thin wrapper around an `OpenFlow` connection.
pub struct OF {
    connection: Connection<std::os::unix::net::UnixStream>,
}

impl OF {
    /// Connect to `OpenFlow` on the given bridge.
    ///
    /// # Errors
    /// Returns an error if the bridge connection cannot be established.
    pub fn connect(bridge: &str) -> Result<Self> {
        debug!(bridge = %bridge, "connecting to openflow");
        Ok(Self {
            connection: Connection::connect_bridge(bridge)?,
        })
    }

    /// Insert a flow into the bridge.
    ///
    /// # Errors
    /// Returns an error if the flow cannot be installed.
    pub fn insert(&mut self, flow_mod: Rule) -> Result<()> {
        trace!("inserting openflow rule");
        self.connection.add_flow(flow_mod)?;
        Ok(())
    }

    /// Remove flows matching an optional cookie.
    ///
    /// # Errors
    /// Returns an error if the flow deletion fails.
    pub fn remove(&mut self, cookie: Option<u64>) -> Result<()> {
        trace!(cookie = ?cookie, "removing openflow flows");
        self.connection.delete_flows(cookie)?;
        Ok(())
    }
}
