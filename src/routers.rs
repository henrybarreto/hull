use crate::config::Config;
use crate::database::Database;
use crate::interfaces::Interface;
use crate::switches::{ovs_ofctl, ovs_vsctl};
use crate::utils::generate_deterministic_mac;
use anyhow::{Result, anyhow};
use std::sync::Arc;

pub struct RouterOps {
    db: Arc<Database>,
    config: Arc<Config>,
}

impl RouterOps {
    /// Create a new router operations instance.
    pub fn new(db: Arc<Database>, config: Arc<Config>) -> Self {
        Self { db, config }
    }

    /// Create a router and apply its flows.
    pub fn create(&self, name: &str) -> Result<()> {
        if self.db.get_router(name).is_ok() {
            return Err(anyhow!("Router '{}' already exists", name));
        }

        let router = self.db.create_router(name)?;
        self.apply_router_flows(&router.name)?;

        Ok(())
    }

    /// Remove a router and delete its flows.
    pub fn remove(&self, name: &str) -> Result<()> {
        if self.db.get_router(name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", name));
        }

        let router = self.db.get_router(name)?;

        self.delete_router_flows(&router.name)?;
        self.db.remove_router(&router.name)?;

        Ok(())
    }

    /// List all routers.
    pub fn list(&self) -> Result<Vec<crate::database::Router>> {
        self.db.list_routers()
    }

    /// List switches attached to a router.
    pub fn list_attached_switches(&self, name: &str) -> Result<Vec<String>> {
        if self.db.get_router(name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", name));
        }

        let router_ports = self.db.list_router_ports_for_router(name)?;

        let mut switch_names = Vec::new();
        for router_port in router_ports {
            switch_names.push(router_port.switch_name);
        }

        Ok(switch_names)
    }

    /// Attach a switch to a router and apply updated flows.
    pub fn attach(&self, router_name: &str, switch_name: &str) -> Result<()> {
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", router_name));
        }

        if self.db.get_switch(switch_name).is_err() {
            return Err(anyhow!("Switch '{}' does not exist", switch_name));
        }

        if let Ok(switches) = self.list_attached_switches(router_name) {
            if switches.contains(&switch_name.to_string()) {
                return Err(anyhow!(
                    "Router '{}' is already attached to switch '{}'",
                    router_name,
                    switch_name
                ));
            }
        }

        let router = self.db.get_router(router_name)?;
        let switch = self.db.get_switch(switch_name)?;

        let router_ports = self.db.list_router_ports_for_router(&router.name)?;
        let router_port = router_ports.iter().find(|p| p.switch_name == switch.name);
        if router_port.is_some() {
            return Err(anyhow!(
                "Router '{}' is already attached to switch '{}'",
                router_name,
                switch_name
            ));
        }

        self.delete_router_flows(&router.name)?;

        // TODO: This is inefficient, we delete and re-apply all flows for the router on every
        // attach/detach.
        self.db
            .create_router_port(&router.name, &switch.name, None, None)?;
        self.apply_router_flows(&router.name)?;

        Ok(())
    }

    /// Detach a switch from a router and apply updated flows.
    pub fn detach(&self, router_name: &str, switch_name: &str) -> Result<()> {
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", router_name));
        }

        if self.db.get_switch(switch_name).is_err() {
            return Err(anyhow!("Switch '{}' does not exist", switch_name));
        }

        if let Ok(switches) = self.list_attached_switches(router_name) {
            if !switches.contains(&switch_name.to_string()) {
                return Err(anyhow!(
                    "Router '{}' is not attached to switch '{}'",
                    router_name,
                    switch_name
                ));
            }
        }

        let router = self.db.get_router(router_name)?;
        let switch = self.db.get_switch(switch_name)?;

        self.delete_router_flows(&router.name)?;

        // TODO: This is inefficient, we delete and re-apply all flows for the router on every
        // attach/detach.
        self.db.remove_router_port(&router.name, &switch.name)?;
        self.apply_router_flows(&router.name)?;

        Ok(())
    }

    /// Configure or clear a router uplink.
    pub fn set_link(&self, router_name: &str, name: &str, ip: &str, mac: &str) -> Result<()> {
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", router_name));
        }

        if !Interface::exists(name) {
            return Err(anyhow!(
                "Uplink interface '{}' does not exist on system",
                name
            ));
        }

        if self.db.get_router_link(name).is_ok() {
            return Err(anyhow!(
                "Uplink interface '{}' is already in use by another router",
                name
            ));
        }

        let router = self.db.get_router(router_name)?;

        self.db
            .update_router_link(&router.name, Some(name), Some(ip), Some(mac))?;

        let _ = ovs_vsctl(&["--may-exist", "add-port", &self.config.bridge_name, name]);

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.delete_router_flows(&router.name)?;
        self.apply_router_flows(&router.name)?;

        Ok(())
    }

    pub fn unset_link(&self, router_name: &str) -> Result<()> {
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", router_name));
        }

        if self.db.get_router_link(router_name).is_err() {
            return Err(anyhow!(
                "Router '{}' does not have an uplink configured",
                router_name
            ));
        }

        let router = self.db.get_router(router_name)?;

        self.db.update_router_link(&router.name, None, None, None)?;

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.delete_router_flows(&router.name)?;
        self.apply_router_flows(&router.name)?;

        Ok(())
    }

    /// Re-apply all router flows from database state.
    pub fn sync(&self) -> Result<()> {
        let routers = self.db.list_routers()?;
        for router in &routers {
            if let Some(link_name) = &router.link_name {
                ovs_vsctl(&[
                    "--may-exist",
                    "add-port",
                    &self.config.bridge_name,
                    link_name,
                ])?;
            }

            self.delete_router_flows(&router.name)?;
            self.apply_router_flows(&router.name)?;
        }

        Ok(())
    }

    /// Apply all router flows for attached switches and optional uplink.
    pub fn apply_router_flows(&self, name: &str) -> Result<()> {
        let bridge = &self.config.bridge_name;

        let router = self.db.get_router(name)?;
        let router_ports = self.db.list_router_ports_for_router(name)?;

        let mut gateways = std::collections::HashMap::new();
        for router_port in &router_ports {
            let switch = self.db.get_switch(&router_port.switch_name)?;

            let gateway_mac = generate_deterministic_mac(name, &router_port.switch_name);
            let gateawy_ip = compute_gateway_ip(&switch.ip)?;

            self.apply_router_gateway_arp_flows((gateway_mac.clone(), gateawy_ip.clone()))?;

            gateways.insert(router_port.switch_name.clone(), (gateway_mac, gateawy_ip));
        }

        for rp in &router_ports {
            let switch = self.db.get_switch(&rp.switch_name)?;

            self.apply_router_gateway_inter_subnet_flows(&switch, &gateways)?;
        }

        if let (Some(link_name), Some(link_mac), Some(link_ip)) = (
            router.link_name.as_ref(),
            router.link_mac.as_ref(),
            router.link_ip.as_ref(),
        ) {
            if let Ok(port) = get_ofport(bridge, link_name) {
                self.apply_router_uplink_flows(link_mac, link_ip, &port)?;

                let mut seen_switches = std::collections::HashSet::new();
                for rp in &router_ports {
                    if !seen_switches.insert(rp.switch_name.clone()) {
                        continue;
                    }

                    let switch = self.db.get_switch(&rp.switch_name)?;
                    let cidr = format!("{}/{}", switch.ip, switch.mask);

                    self.apply_router_link_flows(&router, &switch, &cidr, &port)?;
                }
            } else {
                eprintln!(
                    "WARNING: Could not get ofport for link '{}', skipping ARP flows",
                    link_name
                );
            }
        }

        Ok(())
    }

    /// Apply gateway flows and inter-subnet routing for one switch.
    fn apply_router_gateway_arp_flows(&self, gateway: (String, String)) -> Result<()> {
        let bridge = &self.config.bridge_name;

        let gateway_mac_hex = mac_to_hex(&gateway.0)?;
        let gateway_ip_hex = ip_to_hex(&gateway.1)?;

        // NOTE: These responder flows make the router act as the gateway for the subnet.
        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=280, arp,arp_tpa={},arp_op=1, actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[], mod_dl_src:{},load:0x2->NXM_OF_ARP_OP[], move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[], load:0x{}->NXM_NX_ARP_SHA[], move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[], load:0x{}->NXM_OF_ARP_SPA[],IN_PORT",
                &gateway.1, &gateway.0, gateway_mac_hex, gateway_ip_hex,
            ),
        ])?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=280, icmp,nw_dst={},icmp_type=8, actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[], mod_dl_src:{}, move:NXM_OF_IP_SRC[]->NXM_OF_IP_DST[], mod_nw_src:{},load:0->NXM_OF_ICMP_TYPE[],IN_PORT",
                &gateway.1, &gateway.0, &gateway.1
            ),
        ])?;

        Ok(())
    }

    fn apply_router_gateway_inter_subnet_flows(
        &self,
        switch: &crate::database::Switch,
        gateways: &std::collections::HashMap<String, (String, String)>,
    ) -> Result<()> {
        let bridge = &self.config.bridge_name;

        let (to_gateway_mac, _) = gateways.get(&switch.name).unwrap();

        for other_switch_name in gateways.keys() {
            if other_switch_name == &switch.name {
                continue;
            }

            let (from_gateway_mac, _) = gateways.get(other_switch_name).unwrap();

            let other_ports = self.db.get_switch_ports_for_switch(other_switch_name)?;
            for sp in other_ports {
                ovs_ofctl(&[
                    "add-flow",
                    bridge,
                    &format!(
                        "priority=265, ip,dl_dst={},nw_dst={},actions=mod_dl_src:{},mod_dl_dst:{},dec_ttl,NORMAL",
                        to_gateway_mac, sp.ip, from_gateway_mac, sp.mac,
                    ),
                ])?;
            }
        }

        Ok(())
    }

    /// Apply uplink flows (ARP responder and NAT return).
    fn apply_router_uplink_flows(&self, mac: &str, ip: &str, port: &str) -> Result<()> {
        let bridge = &self.config.bridge_name;

        let mac_hex = mac_to_hex(mac)?;
        let ip_hex = ip_to_hex(ip)?;

        // ARP Responder for the router's external IP on the link.
        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=231, arp,in_port={},arp_tpa={},arp_op=1, actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[], mod_dl_src:{},load:0x2->NXM_OF_ARP_OP[], move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[], load:0x{}->NXM_NX_ARP_SHA[], move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[], load:0x{}->NXM_OF_ARP_SPA[],IN_PORT",
                port, ip, mac, mac_hex, ip_hex,
            ),
        ])?;

        // NAT return flow: Un-NAT packets returning from the external network and resubmit them.
        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=235,in_port={},ip,nw_dst={},actions=ct(zone=1,nat,table=0)",
                port, ip
            ),
        ])?;

        Ok(())
    }

    /// Apply uplink SNAT/DNAT flows for one switch subnet.
    fn apply_router_link_flows(
        &self,
        router: &crate::database::Router,
        switch: &crate::database::Switch,
        cidr: &str,
        port: &str,
    ) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let link_name = router.link_name.as_ref().unwrap();
        let switch_ports = self.db.get_switch_ports_for_switch(&switch.name)?;

        // NOTE: Initial pass: send to conntrack to identify state.
        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=224, ip,nw_src={},ct_state=-trk,actions=ct(table=0,zone=1)",
                cidr
            ),
        ])?;

        let mut common_egress_actions = Vec::new();
        if let Some(src_mac) = &router.link_mac {
            common_egress_actions.push(format!("mod_dl_src:{}", src_mac));
        }

        match get_gateway_mac(link_name) {
            Ok(dst_mac) => common_egress_actions.push(format!("mod_dl_dst:{}", dst_mac)),
            Err(e) => eprintln!("WARNING: Gateway MAC lookup failed: {}", e),
        }

        common_egress_actions.push("dec_ttl".to_string());
        common_egress_actions.push(format!("output:{}", port));

        // NOTE: Handle outbound packets based on conntrack state.
        if let Some(link_ip) = &router.link_ip {
            // New connections: Commit with SNAT.
            let mut snat_actions = vec![format!("ct(commit,zone=1,nat(src={}))", link_ip)];
            snat_actions.extend(common_egress_actions.clone());
            ovs_ofctl(&[
                "add-flow",
                bridge,
                &format!(
                    "priority=225, ip,nw_src={},ct_state=+trk+new,actions={}",
                    cidr,
                    snat_actions.join(","),
                ),
            ])?;
        }

        // NOTE: Established connections: Just apply NAT state (un-NAT or no-NAT as committed).
        let mut est_actions = vec!["ct(zone=1,nat)".to_string()];
        est_actions.extend(common_egress_actions);
        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=225, ip,nw_src={},ct_state=+trk+est,actions={}",
                cidr,
                est_actions.join(","),
            ),
        ])?;

        // NOTE: ARP Forwarding (Uplink).
        let switch_port_outputs: Vec<String> = switch_ports
            .iter()
            .map(|sp| format!("output:{}", sp.interface_name))
            .collect();

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=225, in_port={},arp,arp_tpa={},actions={}",
                port,
                cidr,
                switch_port_outputs.join(","),
            ),
        ])?;

        for sp in switch_ports {
            let mut common_ingress_actions = Vec::new();
            if let Some(src_mac) = &router.link_mac {
                common_ingress_actions.push(format!("mod_dl_src:{}", src_mac));
            }

            common_ingress_actions.push(format!("mod_dl_dst:{}", sp.mac));
            common_ingress_actions.push("dec_ttl".to_string());
            common_ingress_actions.push(format!("output:{}", sp.interface_name));

            let mut direct_in_actions = vec!["ct(commit,zone=1)".to_string()];
            direct_in_actions.extend(common_ingress_actions);

            ovs_ofctl(&[
                "add-flow",
                bridge,
                &format!(
                    "priority=230, in_port={},ip,nw_dst={},actions={}",
                    port,
                    sp.ip,
                    direct_in_actions.join(","),
                ),
            ])?;
        }

        Ok(())
    }

    /// Delete all router flows for attached switches and optional uplink.
    pub fn delete_router_flows(&self, name: &str) -> Result<()> {
        let router = self.db.get_router(name)?;
        let router_ports = self.db.list_router_ports_for_router(name)?;
        let bridge = &self.config.bridge_name;

        let mut router_gateways = std::collections::HashMap::new();
        for rp in &router_ports {
            let switch = self.db.get_switch(&rp.switch_name)?;
            let gateway_mac = generate_deterministic_mac(name, &rp.switch_name);
            let cidr = format!("{}/{}", switch.ip, switch.mask);
            router_gateways.insert(rp.switch_name.clone(), (gateway_mac, cidr));
        }

        let link_port = router
            .link_name
            .as_ref()
            .and_then(|link_name| get_ofport(bridge, link_name).ok());

        // NOTE: Link shared flows cleanup applies once per router.
        if let Some(port) = &link_port {
            self.delete_router_uplink_flows(&router, port)?;
        }

        let mut seen_switches = std::collections::HashSet::new();
        for rp in &router_ports {
            if !seen_switches.insert(rp.switch_name.clone()) {
                continue;
            }

            let switch = self.db.get_switch(&rp.switch_name)?;
            self.delete_router_switch_flows(&switch, &router_gateways)?;
            if let Some(ofport) = &link_port {
                self.delete_router_link_flows(&router, &switch, ofport)?;
            }
        }

        Ok(())
    }

    /// Delete shared uplink flows.
    fn delete_router_uplink_flows(
        &self,
        router: &crate::database::Router,
        port: &str,
    ) -> Result<()> {
        let bridge = &self.config.bridge_name;
        if let Some(ext_ip) = &router.link_ip {
            let _ = ovs_ofctl(&[
                "--strict",
                "del-flows",
                bridge,
                &format!(
                    "priority=231,arp,in_port={},arp_tpa={},arp_op=1",
                    port, ext_ip
                ),
            ]);

            let _ = ovs_ofctl(&[
                "--strict",
                "del-flows",
                bridge,
                &format!("priority=235,in_port={},ip,nw_dst={}", port, ext_ip),
            ]);
        }
        Ok(())
    }

    /// Delete gateway and inter-subnet flows for one switch.
    fn delete_router_switch_flows(
        &self,
        switch: &crate::database::Switch,
        gateways: &std::collections::HashMap<String, (String, String)>,
    ) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let gateway_ip = compute_gateway_ip(&switch.ip)?;
        let (gateway_mac, _) = gateways.get(&switch.name).unwrap();

        for match_str in [
            format!("priority=280,arp,arp_tpa={},arp_op=1", gateway_ip),
            format!("priority=280,icmp,nw_dst={},icmp_type=8", gateway_ip),
        ] {
            let _ = ovs_ofctl(&["--strict", "del-flows", bridge, &match_str]);
        }

        for other_switch_name in gateways.keys() {
            if other_switch_name == &switch.name {
                continue;
            }

            let other_ports = self.db.get_switch_ports_for_switch(other_switch_name)?;
            for sp in other_ports {
                let match_str = format!("priority=265,ip,dl_dst={},nw_dst={}", gateway_mac, sp.ip);
                let _ = ovs_ofctl(&["--strict", "del-flows", bridge, &match_str]);
            }
        }

        Ok(())
    }

    /// Delete uplink flows for one switch subnet.
    fn delete_router_link_flows(
        &self,
        router: &crate::database::Router,
        switch: &crate::database::Switch,
        port: &str,
    ) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let cidr = format!("{}/{}", switch.ip, switch.mask);
        let switch_ports = self.db.get_switch_ports_for_switch(&switch.name)?;

        for match_str in [
            format!("priority=224,ip,nw_src={},ct_state=-trk", cidr),
            format!("priority=225,ip,nw_src={},ct_state=+trk+new", cidr),
            format!("priority=225,ip,nw_src={},ct_state=+trk+est", cidr),
            format!("priority=225,in_port={},arp,arp_tpa={}", port, cidr),
        ] {
            let _ = ovs_ofctl(&["--strict", "del-flows", bridge, &match_str]);
        }

        if let Some(ext_ip) = &router.link_ip {
            let _ = ovs_ofctl(&[
                "--strict",
                "del-flows",
                bridge,
                &format!("priority=235,in_port={},ip,nw_dst={}", port, ext_ip),
            ]);
        }

        for sp in switch_ports {
            let _ = ovs_ofctl(&[
                "--strict",
                "del-flows",
                bridge,
                &format!("priority=230,in_port={},ip,nw_dst={}", port, sp.ip),
            ]);
        }

        Ok(())
    }
}

/// Convert an IPv4 address string (e.g. "10.0.0.1") to a hex string
/// suitable for OVS `load` actions (e.g. "0a000001").
fn ip_to_hex(ip: &str) -> Result<String> {
    let addr: std::net::Ipv4Addr = ip
        .parse()
        .map_err(|e| anyhow!("invalid IP '{}': {}", ip, e))?;
    let octets = addr.octets();
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}",
        octets[0], octets[1], octets[2], octets[3]
    ))
}

/// Convert a MAC address string (e.g. "52:54:00:ab:cd:ef") to a hex string
/// suitable for OVS `load` actions (e.g. "525400abcdef").
fn mac_to_hex(mac: &str) -> Result<String> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return Err(anyhow!("invalid MAC '{}': expected 6 octets", mac));
    }
    Ok(parts.join(""))
}

/// Get the MAC address of the default gateway for a given interface.
fn get_gateway_mac(iface_name: &str) -> Result<String> {
    let gateway_ip = Interface::get_gateway_ip(iface_name)
        .or_else(|_| Interface::get_system_default_gateway_ip())?;

    for attempt in 0..2 {
        if let Ok(mac) = Interface::resolve_neighbor_mac(&gateway_ip) {
            return Ok(mac);
        }

        if attempt == 0 {
            let _ = std::process::Command::new("ping")
                .args(["-c", "1", "-W", "1", &gateway_ip])
                .output();
        }
    }

    Err(anyhow!(
        "could not resolve MAC address for gateway '{}' after attempt.",
        gateway_ip
    ))
}

/// Send a Gratuitous ARP from an OVS port.
fn send_garp(bridge: &str, port: &str, ofport: &str, ip: &str) -> Result<()> {
    let mac = Interface::get_mac(port)?;
    let mac_hex = mac_to_hex(&mac)?;
    let ip_hex = ip_to_hex(ip)?;

    let packet = format!(
        "ffffffffffff{}08060001080006040001{}{}ffffffffffff{}",
        mac_hex, mac_hex, ip_hex, ip_hex
    );

    ovs_ofctl(&[
        "packet-out",
        bridge,
        "none",
        &format!("output:{}", ofport),
        &packet,
    ])?;

    Ok(())
}

/// Get the OpenFlow port number (ofport) for a port on a given bridge.
fn get_ofport(bridge: &str, port_name: &str) -> Result<String> {
    let _ = bridge;
    use crate::switches::ovs_vsctl;
    let ofport = ovs_vsctl(&["get", "Interface", port_name, "ofport"])?;
    let ofport = ofport.trim().to_string();
    if ofport == "-1" || ofport.is_empty() {
        return Err(anyhow!(
            "port '{}' is not ready (ofport={})",
            port_name,
            ofport
        ));
    }
    Ok(ofport)
}

/// Compute the gateway IP for a switch subnet (always the .1 address).
/// E.g. switch IP "10.0.0.0" → gateway "10.0.0.1"
fn compute_gateway_ip(switch_ip: &str) -> Result<String> {
    let addr: std::net::Ipv4Addr = switch_ip
        .parse()
        .map_err(|e| anyhow!("invalid switch IP '{}': {}", switch_ip, e))?;
    let mut octets = addr.octets();
    octets[3] = 1;
    Ok(format!(
        "{}.{}.{}.{}",
        octets[0], octets[1], octets[2], octets[3]
    ))
}
