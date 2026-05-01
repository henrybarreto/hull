use anyhow::Result;

use tracing::{debug, trace};

use crate::openflow::client::Connection;
use crate::openflow::protocol::rule::Rule;

/// Thin wrapper around an `OpenFlow` connection.
pub struct OF {
    connection: Connection<tokio::net::UnixStream>,
}

impl OF {
    /// Connect to `OpenFlow` on the given bridge.
    ///
    /// # Errors
    /// Returns an error if the bridge connection cannot be established.
    pub async fn connect(bridge: &str) -> Result<Self> {
        debug!(bridge = %bridge, "connecting to openflow");
        Ok(Self {
            connection: Connection::connect_bridge(bridge).await?,
        })
    }

    /// Insert a flow into the bridge.
    ///
    /// # Errors
    /// Returns an error if the flow cannot be installed.
    pub async fn insert(&mut self, flow_mod: Rule) -> Result<()> {
        trace!("inserting openflow rule");
        self.connection.add_flow(flow_mod).await?;
        Ok(())
    }

    /// Remove flows matching an optional cookie.
    ///
    /// # Errors
    /// Returns an error if the flow deletion fails.
    pub async fn remove(&mut self, cookie: Option<u64>) -> Result<()> {
        trace!(cookie = ?cookie, "removing openflow flows");
        self.connection.delete_flows(cookie).await?;
        Ok(())
    }
}
