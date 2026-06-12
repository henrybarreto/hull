use anyhow::{Result, anyhow};
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
    fn set_mac(name: &str, mac: &str) -> Result<()> {
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
}
