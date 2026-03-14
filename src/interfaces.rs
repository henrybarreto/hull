use crate::database::Database;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;

/// System-level interface operations.
pub struct Interface;

impl Interface {
    /// Create a TAP interface on the system.
    pub fn create(name: &str) -> Result<()> {
        let status = Command::new("ip")
            .args(["tuntap", "add", "dev", name, "mode", "tap"])
            .status()?;
        if !status.success() {
            return Err(anyhow!(
                "Failed to create TAP interface '{}' on system",
                name
            ));
        }
        Ok(())
    }

    /// Remove the interface from the system.
    pub fn delete(name: &str) -> Result<()> {
        let _ = Command::new("ip").args(["link", "delete", name]).status();
        Ok(())
    }

    /// Check if the interface exists on the system.
    pub fn exists(name: &str) -> bool {
        Command::new("ip")
            .args(["link", "show", name])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Set the interface state to UP.
    pub fn up(name: &str) -> Result<()> {
        let status = Command::new("ip")
            .args(["link", "set", name, "up"])
            .status()?;
        if !status.success() {
            return Err(anyhow!("Failed to set interface '{}' up", name));
        }
        Ok(())
    }

    /// Get the MAC address of the interface.
    pub fn get_mac(name: &str) -> Result<String> {
        let output = Command::new("ip")
            .args(["-j", "link", "show", name])
            .output()?;

        if !output.status.success() {
            return Err(anyhow!(
                "failed to get MAC for '{}': interface may not exist",
                name
            ));
        }

        let json: Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;
        let mac = json[0]["address"]
            .as_str()
            .ok_or_else(|| anyhow!("could not find address for '{}' in JSON", name))?;

        Ok(mac.to_string())
    }

    /// Get the default gateway IP for this interface.
    pub fn get_gateway_ip(name: &str) -> Result<String> {
        let output = Command::new("ip")
            .args(["-j", "route", "show", "dev", name, "default"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout)?;

        if let Some(gw) = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry["gateway"].as_str())
        {
            return Ok(gw.to_string());
        }

        return Err(anyhow!("no default gateway found for interface {}", name));
    }

    /// Get the default gateway IP from the system.
    pub fn get_system_default_gateway_ip() -> Result<String> {
        let output = Command::new("ip")
            .args(["-j", "route", "show", "default"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout)?;

        if let Some(gw) = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry["gateway"].as_str())
        {
            return Ok(gw.to_string());
        }

        return Err(anyhow!("no system default gateway found"));
    }

    /// Resolve the MAC address for a given IP.
    pub fn resolve_neighbor_mac(ip: &str) -> Result<String> {
        let output = Command::new("ip")
            .args(["-j", "neigh", "show", ip])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout)?;

        if let Some(mac) = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry["lladdr"].as_str())
        {
            return Ok(mac.to_string());
        }

        return Err(anyhow!("could not resolve MAC for neighbor {}", ip));
    }
}

pub struct InterfaceOps {
    db: Arc<Database>,
}

impl InterfaceOps {
    /// Create a new interface operations instance.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Create a TAP interface and store it in the database.
    pub fn create(&self, name: &str) -> Result<()> {
        if self.db.get_interface(name).is_ok() {
            return Err(anyhow!("Interface '{}' already exists in database", name));
        }

        if Interface::exists(name) {
            return Err(anyhow!("Interface '{}' already exists on system", name));
        }

        let _ = self.db.create_interface(name)?;

        Interface::create(name)?;
        Interface::up(name)?;

        Ok(())
    }

    /// Fetch an interface from the database.
    pub fn get(&self, name: &str) -> Result<crate::database::Interface> {
        self.db.get_interface(name)
    }

    /// Remove a TAP interface and delete its record.
    pub fn remove(&self, name: &str) -> Result<()> {
        let interface = self.db.get_interface(name)?;

        if Interface::exists(&interface.name) {
            let _ = Interface::delete(&interface.name);
        }

        self.db.remove_interface(&interface.name)?;
        Ok(())
    }

    /// List all interfaces.
    pub fn list(&self) -> Result<Vec<crate::database::Interface>> {
        self.db.list_interfaces()
    }

    /// Re-create any missing interfaces from database state.
    pub fn sync(&self) -> Result<()> {
        let interfaces = self.db.list_interfaces()?;
        for interface in interfaces {
            if !Interface::exists(&interface.name) {
                Interface::create(&interface.name)?;
            }

            Interface::up(&interface.name)?;
        }

        Ok(())
    }
}
