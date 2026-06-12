use crate::cidr::Ipv4Network;
use crate::database::{Database, RouterRoute, Subnet, Switch, SwitchPort, ensure_mac_or_generate};
use crate::ebpf::BridgePlane;
use crate::interfaces::Interface;
use anyhow::{Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone)]
pub struct SwitchRouterOps {
    db: Arc<Database>,
    plane: Arc<BridgePlane>,
}

impl SwitchRouterOps {
    pub const fn new(db: Arc<Database>, plane: Arc<BridgePlane>) -> Self {
        Self { db, plane }
    }

    pub fn create_switch(&self, name: &str) -> Result<Switch> {
        self.db.create_switch(name)
    }

    pub fn remove_switch(&self, name: &str) -> Result<()> {
        self.db.remove_switch(name)
    }

    pub fn list_switches(&self) -> Result<Vec<Switch>> {
        self.db.list_switches()
    }

    pub fn add_subnet(
        &self,
        switch: &str,
        name: &str,
        cidr: &str,
        gateway_ip: Option<&str>,
        gateway_mac: Option<&str>,
    ) -> Result<Subnet> {
        let gateway_ip = match gateway_ip {
            Some(ip) => ip.to_string(),
            None => default_gateway_ip_from_cidr(cidr)?,
        };
        let gateway_mac = ensure_mac_or_generate(gateway_mac);
        self.db
            .create_subnet(switch, name, cidr, &gateway_ip, &gateway_mac)
    }

    pub fn list_subnets(&self, switch: &str) -> Result<Vec<Subnet>> {
        self.db.list_subnets(switch)
    }

    pub fn add_switch_port(
        &self,
        switch: &str,
        subnet: &str,
        name: &str,
        tap: &str,
        mac: Option<&str>,
        ip: Option<&str>,
    ) -> Result<SwitchPort> {
        let mac = ensure_mac_or_generate(mac);
        let ip = match ip {
            Some(v) => v.to_string(),
            None => self.db.allocate_switch_port_ip(switch, subnet)?,
        };
        self.db
            .create_switch_port(switch, subnet, name, tap, &ip, &mac)
    }

    pub fn remove_switch_port(&self, switch: &str, name: &str) -> Result<()> {
        let ep = self.db.get_switch_port(switch, name)?;
        if let Ok(ifindex) = get_ifindex(&ep.tap_name) {
            let _ = self.plane.remove_bridge_member(ifindex);
            let _ = self.plane.unregister_gateway(ifindex);
        }
        self.plane.detach_tap(&ep.tap_name)?;
        Interface::delete(&ep.tap_name)?;
        self.db.remove_switch_port(switch, name)
    }

    pub fn list_switch_ports(&self, switch: &str) -> Result<Vec<SwitchPort>> {
        self.db.list_switch_ports(switch)
    }

    pub fn sync(&self) -> Result<()> {
        self.plane.clear_routes()?;
        self.plane.clear_arp_entries()?;

        let switches = self.db.list_switches()?;
        let switch_name_by_uuid: HashMap<String, String> = switches
            .iter()
            .map(|n| (n.uuid.clone(), n.name.clone()))
            .collect();

        let mut subnet_by_uuid: HashMap<String, Subnet> = HashMap::new();
        for net in &switches {
            for subnet in self.db.list_subnets_for_switch_uuid(&net.uuid)? {
                subnet_by_uuid.insert(subnet.uuid.clone(), subnet);
            }
        }
        let switch_ports = self.db.list_all_switch_ports()?;
        let mut gateway_enabled_networks: HashSet<String> = HashSet::new();
        for router in self.db.list_routers()? {
            for attachment in self.db.list_router_ports_for_router(&router.name)? {
                gateway_enabled_networks.insert(attachment.switch_name);
            }
        }

        // Clear stale gateway/interface entries before rebuilding desired state.
        for ep in &switch_ports {
            if let Ok(ifindex) = get_ifindex(&ep.tap_name) {
                let _ = self.plane.unregister_gateway(ifindex);
            }
        }

        for ep in &switch_ports {
            if !Interface::exists(&ep.tap_name) {
                Interface::create(&ep.tap_name, Some(&ep.mac))?;
            }
            Interface::up(&ep.tap_name)?;
            self.plane.attach_tap(&ep.tap_name)?;

            if let Ok(ifindex) = get_ifindex(&ep.tap_name) {
                let subnet = subnet_by_uuid.get(&ep.subnet_uuid).ok_or_else(|| {
                    anyhow!(
                        "missing subnet '{}' for switch port '{}'",
                        ep.subnet_uuid,
                        ep.name
                    )
                })?;
                let bridge_id = bridge_id(&ep.switch_uuid, &subnet.uuid);
                self.plane.set_bridge_member(ifindex, bridge_id)?;
                self.plane.register_arp_entry(
                    bridge_id,
                    u32::from_be_bytes(parse_ipv4(&ep.ip)?),
                    parse_mac(&ep.mac)?,
                )?;

                if let Some(switch_name) = switch_name_by_uuid.get(&ep.switch_uuid)
                    && gateway_enabled_networks.contains(switch_name)
                {
                    let gw_ip = parse_ipv4(&subnet.gateway_ip)?;
                    let gw_mac = parse_mac(&subnet.gateway_mac)?;
                    self.plane
                        .register_arp_entry(bridge_id, u32::from_be_bytes(gw_ip), gw_mac)?;
                    self.plane
                        .register_gateway(ifindex, u32::from_be_bytes(gw_ip), gw_mac)?;
                }
            }
        }

        self.sync_router_routes(&switch_name_by_uuid, &subnet_by_uuid, &switch_ports)?;

        Ok(())
    }

    fn sync_router_routes(
        &self,
        switch_name_by_uuid: &HashMap<String, String>,
        subnet_by_uuid: &HashMap<String, Subnet>,
        switch_ports: &[SwitchPort],
    ) -> Result<()> {
        for router in self.db.list_routers()? {
            let attachments = self.db.list_router_ports_for_router(&router.name)?;
            let attached_switches: HashSet<String> =
                attachments.into_iter().map(|p| p.switch_name).collect();
            let routes = self.db.list_router_routes(&router.name)?;
            for route in routes {
                self.program_router_route(
                    &route,
                    &attached_switches,
                    switch_name_by_uuid,
                    subnet_by_uuid,
                    switch_ports,
                )?;
            }
        }
        Ok(())
    }

    fn program_router_route(
        &self,
        route: &RouterRoute,
        attached_switches: &HashSet<String>,
        switch_name_by_uuid: &HashMap<String, String>,
        subnet_by_uuid: &HashMap<String, Subnet>,
        switch_ports: &[SwitchPort],
    ) -> Result<()> {
        let Some(next_hop_mac) = route.next_hop_mac.as_deref() else {
            return Ok(());
        };
        let Some(next_hop_ip) = route.next_hop.as_deref() else {
            return Ok(());
        };

        let source = parse_cidr_v4(&route.source)?;
        let destination = parse_cidr_v4(&route.destination)?;
        let next_hop = parse_ipv4(next_hop_ip)?;
        let next_hop_addr = std::net::Ipv4Addr::from(next_hop);
        let next_hop_mac = parse_mac(next_hop_mac)?;

        let mut has_ingress = false;
        let mut egress = None::<(u32, [u8; 6])>;

        for port in switch_ports {
            let Some(subnet) = subnet_by_uuid.get(&port.subnet_uuid) else {
                continue;
            };
            let Some(switch_name) = switch_name_by_uuid.get(&port.switch_uuid) else {
                continue;
            };
            if !attached_switches.contains(switch_name.as_str()) {
                continue;
            }
            let Ok(ifindex) = get_ifindex(&port.tap_name) else {
                continue;
            };
            let Ok(port_ip) = port.ip.parse::<std::net::Ipv4Addr>() else {
                continue;
            };

            if source.contains(port_ip) {
                has_ingress = true;
            }

            let Ok(subnet_cidr) = subnet.cidr.parse::<Ipv4Network>() else {
                continue;
            };
            if subnet_cidr.contains(next_hop_addr) {
                egress = Some((ifindex, parse_mac(&port.mac)?));
            }
        }

        let Some((egress_ifindex, src_mac)) = egress else {
            return Ok(());
        };
        if !has_ingress {
            return Ok(());
        }

        self.plane.add_route(
            source.network().octets(),
            u32::from(source.prefix()),
            destination.network().octets(),
            u32::from(destination.prefix()),
            egress_ifindex,
            next_hop_mac,
            src_mac,
            route
                .metric
                .try_into()
                .map_err(|_| anyhow!("route metric '{}' exceeds u8 range", route.metric))?,
        )
    }
}

fn bridge_id(network_uuid: &str, subnet_uuid: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for b in network_uuid.as_bytes().iter().chain(subnet_uuid.as_bytes()) {
        hash ^= u32::from(*b);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

fn get_ifindex(name: &str) -> Result<u32> {
    let path = format!("/sys/class/net/{name}/ifindex");
    let content = std::fs::read_to_string(path)?;
    content
        .trim()
        .parse::<u32>()
        .map_err(|e| anyhow!("invalid ifindex for '{name}': {e}"))
}

fn parse_mac(mac: &str) -> Result<[u8; 6]> {
    let mut bytes = [0u8; 6];
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return Err(anyhow!("invalid mac '{mac}'"));
    }
    for (byte, part) in bytes.iter_mut().zip(parts) {
        *byte = u8::from_str_radix(part, 16).map_err(|e| anyhow!("invalid mac '{mac}': {e}"))?;
    }
    Ok(bytes)
}

fn parse_ipv4(ip: &str) -> Result<[u8; 4]> {
    ip.parse::<std::net::Ipv4Addr>()
        .map(|v| v.octets())
        .map_err(|e| anyhow!("invalid ipv4 '{ip}': {e}"))
}

fn parse_cidr_v4(cidr: &str) -> Result<Ipv4Network> {
    cidr.parse()
        .map_err(|e| anyhow!("invalid cidr '{cidr}': {e}"))
}

fn default_gateway_ip_from_cidr(cidr: &str) -> Result<String> {
    let network: Ipv4Network = cidr
        .parse()
        .map_err(|e| anyhow!("invalid cidr '{cidr}': {e}"))?;

    let prefix = network.prefix();
    if prefix >= 31 {
        return Err(anyhow!(
            "cannot auto-select gateway for cidr '{cidr}': no usable host addresses"
        ));
    }

    let gw = u32::from(network.network()).wrapping_add(1);
    Ok(std::net::Ipv4Addr::from(gw).to_string())
}
