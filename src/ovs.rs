use anyhow::{Result, anyhow};
use ovsdb_client::ops::Ops;
use ovsdb_client::{Connection, TransactionOutcome};
use serde_json::{Value, json};
use tracing::{debug, trace};

/// Thin wrapper around an OVSDB connection.
pub struct BridgeClient {
    connection: Connection,
}

impl BridgeClient {
    /// Connect to the OVSDB socket.
    ///
    /// # Errors
    /// Returns an error if the database socket cannot be opened.
    pub async fn connect() -> Result<Self> {
        debug!("connecting to ovsdb");
        let connection = Connection::connect("unix:/var/run/openvswitch/db.sock", None)
            .await
            .map_err(|e| anyhow!("Failed to connect to OVSDB: {e}"))?;
        Ok(Self { connection })
    }

    /// Check whether a bridge exists.
    ///
    /// # Errors
    /// Returns an error if the OVSDB query fails.
    pub async fn bridge_exists(&self, name: &str) -> Result<bool> {
        trace!(bridge = %name, "checking bridge existence");
        let bridges = self.list_bridges().await?;
        Ok(bridges.iter().any(|b| b == name))
    }

    /// Create a bridge if it does not already exist.
    ///
    /// # Errors
    /// Returns an error if bridge creation fails.
    pub async fn add_bridge(&self, name: &str) -> Result<()> {
        debug!(bridge = %name, "adding bridge");
        if self.bridge_exists(name).await? {
            return Ok(());
        }

        let bridge_uuid_name = "new_bridge";
        let port_uuid_name = "new_port";
        let iface_uuid_name = "new_iface";

        let ops = vec![
            Ops::insert(
                "Interface",
                json!({
                    "name": name,
                    "type": "internal"
                }),
                Some(iface_uuid_name),
            ),
            Ops::insert(
                "Port",
                json!({
                    "name": name,
                    "interfaces": ["set", [["named-uuid", iface_uuid_name]]]
                }),
                Some(port_uuid_name),
            ),
            Ops::insert(
                "Bridge",
                json!({
                    "name": name,
                    "ports": ["set", [["named-uuid", port_uuid_name]]]
                }),
                Some(bridge_uuid_name),
            ),
            Ops::mutate(
                "Open_vSwitch",
                &[],
                &[json!([
                    "bridges",
                    "insert",
                    ["set", [["named-uuid", bridge_uuid_name]]]
                ])],
            ),
        ];

        self.transact(ops).await
    }

    /// Delete a bridge if it exists.
    ///
    /// # Errors
    /// Returns an error if bridge deletion fails.
    pub async fn del_bridge(&self, name: &str) -> Result<()> {
        debug!(bridge = %name, "deleting bridge");
        let Ok(bridge_uuid) = self.get_bridge_uuid(name).await else {
            return Ok(());
        };

        let ops = vec![
            Ops::mutate(
                "Open_vSwitch",
                &[],
                &[json!([
                    "bridges",
                    "delete",
                    ["set", [["uuid", bridge_uuid]]]
                ])],
            ),
            Ops::delete("Bridge", &[json!(["name", "==", name])]),
        ];

        self.transact(ops).await
    }

    /// List all bridge names.
    ///
    /// # Errors
    /// Returns an error if the OVSDB query fails.
    pub async fn list_bridges(&self) -> Result<Vec<String>> {
        trace!("listing bridges");
        let ops = vec![Ops::select("Bridge", &[], Some(&["name".to_string()]))];

        let response = self
            .connection
            .transact("Open_vSwitch", ops)
            .await
            .map_err(|e| anyhow!("OVSDB transaction failed: {e}"))?;

        let mut bridges = Vec::new();
        for entry in response.entries {
            match entry {
                TransactionOutcome::Select { rows } => {
                    bridges.extend(rows.into_iter().filter_map(|row| {
                        row.get("name")
                            .and_then(serde_json::Value::as_str)
                            .map(std::string::ToString::to_string)
                    }));
                }
                TransactionOutcome::Error(err) => {
                    let details = err.details.unwrap_or_default();
                    let error = err.error;
                    return Err(anyhow!("OVSDB error: {error} - {details}"));
                }
                _ => {}
            }
        }

        Ok(bridges)
    }

    /// Check whether a port exists.
    ///
    /// # Errors
    /// Returns an error if the OVSDB query fails.
    pub async fn port_exists(&self, _bridge_name: &str, port_name: &str) -> Result<bool> {
        trace!(port = %port_name, "checking port existence");
        let ops = vec![Ops::select(
            "Port",
            &[json!(["name", "==", port_name])],
            Some(&["name".to_string()]),
        )];

        let response = self
            .connection
            .transact("Open_vSwitch", ops)
            .await
            .map_err(|e| anyhow!("OVSDB transaction failed: {e}"))?;

        if let Some(TransactionOutcome::Select { rows }) = response.entries.first() {
            return Ok(!rows.is_empty());
        }

        Ok(false)
    }

    /// Look up the `OpenFlow` port for an interface.
    ///
    /// # Errors
    /// Returns an error if the OVSDB query fails.
    pub async fn interface_ofport(&self, interface_name: &str) -> Result<Option<u32>> {
        trace!(interface = %interface_name, "looking up interface ofport");
        let ops = vec![Ops::select(
            "Interface",
            &[json!(["name", "==", interface_name])],
            Some(&["ofport".to_string()]),
        )];

        let response = self
            .connection
            .transact("Open_vSwitch", ops)
            .await
            .map_err(|e| anyhow!("OVSDB transaction failed: {e}"))?;

        if let Some(TransactionOutcome::Select { rows }) = response.entries.first()
            && let Some(row) = rows.first()
            && let Some(ofport) = row.get("ofport").and_then(serde_json::Value::as_i64)
            && ofport >= 0
            && let Ok(ofport) = u32::try_from(ofport)
        {
            return Ok(Some(ofport));
        }

        Ok(None)
    }

    /// Add a port to a bridge.
    ///
    /// # Errors
    /// Returns an error if port creation fails.
    pub async fn add_port(
        &self,
        bridge_name: &str,
        port_name: &str,
        other_config: Value,
    ) -> Result<()> {
        debug!(bridge = %bridge_name, port = %port_name, "adding port");
        if self.port_exists(bridge_name, port_name).await? {
            // Port exists, maybe update other_config?
            // For now just return Ok to match --may-exist behavior
            return Ok(());
        }

        let port_uuid_name = "new_port";
        let iface_uuid_name = "new_iface";

        let ops = vec![
            Ops::insert(
                "Interface",
                json!({
                    "name": port_name,
                    "other_config": to_ovsdb_map(other_config)
                }),
                Some(iface_uuid_name),
            ),
            Ops::insert(
                "Port",
                json!({
                    "name": port_name,
                    "interfaces": ["set", [["named-uuid", iface_uuid_name]]]
                }),
                Some(port_uuid_name),
            ),
            Ops::mutate(
                "Bridge",
                &[json!(["name", "==", bridge_name])],
                &[json!([
                    "ports",
                    "insert",
                    ["set", [["named-uuid", port_uuid_name]]]
                ])],
            ),
        ];

        self.transact(ops).await
    }

    /// Delete a port from a bridge.
    ///
    /// # Errors
    /// Returns an error if port deletion fails.
    pub async fn del_port(&self, bridge_name: &str, port_name: &str) -> Result<()> {
        debug!(bridge = %bridge_name, port = %port_name, "deleting port");
        let Ok(port_uuid) = self.get_port_uuid(port_name).await else {
            return Ok(());
        };

        let ops = vec![
            Ops::mutate(
                "Bridge",
                &[json!(["name", "==", bridge_name])],
                &[json!(["ports", "delete", ["set", [["uuid", port_uuid]]]])],
            ),
            Ops::delete("Port", &[json!(["name", "==", port_name])]),
        ];

        self.transact(ops).await
    }

    async fn transact(&self, ops: Vec<Value>) -> Result<()> {
        trace!(ops = ops.len(), "running ovsdb transaction");
        let response = self
            .connection
            .transact("Open_vSwitch", ops)
            .await
            .map_err(|e| anyhow!("OVSDB transaction failed: {e}"))?;

        for entry in response.entries {
            if let TransactionOutcome::Error(err) = entry {
                return Err(anyhow!(
                    "OVSDB error: {} - {}",
                    err.error,
                    err.details.unwrap_or_default()
                ));
            }
        }

        Ok(())
    }

    async fn get_bridge_uuid(&self, name: &str) -> Result<String> {
        trace!(bridge = %name, "looking up bridge uuid");
        let ops = vec![Ops::select(
            "Bridge",
            &[json!(["name", "==", name])],
            Some(&["_uuid".to_string()]),
        )];

        let response = self
            .connection
            .transact("Open_vSwitch", ops)
            .await
            .map_err(|e| anyhow!("OVSDB transaction failed: {e}"))?;

        if let Some(TransactionOutcome::Select { rows }) = response.entries.first()
            && let Some(row) = rows.first()
            && let Some(uuid_val) = row.get("_uuid")
            && let Some(uuid_arr) = uuid_val.as_array()
            && let Some(uuid_str) = uuid_arr.get(1).and_then(|v| v.as_str())
        {
            return Ok(uuid_str.to_string());
        }

        Err(anyhow!("Bridge '{name}' not found"))
    }

    async fn get_port_uuid(&self, name: &str) -> Result<String> {
        trace!(port = %name, "looking up port uuid");
        let ops = vec![Ops::select(
            "Port",
            &[json!(["name", "==", name])],
            Some(&["_uuid".to_string()]),
        )];

        let response = self
            .connection
            .transact("Open_vSwitch", ops)
            .await
            .map_err(|e| anyhow!("OVSDB transaction failed: {e}"))?;

        if let Some(TransactionOutcome::Select { rows }) = response.entries.first()
            && let Some(row) = rows.first()
            && let Some(uuid_val) = row.get("_uuid")
            && let Some(uuid_arr) = uuid_val.as_array()
            && let Some(uuid_str) = uuid_arr.get(1).and_then(|v| v.as_str())
        {
            return Ok(uuid_str.to_string());
        }

        Err(anyhow!("Port '{name}' not found"))
    }
}

fn to_ovsdb_map(value: Value) -> Value {
    let is_object = matches!(&value, Value::Object(_));
    trace!(is_object, "converting value to ovsdb map");
    match value {
        Value::Object(map) => {
            let mut pairs = Vec::new();
            for (k, v) in map {
                let v_str = match v {
                    Value::String(s) => s,
                    _ => v.to_string(),
                };
                pairs.push(json!([k, v_str]));
            }
            json!(["map", pairs])
        }
        _ => json!(["map", []]),
    }
}
