use anyhow::{Result, anyhow};
use serde_json::Value;
use std::process::Command;
use tracing::{debug, trace};

/// System-level interface operations.
pub struct Interface;

impl Interface {
    /// Create a TAP interface on the system.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails or the interface cannot be created.
    pub fn create(name: &str, mac: Option<&str>) -> Result<()> {
        debug!(interface = %name, "creating system tap interface");
        let status = Command::new("ip")
            .args(["tuntap", "add", "dev", name, "mode", "tap"])
            .status()?;
        if !status.success() {
            return Err(anyhow!("Failed to create TAP interface '{name}' on system"));
        }

        if let Some(mac) = mac {
            Self::set_mac(name, mac)?;
        }

        Ok(())
    }

    /// Set the interface MAC address.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails or the address cannot be set.
    pub fn set_mac(name: &str, mac: &str) -> Result<()> {
        debug!(interface = %name, mac = %mac, "setting interface mac");
        let status = Command::new("ip")
            .args(["link", "set", "dev", name, "address", mac])
            .status()?;
        if !status.success() {
            return Err(anyhow!("Failed to set MAC for interface '{name}'"));
        }
        Ok(())
    }

    /// Remove the interface from the system.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails while deleting an existing interface.
    pub fn delete(name: &str) -> Result<()> {
        trace!(interface = %name, "deleting system interface");
        if !Self::exists(name) {
            trace!(interface = %name, "interface already absent");
            return Ok(());
        }

        let _ = Command::new("ip").args(["link", "delete", name]).status();
        Ok(())
    }

    /// Check if the interface exists on the system.
    pub fn exists(name: &str) -> bool {
        trace!(interface = %name, "checking interface existence");
        Command::new("ip")
            .args(["link", "show", name])
            .output()
            .is_ok_and(|output| output.status.success())
    }

    /// Check whether an interface is administratively up.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails or output cannot be parsed.
    pub fn is_up(name: &str) -> Result<bool> {
        let output = Command::new("ip")
            .args(["-j", "link", "show", name])
            .output()?;
        if !output.status.success() {
            return Err(anyhow!("failed to inspect interface '{name}'"));
        }
        let json: Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
        let up = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("flags"))
            .and_then(Value::as_array)
            .is_some_and(|flags| flags.iter().any(|f| f.as_str() == Some("UP")));
        Ok(up)
    }

    /// Check whether an interface has the provided IPv4 address assigned.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails or output cannot be parsed.
    pub fn has_ipv4(name: &str, ip: &str) -> Result<bool> {
        let output = Command::new("ip")
            .args(["-j", "-4", "addr", "show", "dev", name])
            .output()?;
        if !output.status.success() {
            return Err(anyhow!("failed to inspect ipv4 addresses for '{name}'"));
        }
        let json: Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
        let found = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("addr_info"))
            .and_then(Value::as_array)
            .is_some_and(|infos| {
                infos
                    .iter()
                    .filter_map(|info| info.get("local").and_then(Value::as_str))
                    .any(|local| local == ip)
            });
        Ok(found)
    }

    /// Set the interface state to UP.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails or the interface cannot be brought up.
    pub fn up(name: &str) -> Result<()> {
        debug!(interface = %name, "bringing interface up");
        let status = Command::new("ip")
            .args(["link", "set", name, "up"])
            .status()?;
        if !status.success() {
            return Err(anyhow!("Failed to set interface '{name}' up"));
        }
        Ok(())
    }

    /// Get the MAC address of the interface.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails or its JSON output cannot be parsed.
    pub fn get_mac(name: &str) -> Result<String> {
        trace!(interface = %name, "reading interface mac");
        let output = Command::new("ip")
            .args(["-j", "link", "show", name])
            .output()?;

        if !output.status.success() {
            return Err(anyhow!(
                "failed to get MAC for '{name}': interface may not exist"
            ));
        }

        let json: Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
        let mac = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("address"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("could not find address for '{name}' in JSON"))?;

        Ok(mac.to_string())
    }

    /// Get the first IPv4 address assigned to the interface.
    ///
    /// # Errors
    /// Returns an error if the interface has no IPv4 address or output parsing fails.
    pub fn get_ipv4(name: &str) -> Result<String> {
        let output = Command::new("ip")
            .args(["-j", "-4", "addr", "show", "dev", name])
            .output()?;
        if !output.status.success() {
            return Err(anyhow!("failed to get ipv4 for interface '{name}'"));
        }
        let json: Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
        let ip = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("addr_info"))
            .and_then(Value::as_array)
            .and_then(|infos| infos.first())
            .and_then(|info| info.get("local"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("no ipv4 address found on interface '{name}'"))?;
        Ok(ip.to_string())
    }

    /// Get the default gateway IP for this interface.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails, its JSON output cannot be parsed,
    /// or the interface has no default gateway.
    pub fn get_gateway_ip(name: &str) -> Result<String> {
        trace!(interface = %name, "reading interface gateway");
        let output = Command::new("ip")
            .args(["-j", "route", "show", "dev", name, "default"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout)?;

        if let Some(gw) = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("gateway"))
            .and_then(Value::as_str)
        {
            return Ok(gw.to_string());
        }

        Err(anyhow!("no default gateway found for interface {name}"))
    }

    /// Get the default gateway IP from the system.
    ///
    /// # Errors
    /// Returns an error if the `ip` command fails, its JSON output cannot be parsed,
    /// or no default gateway exists.
    pub fn get_system_default_gateway_ip() -> Result<String> {
        trace!("reading system default gateway");
        let output = Command::new("ip")
            .args(["-j", "route", "show", "default"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout)?;

        if let Some(gw) = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("gateway"))
            .and_then(Value::as_str)
        {
            return Ok(gw.to_string());
        }

        Err(anyhow!("no system default gateway found"))
    }

    /// Get a neighbor MAC address for an IP on an interface.
    ///
    /// # Errors
    /// Returns an error if the neighbor is absent or has no link-layer address.
    pub fn get_neighbor_mac(name: &str, ip: &str) -> Result<String> {
        trace!(interface = %name, ip = %ip, "reading neighbor mac");
        let output = Command::new("ip")
            .args(["-j", "neigh", "show", "to", ip, "dev", name])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout)?;

        if let Some(mac) = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.get("lladdr"))
            .and_then(Value::as_str)
        {
            return Ok(mac.to_string());
        }

        Err(anyhow!(
            "no neighbor MAC found for '{ip}' on interface '{name}'"
        ))
    }
}
