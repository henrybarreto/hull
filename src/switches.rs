use crate::config::Config;
use crate::database::{Database, Switch, SwitchPort};
use crate::interfaces::Interface;
use anyhow::{Result, anyhow};
use ipnetwork::IpNetwork;
use std::net::IpAddr;
use std::process::Command;
use std::sync::{Arc, Mutex};

pub static OVS_OFTCTL_HOOK: Mutex<Option<Box<dyn Fn(&[&str]) -> Result<String> + Send + Sync>>> =
    Mutex::new(None);
pub static OVS_VSCTL_HOOK: Mutex<Option<Box<dyn Fn(&[&str]) -> Result<String> + Send + Sync>>> =
    Mutex::new(None);

pub struct SwitchOps {
    db: Arc<Database>,
    config: Arc<Config>,
}

impl SwitchOps {
    /// Create a new switch operations instance.
    pub fn new(db: Arc<Database>, config: Arc<Config>) -> Self {
        Self { db, config }
    }
    /// Create a switch and apply its flows.
    pub fn create(&self, name: &str, ip: &str, mask: u8) -> Result<()> {
        if self.db.get_switch(name).is_ok() {
            return Err(anyhow!("Switch '{}' already exists", name));
        }

        let switch = self.db.create_switch(name, ip, mask)?;

        self.apply_switch_flows(&self.config, &switch)?;

        Ok(())
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        if !self.db.get_switch(name).is_ok() {
            return Err(anyhow!("Switch '{}' does not exist", name));
        }

        let switch = self.db.get_switch(name)?;

        self.delete_switch_flows(&self.config, &switch)?;
        self.db.remove_switch(&switch.name)?;

        Ok(())
    }

    /// List all switches.
    pub fn list(&self) -> Result<Vec<crate::database::Switch>> {
        self.db.list_switches()
    }

    /// Fetch a switch by name.
    pub fn get(&self, name: &str) -> Result<crate::database::Switch> {
        self.db.get_switch(name)
    }

    /// Create a switch port and apply updated flows.
    pub fn create_switch_port(&self, name: &str, switch: &str, interface_name: &str) -> Result<()> {
        if self.db.get_switch_port(name).is_ok() {
            return Err(anyhow!("Switch port '{}' already exists", name));
        }

        if !Interface::exists(interface_name) {
            return Err(anyhow!(
                "Interface '{}' does not exist on system",
                interface_name
            ));
        }

        // TODO: Avoid multiple switch ports using the same interface.

        let s = self.db.get_switch(switch)?;
        let interface = self.db.get_interface(interface_name)?;

        let mac = Interface::get_mac(interface_name)?;
        let ip = self.allocate_ip(&s.name, &s.ip, s.mask)?;

        let port = self
            .db
            .create_switch_port(name, &s.name, &interface.name, &ip, &mac)?;

        ovs_vsctl(&[
            "--may-exist",
            "add-port",
            &self.config.bridge_name,
            &port.interface_name,
            "--",
            "set",
            "Interface",
            &port.interface_name,
            &format!("other_config:hull-switch={}", switch),
            &format!("other_config:hull-port={}", name),
        ])?;

        let ports = self.db.get_switch_ports_for_switch(&s.name)?;
        for p in &ports {
            let _ = self.delete_switch_port_flows(&self.config, &s, p);
            self.apply_switch_port_flows(&self.config, &s, &ports, p)?;
        }

        Ok(())
    }

    /// Remove a switch port and apply updated flows.
    pub fn remove_switch_port(&self, switch: &str, name: &str) -> Result<()> {
        if !self.db.get_switch(switch).is_ok() {
            return Err(anyhow!("Switch '{}' does not exist", switch));
        }

        if !self.db.get_switch_port(name).is_ok() {
            return Err(anyhow!("Switch port '{}' does not exist", name));
        }

        // NOTE: Check if switch port is attached to a switch before deleting.

        let switch = self.db.get_switch(switch)?;
        let port = self.db.get_switch_port(name)?;

        let ports = self.db.get_switch_ports_for_switch(&switch.name)?;
        for p in &ports {
            let _ = self.delete_switch_port_flows(&self.config, &switch, p);
        }

        let _ = ovs_vsctl(&["del-port", &self.config.bridge_name, &port.interface_name]);

        self.db.remove_switch_port(&port.name)?;
        self.apply_switch_flows(&self.config, &switch)?;

        Ok(())
    }

    /// List all switch ports.
    pub fn list_switch_ports(&self) -> Result<Vec<crate::database::SwitchPort>> {
        self.db.list_switch_ports()
    }

    /// Fetch a switch port by name.
    pub fn get_switch_port(&self, name: &str) -> Result<crate::database::SwitchPort> {
        self.db.get_switch_port(name)
    }

    /// Allocate an unused IP from the switch subnet.
    fn allocate_ip(&self, switch_name: &str, switch_ip: &str, switch_mask: u8) -> Result<String> {
        let network_str = format!("{}/{}", switch_ip, switch_mask);
        let network: IpNetwork = network_str
            .parse()
            .map_err(|e| anyhow!("invalid switch network: {}", e))?;

        let existing_ports = self.db.get_switch_ports_for_switch(switch_name)?;
        let used_ips: std::collections::HashSet<String> =
            existing_ports.into_iter().map(|p| p.ip).collect();

        for ip in network.iter() {
            let ip_str = ip.to_string();

            if ip == network.network() {
                continue;
            }
            if let IpNetwork::V4(v4) = network {
                if ip == IpAddr::V4(v4.broadcast()) {
                    continue;
                }
            }

            if ip_str.ends_with(".1") {
                continue;
            }

            if !used_ips.contains(&ip_str) {
                return Ok(ip_str);
            }
        }

        Err(anyhow!("No available IPs in switch subnet"))
    }

    /// Re-apply all switch flows from database state.
    pub fn sync(&self) -> Result<()> {
        let switches = self.db.list_switches()?;
        for switch in switches {
            let ports = self.db.get_switch_ports_for_switch(&switch.name)?;
            for port in &ports {
                ovs_vsctl(&[
                    "--may-exist",
                    "add-port",
                    &self.config.bridge_name,
                    &port.interface_name,
                    "--",
                    "set",
                    "Interface",
                    &port.interface_name,
                    &format!("other_config:hull-switch={}", switch.name),
                    &format!("other_config:hull-port={}", port.name),
                ])?;
            }

            self.delete_switch_flows(&self.config, &switch)?;
            self.apply_switch_flows(&self.config, &switch)?;
        }

        Ok(())
    }

    /// Apply all flows for a switch.
    pub fn apply_switch_flows(&self, config: &Config, switch: &Switch) -> Result<()> {
        let bridge = &config.bridge_name;
        let cidr = format!("{}/{}", switch.ip, switch.mask);
        let ports = self.db.get_switch_ports_for_switch(&switch.name)?;

        // NOTE: Apply per-port flows before switch-wide broadcast/drop flows.
        for port in &ports {
            self.apply_switch_port_flows(&self.config, switch, &ports, port)?;
        }

        let port_outputs: Vec<String> = ports
            .iter()
            .map(|p| format!("output:{}", p.interface_name))
            .collect();

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=239,arp,dl_dst=ff:ff:ff:ff:ff:ff,arp_spa={},arp_tpa={},actions={}",
                cidr,
                cidr,
                port_outputs.join(","),
            ),
        ])?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=235,ip,nw_src={},nw_dst={},actions=drop",
                cidr, cidr,
            ),
        ])?;

        Ok(())
    }

    /// Apply flows for a specific switch port.
    pub fn apply_switch_port_flows(
        &self,
        config: &Config,
        switch: &Switch,
        ports: &[SwitchPort],
        port: &SwitchPort,
    ) -> Result<()> {
        let bridge = &config.bridge_name;

        let cidr = format!("{}/{}", switch.ip, switch.mask);

        // NOTE: Priority 251/250 enforce intra-subnet path rules before destination matches.
        let other_ports: Vec<String> = ports
            .iter()
            .filter(|p| p.name != port.name)
            .map(|p| format!("output:{}", p.interface_name))
            .collect();

        let actions = if other_ports.is_empty() {
            "drop".to_string()
        } else {
            other_ports.join(",")
        };

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=251,ip,in_port={},dl_src={},nw_src={},nw_dst={},actions={}",
                port.interface_name, port.mac, cidr, cidr, actions,
            ),
        ])?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=250,ip,in_port={},nw_src={},nw_dst={},actions=drop",
                port.interface_name, cidr, cidr,
            ),
        ])?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=240,ip,dl_dst={},nw_dst={},actions=output:{}",
                port.mac, port.ip, port.interface_name,
            ),
        ])?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=240,arp,dl_dst={},arp_tpa={},actions=output:{}",
                port.mac, port.ip, port.interface_name,
            ),
        ])?;

        Ok(())
    }

    /// Delete all flows for a switch.
    pub fn delete_switch_flows(&self, config: &Config, switch: &Switch) -> Result<()> {
        let bridge = &config.bridge_name;

        let cidr = format!("{}/{}", switch.ip, switch.mask);

        let ports = self.db.get_switch_ports_for_switch(&switch.name)?;
        for port in &ports {
            let _ = self.delete_switch_port_flows(config, switch, port);
        }

        // NOTE: Delete specific flows by match to avoid removing unrelated flows if multiple
        // switches share a bridge.
        for match_str in [
            format!(
                "priority=239,arp,dl_dst=ff:ff:ff:ff:ff:ff,arp_spa={},arp_tpa={}",
                cidr, cidr
            ),
            format!("priority=235,ip,nw_src={},nw_dst={}", cidr, cidr),
        ] {
            let _ = ovs_ofctl(&["del-flows", bridge, &match_str]);
        }

        Ok(())
    }

    /// Delete flows for a specific switch port.
    pub fn delete_switch_port_flows(
        &self,
        config: &Config,
        switch: &Switch,
        port: &SwitchPort,
    ) -> Result<()> {
        let bridge = &config.bridge_name;
        let cidr = format!("{}/{}", switch.ip, switch.mask);

        for match_str in [
            format!(
                "priority=251,ip,in_port={},dl_src={},nw_src={},nw_dst={}",
                port.interface_name, port.mac, cidr, cidr
            ),
            format!(
                "priority=250,ip,in_port={},nw_src={},nw_dst={}",
                port.interface_name, cidr, cidr
            ),
            format!("priority=240,ip,dl_dst={},nw_dst={}", port.mac, port.ip),
            format!("priority=240,arp,dl_dst={},arp_tpa={}", port.mac, port.ip),
        ] {
            let _ = ovs_ofctl(&["del-flows", bridge, &match_str]);
        }

        Ok(())
    }
}

/// Run ovs-vsctl and return stdout on success.
pub fn ovs_vsctl(args: &[&str]) -> Result<String> {
    if let Ok(hook) = OVS_VSCTL_HOOK.lock() {
        if let Some(ref f) = *hook {
            return f(args);
        }
    }
    let output = Command::new("ovs-vsctl").args(args).output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "ovs-vsctl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run ovs-ofctl and return stdout on success.
pub fn ovs_ofctl(args: &[&str]) -> Result<String> {
    if let Ok(hook) = OVS_OFTCTL_HOOK.lock() {
        if let Some(ref f) = *hook {
            return f(args);
        }
    }
    let output = Command::new("ovs-ofctl").args(args).output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "ovs-ofctl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
