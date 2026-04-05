use crate::config::Config;
use crate::database::{Database, Router};
use crate::interfaces::Interface;
use crate::switches::{ovs_ofctl, ovs_vsctl};
use crate::utils::generate_deterministic_mac;
use anyhow::{Result, anyhow};
use ipnetwork::IpNetwork;
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

    /// Configure or clear a router link.
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
                "Router '{}' does not have a link configured",
                router_name
            ));
        }

        let router = self.db.get_router(router_name)?;

        let _ = ovs_vsctl(&[
            "--if-exists",
            "del-port",
            &self.config.bridge_name,
            &router.link_name.as_ref().unwrap(),
        ]);

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

    pub fn add_route(
        &self,
        router_name: &str,
        source: &str,
        destination: &str,
        next_hop: Option<&str>,
        metric: u32,
    ) -> Result<()> {
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", router_name));
        }

        let router = self.db.get_router(router_name)?;
        self.db
            .create_route(&router.uuid, source, destination, next_hop, metric)?;

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.delete_router_flows(&router.name)?;
        self.apply_router_flows(&router.name)?;

        Ok(())
    }

    pub fn rm_route(&self, router_name: &str, source: &str, destination: &str) -> Result<()> {
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", router_name));
        }

        let router = self.db.get_router(router_name)?;

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.db.remove_route(&router.uuid, source, destination)?;
        self.delete_router_flows(&router.name)?;
        self.apply_router_flows(&router.name)?;

        Ok(())
    }

    pub fn list_routes(&self, router_name: &str) -> Result<Vec<crate::database::RouterRoute>> {
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{}' does not exist", router_name));
        }

        let router = self.db.get_router(router_name)?;

        self.db.list_routes_for_router(&router.uuid)
    }

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

            self.apply_router_gateway_flows(&switch, &gateways)?;
        }

        self.apply_router_routes_flows(&router)?;

        if let (Some(link_name), Some(link_mac), Some(link_ip)) = (
            router.link_name.as_ref(),
            router.link_mac.as_ref(),
            router.link_ip.as_ref(),
        ) {
            if let Ok(port) = get_ofport(bridge, link_name) {
                self.apply_link_arp_flows(link_mac, link_ip, &port)?;
                self.apply_link_nat_flows(&router, &port)?;

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

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=280,arp,arp_tpa={},arp_op=1,actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:{},load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],load:0x{}->NXM_NX_ARP_SHA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x{}->NXM_OF_ARP_SPA[],IN_PORT",
                &gateway.1, &gateway.0, gateway_mac_hex, gateway_ip_hex,
            ),
        ])?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=280,icmp,nw_dst={},icmp_type=8,actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:{},move:NXM_OF_IP_SRC[]->NXM_OF_IP_DST[],mod_nw_src:{},load:0->NXM_OF_ICMP_TYPE[],IN_PORT",
                &gateway.1, &gateway.0, &gateway.1
            ),
        ])?;

        Ok(())
    }

    fn apply_router_gateway_flows(
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
                        "priority=265,ip,dl_dst={},nw_dst={},actions=mod_dl_src:{},mod_dl_dst:{},dec_ttl,output:{}",
                        to_gateway_mac, sp.ip, from_gateway_mac, sp.mac, sp.interface_name,
                    ),
                ])?;
            }
        }

        Ok(())
    }

    /// Apply ARP responder flow for the link's external IP.
    fn apply_link_arp_flows(&self, mac: &str, ip: &str, port: &str) -> Result<()> {
        let bridge = &self.config.bridge_name;

        let mac_hex = mac_to_hex(mac)?;
        let ip_hex = ip_to_hex(ip)?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=231,arp,in_port={},arp_tpa={},arp_op=1,actions=move:NXM_OF_ETH_SRC[]->NXM_OF_ETH_DST[],mod_dl_src:{},load:0x2->NXM_OF_ARP_OP[],move:NXM_NX_ARP_SHA[]->NXM_NX_ARP_THA[],load:0x{}->NXM_NX_ARP_SHA[],move:NXM_OF_ARP_SPA[]->NXM_OF_ARP_TPA[],load:0x{}->NXM_OF_ARP_SPA[],IN_PORT",
                port, ip, mac, mac_hex, ip_hex,
            ),
        ])?;

        Ok(())
    }

    /// Apply NAT flows for the link: return flow + per-route NAT flows.
    fn apply_link_nat_flows(&self, router: &Router, port: &str) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let link_ip = router.link_ip.as_ref().unwrap();
        let link_mac = router.link_mac.as_ref().unwrap();

        // Return flow: un-NAT packets returning from the external network and resubmit them.
        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=235,in_port={},ip,nw_dst={},actions=ct(zone=1,nat,table=0)",
                port, link_ip
            ),
        ])?;

        // Per-route NAT flows for non-inter-subnet destinations.
        // route.source acts as a policy filter: only matching subnets get NATed.
        let router_ports = self.db.list_router_ports_for_router(&router.name)?;
        let routes = self.db.list_routes_for_router(&router.uuid)?;
        let attached_cidrs: Vec<String> = router_ports
            .iter()
            .filter_map(|rp| self.db.get_switch(&rp.switch_name).ok())
            .map(|s| format!("{}/{}", s.ip, s.mask))
            .collect();

        // Resolve all next_hop MACs upfront to avoid partial flow state on failure.
        let next_hop_macs: std::collections::HashMap<String, String> = routes
            .iter()
            .filter_map(|r| r.next_hop.as_deref())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .map(|ip| resolve_neighbor_mac(ip).map(|mac| (ip.to_string(), mac)))
            .collect::<Result<_>>()?;

        for route in &routes {
            let is_inter_subnet = attached_cidrs.iter().any(|c| c == &route.destination);
            if is_inter_subnet {
                continue;
            }

            let dst_prefix_len = route
                .destination
                .split('/')
                .nth(1)
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);

            let priority = (225 + dst_prefix_len as u32).saturating_sub(route.metric);

            let mut new_actions = Vec::new();
            new_actions.push(format!("ct(commit,zone=1,nat(src={}))", link_ip));
            new_actions.push(format!("mod_dl_src:{}", link_mac));
            if let Some(next_hop) = &route.next_hop {
                new_actions.push(format!(
                    "mod_dl_dst:{}",
                    next_hop_macs.get(next_hop).unwrap()
                ));
            }
            new_actions.push("dec_ttl".to_string());
            new_actions.push(format!("output:{}", port));

            ovs_ofctl(&[
                "add-flow",
                bridge,
                &format!(
                    "priority={},ct_state=+new+trk,ip,nw_src={},nw_dst={},actions={}",
                    priority,
                    route.source,
                    route.destination,
                    new_actions.join(",")
                ),
            ])?;

            let mut est_actions = Vec::new();
            est_actions.push("ct(zone=1,nat)".to_string());
            est_actions.push(format!("mod_dl_src:{}", link_mac));
            if let Some(next_hop) = &route.next_hop {
                est_actions.push(format!(
                    "mod_dl_dst:{}",
                    next_hop_macs.get(next_hop).unwrap()
                ));
            }
            est_actions.push("dec_ttl".to_string());
            est_actions.push(format!("output:{}", port));

            ovs_ofctl(&[
                "add-flow",
                bridge,
                &format!(
                    "priority={},ct_state=+est+trk,ip,nw_src={},nw_dst={},actions={}",
                    priority,
                    route.source,
                    route.destination,
                    est_actions.join(",")
                ),
            ])?;
        }

        // Drop traffic from attached subnets not covered by any route's source.
        let covered_cidrs: Vec<&str> = routes.iter().map(|r| r.source.as_str()).collect();
        for cidr in &attached_cidrs {
            if !cidr_is_covered(cidr, &covered_cidrs) {
                ovs_ofctl(&[
                    "add-flow",
                    bridge,
                    &format!(
                        "priority=220,ct_state=+new+trk,ip,nw_src={},actions=drop",
                        cidr
                    ),
                ])?;
            }
        }

        Ok(())
    }

    fn apply_router_routes_flows(&self, router: &Router) -> Result<()> {
        let bridge = &self.config.bridge_name;

        let router_ports = self.db.list_router_ports_for_router(&router.name)?;
        let routes = self.db.list_routes_for_router(&router.uuid)?;

        let cidr_to_switch: std::collections::HashMap<String, &str> = router_ports
            .iter()
            .filter_map(|rp| {
                self.db
                    .get_switch(&rp.switch_name)
                    .ok()
                    .map(|s| (format!("{}/{}", s.ip, s.mask), rp.switch_name.as_str()))
            })
            .collect();

        // Resolve all next_hop MACs upfront to avoid partial flow state on failure.
        let next_hop_macs: std::collections::HashMap<String, String> = routes
            .iter()
            .filter_map(|r| r.next_hop.as_deref())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .map(|ip| resolve_neighbor_mac(ip).map(|mac| (ip.to_string(), mac)))
            .collect::<Result<_>>()?;

        for route in &routes {
            let Some(switch_name) = cidr_to_switch.get(&route.destination) else {
                continue;
            };

            let dest_ports = self.db.get_switch_ports_for_switch(switch_name)?;
            let dest_port_outputs: Vec<String> = dest_ports
                .iter()
                .map(|sp| format!("output:{}", sp.interface_name))
                .collect();

            let dst_prefix_len = route
                .destination
                .split('/')
                .nth(1)
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);

            let priority = (225 + dst_prefix_len as u32).saturating_sub(route.metric);

            let actions = if let Some(next_hop) = &route.next_hop {
                let next_hop_mac = next_hop_macs.get(next_hop).unwrap();
                format!(
                    "mod_dl_dst:{},{}",
                    next_hop_mac,
                    dest_port_outputs.join(",")
                )
            } else {
                dest_port_outputs.join(",")
            };

            ovs_ofctl(&[
                "add-flow",
                bridge,
                &format!(
                    "priority={},ip,nw_src={},nw_dst={},actions={}",
                    priority, route.source, route.destination, actions
                ),
            ])?;
        }

        Ok(())
    }

    fn delete_routes_flows(&self, router: &Router) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let routes = self.db.list_routes_for_router(&router.uuid)?;

        for route in &routes {
            let _ = ovs_ofctl(&[
                "del-flows",
                bridge,
                &format!("ip,nw_src={},nw_dst={}", route.source, route.destination),
            ]);
        }

        Ok(())
    }

    fn apply_router_link_flows(
        &self,
        router: &crate::database::Router,
        switch: &crate::database::Switch,
        cidr: &str,
        port: &str,
    ) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let switch_ports = self.db.get_switch_ports_for_switch(&switch.name)?;

        ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!(
                "priority=224,ip,nw_src={},ct_state=-trk,actions=ct(table=0,zone=1)",
                cidr
            ),
        ])?;

        for sp in switch_ports {
            let mut ingress_actions = Vec::new();
            if let Some(src_mac) = &router.link_mac {
                ingress_actions.push(format!("mod_dl_src:{}", src_mac));
            }

            ingress_actions.push(format!("mod_dl_dst:{}", sp.mac));
            ingress_actions.push("dec_ttl".to_string());
            ingress_actions.push(format!("output:{}", sp.interface_name));

            let mut direct_in_actions = vec!["ct(commit,zone=1)".to_string()];
            direct_in_actions.extend(ingress_actions);

            ovs_ofctl(&[
                "add-flow",
                bridge,
                &format!(
                    "priority=230,in_port={},ip,nw_dst={},actions={}",
                    port,
                    sp.ip,
                    direct_in_actions.join(",")
                ),
            ])?;
        }

        Ok(())
    }

    /// Delete all router flows for attached switches and optional link.
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

        if let Some(port) = &link_port {
            if let Some(link_ip) = &router.link_ip {
                self.delete_link_arp_flows(link_ip, port)?;
            }
            self.delete_link_nat_flows(&router, port)?;
        }

        self.delete_routes_flows(&router)?;

        let mut seen_switches = std::collections::HashSet::new();
        for rp in &router_ports {
            if !seen_switches.insert(rp.switch_name.clone()) {
                continue;
            }

            let switch = self.db.get_switch(&rp.switch_name)?;
            self.delete_router_switch_flows(&switch, &router_gateways)?;
            if let Some(ofport) = &link_port {
                self.delete_router_link_flows(&switch, ofport)?;
            }
        }

        Ok(())
    }

    /// Delete ARP responder flow for the link's external IP.
    fn delete_link_arp_flows(&self, ext_ip: &str, port: &str) -> Result<()> {
        let bridge = &self.config.bridge_name;

        let _ = ovs_ofctl(&[
            "--strict",
            "del-flows",
            bridge,
            &format!(
                "priority=231,arp,in_port={},arp_tpa={},arp_op=1",
                port, ext_ip
            ),
        ]);

        Ok(())
    }

    /// Delete NAT flows for the link: return flow + per-route NAT + ct_state flows.
    fn delete_link_nat_flows(&self, router: &Router, port: &str) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let router_ports = self.db.list_router_ports_for_router(&router.name)?;
        let routes = self.db.list_routes_for_router(&router.uuid)?;

        // Delete NAT return flow
        if let Some(link_ip) = &router.link_ip {
            let _ = ovs_ofctl(&[
                "--strict",
                "del-flows",
                bridge,
                &format!("priority=235,in_port={},ip,nw_dst={}", port, link_ip),
            ]);
        }

        // Delete per-route NAT flows for external routes
        let attached_cidrs: Vec<String> = router_ports
            .iter()
            .filter_map(|rp| self.db.get_switch(&rp.switch_name).ok())
            .map(|s| format!("{}/{}", s.ip, s.mask))
            .collect();

        for route in &routes {
            let is_inter_subnet = attached_cidrs.iter().any(|c| c == &route.destination);
            if is_inter_subnet {
                continue;
            }

            let dst_prefix_len = route
                .destination
                .split('/')
                .nth(1)
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);

            let priority = (225 + dst_prefix_len as u32).saturating_sub(route.metric);

            for match_str in [
                format!(
                    "priority={},ct_state=+new+trk,ip,nw_src={},nw_dst={}",
                    priority, route.source, route.destination
                ),
                format!(
                    "priority={},ct_state=+est+trk,ip,nw_src={},nw_dst={}",
                    priority, route.source, route.destination
                ),
            ] {
                let _ = ovs_ofctl(&["--strict", "del-flows", bridge, &match_str]);
            }
        }

        // Delete drop rules for uncovered subnets
        let covered_cidrs: Vec<&str> = routes.iter().map(|r| r.source.as_str()).collect();
        for cidr in &attached_cidrs {
            if !cidr_is_covered(cidr, &covered_cidrs) {
                let _ = ovs_ofctl(&[
                    "--strict",
                    "del-flows",
                    bridge,
                    &format!(
                        "priority=220,ct_state=+new+trk,ip,nw_src={},actions=drop",
                        cidr
                    ),
                ]);
            }
        }

        Ok(())
    }

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

    fn delete_router_link_flows(&self, switch: &crate::database::Switch, port: &str) -> Result<()> {
        let bridge = &self.config.bridge_name;
        let cidr = format!("{}/{}", switch.ip, switch.mask);
        let switch_ports = self.db.get_switch_ports_for_switch(&switch.name)?;

        let match_str = format!("priority=224,ip,nw_src={},ct_state=-trk", cidr);
        let _ = ovs_ofctl(&["--strict", "del-flows", bridge, &match_str]);

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

fn mac_to_hex(mac: &str) -> Result<String> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return Err(anyhow!("invalid MAC '{}': expected 6 octets", mac));
    }
    Ok(parts.join(""))
}

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

fn resolve_neighbor_mac(ip: &str) -> Result<String> {
    for _attempt in 0..2 {
        let _ = std::process::Command::new("ping")
            .args(["-c", "1", "-W", "1", ip])
            .output();

        std::thread::sleep(std::time::Duration::from_millis(100));

        let output = std::process::Command::new("ip")
            .args(["-j", "neigh", "show", ip])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            if let Some(mac) = json
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|entry| entry["lladdr"].as_str())
            {
                return Ok(mac.to_string());
            }
        }
    }

    Err(anyhow!("could not resolve MAC for neighbor '{}'", ip))
}

fn cidr_is_covered(cidr: &str, sources: &[&str]) -> bool {
    let Ok(cidr_net) = cidr.parse::<IpNetwork>() else {
        return false;
    };
    sources.iter().any(|s| {
        let Ok(source_net) = s.parse::<IpNetwork>() else {
            return false;
        };
        source_net.contains(cidr_net.network()) || cidr_net.contains(source_net.network())
    })
}
