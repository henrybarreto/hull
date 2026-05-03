use crate::config::Config;
use crate::database::{Database, Switch, SwitchPort};
use crate::interfaces::Interface;
use crate::of;
use crate::utils::{FlowCookieKind, flow_cookie};
use anyhow::{Result, anyhow};
use ipnetwork::IpNetwork;
use serde_json::json;
use std::net::IpAddr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, trace};

use crate::openflow::protocol::action::Action;
use crate::openflow::protocol::constants::{ETH_TYPE_ARP, ETH_TYPE_IPV4};
use crate::openflow::protocol::instruction::Instruction;
use crate::openflow::protocol::ofmatch::Match;
use crate::openflow::protocol::oxm;
use crate::openflow::protocol::rule::Rule;

/// Switch CRUD and flow programming operations.
pub struct SwitchOps {
    db: Arc<Database>,
    config: Arc<Config>,
    ovs: Arc<crate::ovs::BridgeClient>,
}

impl SwitchOps {
    /// Create a new switch operations instance.
    pub const fn new(
        db: Arc<Database>,
        config: Arc<Config>,
        ovs: Arc<crate::ovs::BridgeClient>,
    ) -> Self {
        Self { db, config, ovs }
    }

    fn switch_cookie(switch: &Switch) -> Result<u64> {
        flow_cookie(FlowCookieKind::Switch, &switch.uuid)
    }

    fn add_flow(cookie: u64, priority: u16, of_match: Match, actions: Vec<Action>) -> Rule {
        Rule::add(
            0,
            0,
            priority,
            of_match,
            vec![Instruction::apply_actions(actions)],
        )
        .with_cookie(cookie)
    }

    fn wait_for_ofport(&self, interface_name: &str) -> Result<u32> {
        for _ in 0..50 {
            if let Some(ofport) = self.ovs.interface_ofport(interface_name)? {
                return Ok(ofport);
            }
            thread::sleep(Duration::from_millis(50));
        }

        Err(anyhow!(
            "interface '{interface_name}' did not receive an OVS ofport"
        ))
    }

    fn insert_flow(&self, of: &mut of::OF, flow: Rule) -> Result<()> {
        of.insert(flow)
    }

    fn remove_flows(&self, of: &mut of::OF, cookie: u64) -> Result<()> {
        of.remove(Some(cookie))
    }

    /// Create a switch and apply its flows.
    ///
    /// # Errors
    /// Returns an error if the switch already exists, database updates fail, or flow programming fails.
    pub fn create(&self, name: &str, ip: &str, mask: u8) -> Result<()> {
        debug!(switch = %name, ip = %ip, mask, "creating switch");
        if self.db.get_switch(name).is_ok() {
            return Err(anyhow!("Switch '{name}' already exists"));
        }

        let switch = self.db.create_switch(name, ip, mask)?;

        self.apply_switch_flows(&self.config, &switch)?;

        Ok(())
    }

    /// # Errors
    /// Returns an error if the switch does not exist, database updates fail, or flow deletion fails.
    pub fn remove(&self, name: &str) -> Result<()> {
        debug!(switch = %name, "removing switch");
        if self.db.get_switch(name).is_err() {
            return Err(anyhow!("Switch '{name}' does not exist"));
        }

        let switch = self.db.get_switch(name)?;

        self.delete_switch_flows(&self.config, &switch)?;
        self.db.remove_switch(&switch.name)?;

        Ok(())
    }

    /// List all switches.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn list(&self) -> Result<Vec<crate::database::Switch>> {
        trace!("listing switches");
        self.db.list_switches()
    }

    /// Fetch a switch by name.
    ///
    /// # Errors
    /// Returns an error if the switch is not found or the database query fails.
    pub fn get(&self, name: &str) -> Result<crate::database::Switch> {
        trace!(switch = %name, "fetching switch");
        self.db.get_switch(name)
    }

    /// Create a switch port and apply updated flows.
    ///
    /// # Errors
    /// Returns an error if the port already exists, the switch or interface is missing,
    /// or flow programming fails.
    pub fn create_switch_port(&self, name: &str, switch: &str, interface_name: &str) -> Result<()> {
        debug!(port = %name, switch = %switch, interface = %interface_name, "creating switch port");
        if self.db.get_switch_port(name).is_ok() {
            return Err(anyhow!("Switch port '{name}' already exists"));
        }

        if !Interface::exists(interface_name) {
            return Err(anyhow!(
                "Interface '{interface_name}' does not exist on system"
            ));
        }

        // TODO: Avoid multiple switch ports using the same interface.

        let s = self.db.get_switch(switch)?;
        let interface = self.db.get_interface(interface_name)?;

        let ip = self.allocate_ip(&s.name, &s.ip, s.mask)?;

        let port = self
            .db
            .create_switch_port(name, &s.name, &interface.name, &ip)?;

        self.ovs.add_port(
            &self.config.bridge_name,
            &port.interface_name,
            json!({
                "hull-switch": switch,
                "hull-port": name,
            }),
        )?;

        self.delete_switch_flows(&self.config, &s)?;
        self.apply_switch_flows(&self.config, &s)?;

        Ok(())
    }

    /// Remove a switch port and apply updated flows.
    ///
    /// # Errors
    /// Returns an error if the switch or port does not exist, database updates fail,
    /// or flow programming fails.
    pub fn remove_switch_port(&self, switch: &str, name: &str) -> Result<()> {
        debug!(port = %name, switch = %switch, "removing switch port");
        if self.db.get_switch(switch).is_err() {
            return Err(anyhow!("Switch '{switch}' does not exist"));
        }

        if self.db.get_switch_port(name).is_err() {
            return Err(anyhow!("Switch port '{name}' does not exist"));
        }

        // NOTE: Check if switch port is attached to a switch before deleting.

        let switch = self.db.get_switch(switch)?;
        let port = self.db.get_switch_port(name)?;

        self.delete_switch_flows(&self.config, &switch)?;

        let _ = self
            .ovs
            .del_port(&self.config.bridge_name, &port.interface_name);

        self.db.remove_switch_port(&port.name)?;
        self.apply_switch_flows(&self.config, &switch)?;

        Ok(())
    }

    /// List all switch ports.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn list_switch_ports(&self) -> Result<Vec<crate::database::SwitchPort>> {
        trace!("listing switch ports");
        self.db.list_switch_ports()
    }

    /// Fetch a switch port by name.
    ///
    /// # Errors
    /// Returns an error if the port is not found or the database query fails.
    pub fn get_switch_port(&self, name: &str) -> Result<crate::database::SwitchPort> {
        trace!(port = %name, "fetching switch port");
        self.db.get_switch_port(name)
    }

    /// Allocate an unused IP from the switch subnet.
    fn allocate_ip(&self, switch_name: &str, switch_ip: &str, switch_mask: u8) -> Result<String> {
        let network_str = format!("{switch_ip}/{switch_mask}");
        let network: IpNetwork = network_str
            .parse()
            .map_err(|e| anyhow!("invalid switch network: {e}"))?;

        let existing_ports = self.db.get_switch_ports_for_switch(switch_name)?;
        let used_ips: std::collections::HashSet<String> =
            existing_ports.into_iter().map(|p| p.ip).collect();

        for ip in &network {
            let ip_str = ip.to_string();

            if ip == network.network() {
                continue;
            }
            if let IpNetwork::V4(v4) = network
                && ip == IpAddr::V4(v4.broadcast())
            {
                continue;
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
    ///
    /// # Errors
    /// Returns an error if the database query or flow programming fails.
    pub fn sync(&self) -> Result<()> {
        debug!("syncing switches from database");
        let switches = self.db.list_switches()?;
        for switch in switches {
            trace!(switch = %switch.name, "syncing switch");
            let ports = self.db.get_switch_ports_for_switch(&switch.name)?;
            for port in &ports {
                trace!(switch = %switch.name, port = %port.name, "ensuring switch port is attached");
                self.ovs.add_port(
                    &self.config.bridge_name,
                    &port.interface_name,
                    json!({
                        "hull-switch": switch.name,
                        "hull-port": port.name,
                    }),
                )?;
            }

            self.delete_switch_flows(&self.config, &switch)?;
            self.apply_switch_flows(&self.config, &switch)?;
        }

        Ok(())
    }

    /// Apply all flows for a switch.
    ///
    /// # Errors
    /// Returns an error if flow programming fails or the switch subnet cannot be parsed.
    pub fn apply_switch_flows(&self, config: &Config, switch: &Switch) -> Result<()> {
        trace!(switch = %switch.name, "applying switch flows");
        let mut of = of::OF::connect(&config.bridge_name)?;
        let cookie = Self::switch_cookie(switch)?;
        let ports = self.db.get_switch_ports_for_switch(&switch.name)?;
        let switch_ip = parse_ipv4(&switch.ip)?;
        let switch_mask = mask_to_ipv4_mask(switch.mask);

        // NOTE: Apply per-port flows before switch-wide drop flows.
        for port in &ports {
            trace!(switch = %switch.name, port = %port.name, "applying port flows");
            self.apply_switch_port_flows(&mut of, switch, &ports, port, cookie)?;
        }

        let drop_match = Match::new(vec![
            oxm::eth_type(ETH_TYPE_IPV4),
            oxm::ipv4_src_masked(switch_ip, switch_mask),
            oxm::ipv4_dst_masked(switch_ip, switch_mask),
        ]);
        self.insert_flow(&mut of, Self::add_flow(cookie, 235, drop_match, Vec::new()))?;

        Ok(())
    }

    /// Apply flows for a specific switch port.
    ///
    /// # Errors
    /// Returns an error if flow programming fails or any IP or MAC address is invalid.
    pub fn apply_switch_port_flows(
        &self,
        of: &mut of::OF,
        switch: &Switch,
        ports: &[SwitchPort],
        port: &SwitchPort,
        cookie: u64,
    ) -> Result<()> {
        trace!(switch = %switch.name, port = %port.name, "building port flows");
        let port_ofport = self.wait_for_ofport(&port.interface_name)?;
        let mut other_ports = Vec::new();
        for p in ports.iter().filter(|p| p.name != port.name) {
            let ofport = self.wait_for_ofport(&p.interface_name)?;
            other_ports.push(Action::output(ofport));
        }
        let port_ip = parse_ipv4(&port.ip)?;
        let switch_ip = parse_ipv4(&switch.ip)?;
        let switch_mask = mask_to_ipv4_mask(switch.mask);
        let port_mac = parse_mac(&port.mac)?;

        self.insert_flow(
            of,
            Self::add_flow(
                cookie,
                250,
                Match::new(vec![
                    oxm::eth_type(ETH_TYPE_IPV4),
                    oxm::in_port(port_ofport),
                    oxm::ipv4_src(port_ip),
                    oxm::ipv4_dst_masked(switch_ip, switch_mask),
                ]),
                other_ports,
            ),
        )?;

        self.insert_flow(
            of,
            Self::add_flow(
                cookie,
                245,
                Match::new(vec![
                    oxm::eth_type(ETH_TYPE_IPV4),
                    oxm::in_port(port_ofport),
                    oxm::ipv4_src_masked(switch_ip, switch_mask),
                    oxm::ipv4_dst_masked(switch_ip, switch_mask),
                ]),
                Vec::new(),
            ),
        )?;

        self.insert_flow(
            of,
            Self::add_flow(
                cookie,
                240,
                Match::new(vec![
                    oxm::eth_type(ETH_TYPE_IPV4),
                    oxm::eth_dst(port_mac),
                    oxm::ipv4_dst(port_ip),
                ]),
                vec![Action::output(port_ofport)],
            ),
        )?;

        self.insert_flow(
            of,
            Self::add_flow(
                cookie,
                240,
                Match::new(vec![oxm::eth_type(ETH_TYPE_ARP), oxm::arp_tpa(port_ip)]),
                vec![Action::output(port_ofport)],
            ),
        )?;

        Ok(())
    }

    /// Delete all flows for a switch.
    ///
    /// # Errors
    /// Returns an error if flow deletion fails.
    pub fn delete_switch_flows(&self, config: &Config, switch: &Switch) -> Result<()> {
        debug!(switch = %switch.name, "deleting switch flows");
        let mut of = of::OF::connect(&config.bridge_name)?;
        let cookie = Self::switch_cookie(switch)?;
        let _ = self.remove_flows(&mut of, cookie);

        Ok(())
    }
}

fn parse_ipv4(ip: &str) -> Result<[u8; 4]> {
    trace!(ip = %ip, "parsing ipv4");
    Ok(ip.parse::<std::net::Ipv4Addr>()?.octets())
}

fn mask_to_ipv4_mask(mask: u8) -> [u8; 4] {
    trace!(mask, "converting mask to ipv4 mask");
    if mask == 0 {
        return [0, 0, 0, 0];
    }
    (!0u32 << (32 - u32::from(mask))).to_be_bytes()
}

fn parse_mac(mac: &str) -> Result<[u8; 6]> {
    trace!(mac = %mac, "parsing mac");
    let mut bytes = [0u8; 6];
    let mut parts = mac.split(':');
    for byte in &mut bytes {
        let part = parts
            .next()
            .ok_or_else(|| anyhow!("invalid MAC '{mac}': expected 6 octets"))?;
        *byte = u8::from_str_radix(part, 16).map_err(|e| anyhow!("invalid MAC '{mac}': {e}"))?;
    }
    if parts.next().is_some() {
        return Err(anyhow!("invalid MAC '{mac}': expected 6 octets"));
    }
    Ok(bytes)
}
