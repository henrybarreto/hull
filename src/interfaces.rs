use crate::database::Database;
use crate::utils::generate_random_mac;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, info, trace};

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
}

/// Database-backed interface operations.
pub struct InterfaceOps {
    db: Arc<Database>,
}

impl InterfaceOps {
    /// Create a new interface operations instance.
    pub const fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Create a TAP interface and store it in the database.
    ///
    /// # Errors
    /// Returns an error if the interface exists, database writes fail, or system setup fails.
    pub fn create(&self, name: &str, mac: Option<&str>) -> Result<crate::database::Interface> {
        debug!(interface = %name, "creating interface record");
        if self.db.get_interface(name).is_ok() {
            return Err(anyhow!("Interface '{name}' already exists in database"));
        }

        if Interface::exists(name) {
            return Err(anyhow!("Interface '{name}' already exists on system"));
        }

        let mac = mac.map_or_else(generate_random_mac, std::string::ToString::to_string);
        let interface = self.db.create_interface(name, &mac)?;

        Interface::create(name, Some(&interface.mac))?;
        Interface::up(name)?;

        Ok(interface)
    }

    /// Fetch an interface from the database.
    ///
    /// # Errors
    /// Returns an error if the interface does not exist or the database query fails.
    pub fn get(&self, name: &str) -> Result<crate::database::Interface> {
        trace!(interface = %name, "fetching interface record");
        self.db.get_interface(name)
    }

    /// Remove a TAP interface and delete its record.
    ///
    /// # Errors
    /// Returns an error if the interface does not exist, database deletion fails,
    /// or system cleanup fails.
    pub fn remove(&self, name: &str) -> Result<()> {
        debug!(interface = %name, "removing interface");
        let interface = self.db.get_interface(name)?;

        if Interface::exists(&interface.name) {
            let _ = Interface::delete(&interface.name);
        }

        self.db.remove_interface(&interface.name)?;
        Ok(())
    }

    /// List all interfaces.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn list(&self) -> Result<Vec<crate::database::Interface>> {
        trace!("listing interfaces");
        self.db.list_interfaces()
    }

    /// Re-create any missing interfaces from database state.
    ///
    /// # Errors
    /// Returns an error if database access or system setup fails.
    pub fn sync(&self) -> Result<()> {
        info!("syncing interfaces from database");
        let interfaces = self.db.list_interfaces()?;
        info!(
            count = interfaces.len(),
            "found tracked interfaces to reconcile"
        );
        for interface in interfaces {
            if !Interface::exists(&interface.name) {
                info!(interface = %interface.name, "recreating missing interface");
                Interface::create(&interface.name, Some(&interface.mac))?;
            }

            Interface::up(&interface.name)?;
        }

        info!("completed interface reconciliation");

        Ok(())
    }
}
