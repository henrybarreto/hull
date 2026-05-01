use crate::config::Config;
use crate::database::{Database, Router};
use crate::interfaces::Interface;
use crate::of;
use crate::utils::{FlowCookieKind, flow_cookie, generate_deterministic_mac};
use anyhow::{Result, anyhow};
use ipnetwork::IpNetwork;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, trace};

use crate::openflow::protocol::action::Action;
use crate::openflow::protocol::constants::{
    ETH_TYPE_ARP, ETH_TYPE_IPV4, OFPP_IN_PORT, OFPXMT_OFB_ARP_SHA, OFPXMT_OFB_ARP_SPA,
    OFPXMT_OFB_ARP_THA, OFPXMT_OFB_ARP_TPA, OFPXMT_OFB_ETH_DST, OFPXMT_OFB_ETH_SRC,
    OFPXMT_OFB_IPV4_DST, OFPXMT_OFB_IPV4_SRC,
};
use crate::openflow::protocol::instruction::Instruction;
use crate::openflow::protocol::ofmatch::Match;
use crate::openflow::protocol::oxm;
use crate::openflow::protocol::rule::Rule;

/// Router CRUD and flow programming operations.
pub struct RouterOps {
    db: Arc<Database>,
    config: Arc<Config>,
    ovs: Arc<crate::ovs::BridgeClient>,
}

impl RouterOps {
    /// Create a new router operations instance.
    pub const fn new(
        db: Arc<Database>,
        config: Arc<Config>,
        ovs: Arc<crate::ovs::BridgeClient>,
    ) -> Self {
        Self { db, config, ovs }
    }

    fn router_cookie(router: &Router) -> Result<u64> {
        flow_cookie(FlowCookieKind::Router, &router.uuid)
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

    async fn add_packet_flow(&self, of: &mut of::OF, flow: Rule) -> Result<()> {
        of.insert(flow).await
    }

    async fn insert_flow(&self, bridge: &str, cookie: u64, flow: impl AsRef<str>) -> Result<()> {
        // NOTE: The remaining shell-backed path only covers NXAST_CT/NXAST_NAT.
        // The local OpenFlow text/spec sources and installed headers do not
        // expose those wire structs yet.
        let flow = flow.as_ref();
        let _ = ovs_ofctl(&[
            "add-flow",
            bridge,
            &format!("cookie=0x{cookie:016x},{flow}"),
        ])
        .await?;
        Ok(())
    }

    async fn remove_flows(&self, of: &mut of::OF, cookie: u64) -> Result<()> {
        of.remove(Some(cookie)).await
    }

    async fn wait_for_ofport(&self, interface_name: &str) -> Result<u32> {
        for _ in 0..50 {
            if let Some(ofport) = self.ovs.interface_ofport(interface_name).await? {
                return Ok(ofport);
            }
            sleep(Duration::from_millis(50)).await;
        }

        Err(anyhow!(
            "interface '{interface_name}' did not receive an OVS ofport"
        ))
    }

    fn is_bridge_local_port(&self, port_name: &str) -> bool {
        port_name == self.config.bridge_name
    }

    /// Create a router and apply its flows.
    ///
    /// # Errors
    /// Returns an error if the router already exists, database updates fail, or flow programming fails.
    pub async fn create(&self, name: &str) -> Result<()> {
        debug!(router = %name, "creating router");
        if self.db.get_router(name).is_ok() {
            return Err(anyhow!("Router '{name}' already exists"));
        }

        let router = self.db.create_router(name)?;
        self.apply_router_flows(&router.name).await?;

        Ok(())
    }

    /// Remove a router and delete its flows.
    ///
    /// # Errors
    /// Returns an error if the router does not exist, database updates fail, or flow deletion fails.
    pub async fn remove(&self, name: &str) -> Result<()> {
        debug!(router = %name, "removing router");
        if self.db.get_router(name).is_err() {
            return Err(anyhow!("Router '{name}' does not exist"));
        }

        let router = self.db.get_router(name)?;

        self.delete_router_flows(&router.name).await?;
        self.db.remove_router(&router.name)?;

        Ok(())
    }

    /// List all routers.
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub fn list(&self) -> Result<Vec<crate::database::Router>> {
        trace!("listing routers");
        self.db.list_routers()
    }

    /// List switches attached to a router.
    ///
    /// # Errors
    /// Returns an error if the router does not exist or the database query fails.
    pub fn list_attached_switches(&self, name: &str) -> Result<Vec<String>> {
        trace!(router = %name, "listing attached switches");
        if self.db.get_router(name).is_err() {
            return Err(anyhow!("Router '{name}' does not exist"));
        }

        let router_ports = self.db.list_router_ports_for_router(name)?;

        let mut switch_names = Vec::new();
        for router_port in router_ports {
            switch_names.push(router_port.switch_name);
        }

        Ok(switch_names)
    }

    /// Attach a switch to a router and apply updated flows.
    ///
    /// # Errors
    /// Returns an error if either object does not exist, the pair is already attached,
    /// or database/flow programming fails.
    pub async fn attach(&self, router_name: &str, switch_name: &str) -> Result<()> {
        debug!(router = %router_name, switch = %switch_name, "attaching switch to router");
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{router_name}' does not exist"));
        }

        if self.db.get_switch(switch_name).is_err() {
            return Err(anyhow!("Switch '{switch_name}' does not exist"));
        }

        if let Ok(switches) = self.list_attached_switches(router_name)
            && switches.contains(&switch_name.to_string())
        {
            return Err(anyhow!(
                "Router '{router_name}' is already attached to switch '{switch_name}'"
            ));
        }

        let router = self.db.get_router(router_name)?;
        let switch = self.db.get_switch(switch_name)?;

        let router_ports = self.db.list_router_ports_for_router(&router.name)?;
        let router_port = router_ports.iter().find(|p| p.switch_name == switch.name);
        if router_port.is_some() {
            return Err(anyhow!(
                "Router '{router_name}' is already attached to switch '{switch_name}'"
            ));
        }

        self.delete_router_flows(&router.name).await?;

        // TODO: This is inefficient, we delete and re-apply all flows for the router on every
        // attach/detach.
        self.db
            .create_router_port(&router.name, &switch.name, None, None)?;
        self.apply_router_flows(&router.name).await?;

        Ok(())
    }

    /// Detach a switch from a router and apply updated flows.
    ///
    /// # Errors
    /// Returns an error if either object does not exist, the pair is not attached,
    /// or database/flow programming fails.
    pub async fn detach(&self, router_name: &str, switch_name: &str) -> Result<()> {
        debug!(router = %router_name, switch = %switch_name, "detaching switch from router");
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{router_name}' does not exist"));
        }

        if self.db.get_switch(switch_name).is_err() {
            return Err(anyhow!("Switch '{switch_name}' does not exist"));
        }

        if let Ok(switches) = self.list_attached_switches(router_name)
            && !switches.contains(&switch_name.to_string())
        {
            return Err(anyhow!(
                "Router '{router_name}' is not attached to switch '{switch_name}'"
            ));
        }

        let router = self.db.get_router(router_name)?;
        let switch = self.db.get_switch(switch_name)?;

        self.delete_router_flows(&router.name).await?;

        // TODO: This is inefficient, we delete and re-apply all flows for the router on every
        // attach/detach.
        self.db.remove_router_port(&router.name, &switch.name)?;
        self.apply_router_flows(&router.name).await?;

        Ok(())
    }

    /// Configure or clear a router link.
    ///
    /// # Errors
    /// Returns an error if the router or bridge port is missing, the port is already in use,
    /// or database/flow programming fails.
    pub async fn set_link(
        &self,
        router_name: &str,
        port_name: &str,
        ip: &str,
        mac: &str,
    ) -> Result<()> {
        debug!(router = %router_name, port = %port_name, ip = %ip, mac = %mac, "setting router link");
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{router_name}' does not exist"));
        }

        if !Interface::exists(port_name).await {
            return Err(anyhow!(
                "Bridge port '{port_name}' does not exist on system"
            ));
        }

        if self.db.get_router_link(port_name).is_ok() {
            return Err(anyhow!(
                "Bridge port '{port_name}' is already in use by another router"
            ));
        }

        let router = self.db.get_router(router_name)?;

        self.db
            .update_router_link(&router.name, Some(port_name), Some(ip), Some(mac))?;

        if !self.is_bridge_local_port(port_name) {
            let _ = self
                .ovs
                .add_port(&self.config.bridge_name, port_name, serde_json::json!({}))
                .await;
        }

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.delete_router_flows(&router.name).await?;
        self.apply_router_flows(&router.name).await?;

        Ok(())
    }

    /// Unset a router link.
    ///
    /// # Errors
    /// Returns an error if the router does not exist, does not have a link, or flow/database operations fail.
    pub async fn unset_link(&self, router_name: &str) -> Result<()> {
        debug!(router = %router_name, "unsetting router link");
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{router_name}' does not exist"));
        }

        if self.db.get_router_link(router_name).is_err() {
            return Err(anyhow!(
                "Router '{router_name}' does not have a link configured"
            ));
        }

        let router = self.db.get_router(router_name)?;

        if let Some(port_name) = router.link_name.as_ref()
            && !self.is_bridge_local_port(port_name)
        {
            let _ = self.ovs.del_port(&self.config.bridge_name, port_name).await;
        }

        self.db.update_router_link(&router.name, None, None, None)?;

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.delete_router_flows(&router.name).await?;
        self.apply_router_flows(&router.name).await?;

        Ok(())
    }

    /// Re-apply all router flows from database state.
    ///
    /// # Errors
    /// Returns an error if database access or flow programming fails.
    pub async fn sync(&self) -> Result<()> {
        debug!("syncing routers from database");
        let routers = self.db.list_routers()?;
        for router in &routers {
            trace!(router = %router.name, "syncing router");
            if let Some(port_name) = &router.link_name
                && !self.is_bridge_local_port(port_name)
            {
                trace!(router = %router.name, port = %port_name, "ensuring router link port exists");
                self.ovs
                    .add_port(&self.config.bridge_name, port_name, serde_json::json!({}))
                    .await?;
            } else if let Some(port_name) = &router.link_name {
                trace!(router = %router.name, port = %port_name, "router link port already local");
            }

            self.delete_router_flows(&router.name).await?;
            self.apply_router_flows(&router.name).await?;
        }

        Ok(())
    }

    /// Add a router route and re-apply flows.
    ///
    /// # Errors
    /// Returns an error if the router does not exist, database updates fail, or flow programming fails.
    pub async fn add_route(
        &self,
        router_name: &str,
        source: &str,
        destination: &str,
        next_hop: Option<&str>,
        next_hop_mac: Option<&str>,
        metric: u32,
    ) -> Result<()> {
        debug!(
            router = %router_name,
            source = %source,
            destination = %destination,
            next_hop = ?next_hop,
            next_hop_mac = ?next_hop_mac,
            metric,
            "adding route"
        );
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{router_name}' does not exist"));
        }

        let router = self.db.get_router(router_name)?;
        self.db.create_route(
            &router.uuid,
            source,
            destination,
            next_hop,
            next_hop_mac,
            metric,
        )?;

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.delete_router_flows(&router.name).await?;
        self.apply_router_flows(&router.name).await?;

        Ok(())
    }

    /// Remove a router route and re-apply flows.
    ///
    /// # Errors
    /// Returns an error if the router does not exist, database updates fail, or flow programming fails.
    pub async fn rm_route(&self, router_name: &str, source: &str, destination: &str) -> Result<()> {
        debug!(router = %router_name, source = %source, destination = %destination, "removing route");
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{router_name}' does not exist"));
        }

        let router = self.db.get_router(router_name)?;

        // TODO: We should avoid delete and replay the whole router flows here, but it's simpler
        // for now.
        self.db.remove_route(&router.uuid, source, destination)?;
        self.delete_router_flows(&router.name).await?;
        self.apply_router_flows(&router.name).await?;

        Ok(())
    }

    /// List routes attached to a router.
    ///
    /// # Errors
    /// Returns an error if the router does not exist or the database query fails.
    pub fn list_routes(&self, router_name: &str) -> Result<Vec<crate::database::RouterRoute>> {
        trace!(router = %router_name, "listing routes");
        if self.db.get_router(router_name).is_err() {
            return Err(anyhow!("Router '{router_name}' does not exist"));
        }

        let router = self.db.get_router(router_name)?;

        self.db.list_routes_for_router(&router.uuid)
    }

    ///
    /// # Errors
    /// Returns an error if the router does not exist or any flow/database operation fails.
    pub async fn apply_router_flows(&self, name: &str) -> Result<()> {
        trace!(router = %name, "applying router flows");
        let router = self.db.get_router(name)?;
        let cookie = Self::router_cookie(&router)?;
        let router_ports = self.db.list_router_ports_for_router(name)?;
        let mut of = of::OF::connect(&self.config.bridge_name).await?;

        let mut gateways = std::collections::HashMap::new();
        for router_port in &router_ports {
            let switch = self.db.get_switch(&router_port.switch_name)?;

            let gateway_mac = generate_deterministic_mac(name, &router_port.switch_name);
            let gateway_ip = compute_gateway_ip(&switch.ip)?;

            self.apply_router_gateway_arp_flows(
                &mut of,
                cookie,
                (gateway_mac.clone(), gateway_ip.clone()),
            )
            .await?;

            gateways.insert(router_port.switch_name.clone(), (gateway_mac, gateway_ip));
        }

        for rp in &router_ports {
            let switch = self.db.get_switch(&rp.switch_name)?;

            self.apply_router_gateway_flows(&mut of, cookie, &switch, &gateways)
                .await?;
        }

        self.apply_router_routes_flows(&mut of, cookie, &router)
            .await?;

        if let (Some(link_name), Some(link_mac), Some(link_ip)) = (
            router.link_name.as_ref(),
            router.link_mac.as_ref(),
            router.link_ip.as_ref(),
        ) {
            self.apply_link_arp_flows(&mut of, cookie, link_mac, link_ip, link_name)
                .await?;

            self.apply_link_nat_flows(cookie, &router, link_name)
                .await?;

            let mut seen_switches = std::collections::HashSet::new();
            for rp in &router_ports {
                if !seen_switches.insert(rp.switch_name.clone()) {
                    continue;
                }

                let switch = self.db.get_switch(&rp.switch_name)?;
                let cidr = format!("{}/{}", switch.ip, switch.mask);

                self.apply_router_link_flows(cookie, &router, &switch, &cidr, link_name)
                    .await?;
            }
        }

        Ok(())
    }

    /// Apply gateway flows and inter-subnet routing for one switch.
    async fn apply_router_gateway_arp_flows(
        &self,
        of: &mut of::OF,
        cookie: u64,
        gateway: (String, String),
    ) -> Result<()> {
        trace!("applying router gateway arp flows");
        let gateway_mac = parse_mac(&gateway.0)?;
        let gateway_ip = parse_ipv4(&gateway.1)?;

        self.add_packet_flow(
            of,
            Self::add_flow(
                cookie,
                280,
                Match::new(vec![
                    oxm::eth_type(ETH_TYPE_ARP),
                    oxm::arp_tpa(gateway_ip),
                    oxm::arp_op(1),
                ]),
                vec![
                    Action::CopyField {
                        n_bits: 48,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(OFPXMT_OFB_ETH_SRC, 6, OFPXMT_OFB_ETH_DST, 6),
                    },
                    Action::SetField(oxm::eth_src(gateway_mac)),
                    Action::SetField(oxm::arp_op(2)),
                    Action::CopyField {
                        n_bits: 48,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(OFPXMT_OFB_ARP_SHA, 6, OFPXMT_OFB_ARP_THA, 6),
                    },
                    Action::SetField(oxm::arp_sha(gateway_mac)),
                    Action::CopyField {
                        n_bits: 32,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(OFPXMT_OFB_ARP_SPA, 4, OFPXMT_OFB_ARP_TPA, 4),
                    },
                    Action::SetField(oxm::arp_spa(gateway_ip)),
                    Action::output(OFPP_IN_PORT),
                ],
            ),
        )
        .await?;

        self.add_packet_flow(
            of,
            Self::add_flow(
                cookie,
                280,
                Match::new(vec![
                    oxm::eth_type(ETH_TYPE_IPV4),
                    oxm::ipv4_dst(gateway_ip),
                    oxm::ip_proto(1),
                    oxm::icmpv4_type(8),
                ]),
                vec![
                    Action::CopyField {
                        n_bits: 48,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(OFPXMT_OFB_ETH_SRC, 6, OFPXMT_OFB_ETH_DST, 6),
                    },
                    Action::SetField(oxm::eth_src(gateway_mac)),
                    Action::CopyField {
                        n_bits: 32,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(
                            OFPXMT_OFB_IPV4_SRC,
                            4,
                            OFPXMT_OFB_IPV4_DST,
                            4,
                        ),
                    },
                    Action::SetField(oxm::ipv4_src(gateway_ip)),
                    Action::SetField(oxm::icmpv4_type(0)),
                    Action::output(OFPP_IN_PORT),
                ],
            ),
        )
        .await?;

        Ok(())
    }

    async fn apply_router_gateway_flows(
        &self,
        of: &mut of::OF,
        cookie: u64,
        switch: &crate::database::Switch,
        gateways: &std::collections::HashMap<String, (String, String)>,
    ) -> Result<()> {
        trace!(switch = %switch.name, "applying router gateway flows");
        let Some((to_gateway_mac, _)) = gateways.get(&switch.name) else {
            return Ok(());
        };
        let to_gateway_mac = parse_mac(to_gateway_mac)?;

        for other_switch_name in gateways.keys() {
            if other_switch_name == &switch.name {
                continue;
            }

            let Some((from_gateway_mac, _)) = gateways.get(other_switch_name) else {
                continue;
            };
            let from_gateway_mac = parse_mac(from_gateway_mac)?;

            let other_ports = self.db.get_switch_ports_for_switch(other_switch_name)?;
            for sp in other_ports {
                let sp_ofport = self.wait_for_ofport(&sp.interface_name).await?;
                let sp_mac = parse_mac(&sp.mac)?;
                let sp_ip = parse_ipv4(&sp.ip)?;

                self.add_packet_flow(
                    of,
                    Self::add_flow(
                        cookie,
                        265,
                        Match::new(vec![
                            oxm::eth_type(ETH_TYPE_IPV4),
                            oxm::eth_dst(to_gateway_mac),
                            oxm::ipv4_dst(sp_ip),
                        ]),
                        vec![
                            Action::SetField(oxm::eth_src(from_gateway_mac)),
                            Action::SetField(oxm::eth_dst(sp_mac)),
                            Action::DecNwTtl,
                            Action::output(sp_ofport),
                        ],
                    ),
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Apply ARP responder flow for the link's external IP.
    async fn apply_link_arp_flows(
        &self,
        of: &mut of::OF,
        cookie: u64,
        mac: &str,
        ip: &str,
        port: &str,
    ) -> Result<()> {
        trace!(port = %port, "applying router link arp flows");
        let port_ofport = self.wait_for_ofport(port).await?;
        let mac = parse_mac(mac)?;
        let ip = parse_ipv4(ip)?;

        self.add_packet_flow(
            of,
            Self::add_flow(
                cookie,
                231,
                Match::new(vec![
                    oxm::eth_type(ETH_TYPE_ARP),
                    oxm::in_port(port_ofport),
                    oxm::arp_tpa(ip),
                    oxm::arp_op(1),
                ]),
                vec![
                    Action::CopyField {
                        n_bits: 48,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(OFPXMT_OFB_ETH_SRC, 6, OFPXMT_OFB_ETH_DST, 6),
                    },
                    Action::SetField(oxm::eth_src(mac)),
                    Action::SetField(oxm::arp_op(2)),
                    Action::CopyField {
                        n_bits: 48,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(OFPXMT_OFB_ARP_SHA, 6, OFPXMT_OFB_ARP_THA, 6),
                    },
                    Action::SetField(oxm::arp_sha(mac)),
                    Action::CopyField {
                        n_bits: 32,
                        src_offset: 0,
                        dst_offset: 0,
                        oxm_ids: oxm::copy_field_ids(OFPXMT_OFB_ARP_SPA, 4, OFPXMT_OFB_ARP_TPA, 4),
                    },
                    Action::SetField(oxm::arp_spa(ip)),
                    Action::output(OFPP_IN_PORT),
                ],
            ),
        )
        .await?;

        Ok(())
    }

    /// Apply NAT flows for the link: return flow + per-route NAT flows.
    async fn apply_link_nat_flows(&self, cookie: u64, router: &Router, port: &str) -> Result<()> {
        trace!(router = %router.name, port = %port, "applying router link nat flows");
        let bridge = &self.config.bridge_name;
        let Some(link_ip) = router.link_ip.as_ref() else {
            return Err(anyhow!("router '{}' has no link ip", router.name));
        };
        let Some(link_mac) = router.link_mac.as_ref() else {
            return Err(anyhow!("router '{}' has no link mac", router.name));
        };

        // Return flow: un-NAT packets returning from the external network and resubmit them.
        self.insert_flow(
            bridge,
            cookie,
            format!(
                "priority=235,in_port={port},ip,nw_dst={link_ip},actions=ct(zone=1,nat,table=0)"
            ),
        )
        .await?;

        // Per-route NAT flows for non-inter-subnet destinations.
        // route.source acts as a policy filter: only matching subnets get NATed.
        let router_ports = self.db.list_router_ports_for_router(&router.name)?;
        let router_routes = self.db.list_routes_for_router(&router.uuid)?;
        let attached_cidrs: Vec<String> = router_ports
            .iter()
            .filter_map(|rp| self.db.get_switch(&rp.switch_name).ok())
            .map(|s| format!("{}/{}", s.ip, s.mask))
            .collect();

        for route in &router_routes {
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

            let priority = (225 + u32::from(dst_prefix_len)).saturating_sub(route.metric);

            let mut new_actions = Vec::new();
            new_actions.push(format!("ct(commit,zone=1,nat(src={link_ip}))"));
            new_actions.push(format!("mod_dl_src:{link_mac}"));
            if let Some(next_hop_mac) = route.next_hop_mac.as_ref() {
                new_actions.push(format!("mod_dl_dst:{next_hop_mac}"));
            }
            new_actions.push("dec_ttl".to_string());
            new_actions.push(format!("output:{port}"));

            self.insert_flow(
                bridge,
                cookie,
                format!(
                    "priority={priority},ct_state=+new+trk,ip,nw_src={},nw_dst={},actions={}",
                    route.source,
                    route.destination,
                    new_actions.join(",")
                ),
            )
            .await?;

            let mut est_actions = Vec::new();
            est_actions.push("ct(zone=1,nat)".to_string());
            est_actions.push(format!("mod_dl_src:{link_mac}"));
            if let Some(next_hop_mac) = route.next_hop_mac.as_ref() {
                est_actions.push(format!("mod_dl_dst:{next_hop_mac}"));
            }
            est_actions.push("dec_ttl".to_string());
            est_actions.push(format!("output:{port}"));

            self.insert_flow(
                bridge,
                cookie,
                format!(
                    "priority={priority},ct_state=+est+trk,ip,nw_src={},nw_dst={},actions={}",
                    route.source,
                    route.destination,
                    est_actions.join(",")
                ),
            )
            .await?;
        }

        // Drop traffic from attached subnets not covered by any route's source.
        let covered_cidrs: Vec<&str> = router_routes.iter().map(|r| r.source.as_str()).collect();
        for cidr in &attached_cidrs {
            if !cidr_is_covered(cidr, &covered_cidrs) {
                self.insert_flow(
                    bridge,
                    cookie,
                    format!("priority=220,ct_state=+new+trk,ip,nw_src={cidr},actions=drop"),
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn apply_router_routes_flows(
        &self,
        of: &mut of::OF,
        cookie: u64,
        router: &Router,
    ) -> Result<()> {
        trace!(router = %router.name, "applying router route flows");
        let router_ports = self.db.list_router_ports_for_router(&router.name)?;
        let router_routes = self.db.list_routes_for_router(&router.uuid)?;

        let cidr_to_switch: std::collections::HashMap<String, &str> = router_ports
            .iter()
            .filter_map(|rp| {
                self.db
                    .get_switch(&rp.switch_name)
                    .ok()
                    .map(|s| (format!("{}/{}", s.ip, s.mask), rp.switch_name.as_str()))
            })
            .collect();

        for route in &router_routes {
            let Some(switch_name) = cidr_to_switch.get(&route.destination) else {
                continue;
            };

            let dest_ports = self.db.get_switch_ports_for_switch(switch_name)?;
            let mut dest_port_outputs = Vec::new();
            for sp in &dest_ports {
                dest_port_outputs.push(Action::output(
                    self.wait_for_ofport(&sp.interface_name).await?,
                ));
            }

            let dst_prefix_len = route
                .destination
                .split('/')
                .nth(1)
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);

            let priority = (225 + u32::from(dst_prefix_len)).saturating_sub(route.metric);
            let priority = u16::try_from(priority)
                .map_err(|_| anyhow!("route priority out of range: {priority}"))?;

            let mut actions = Vec::new();
            if let Some(next_hop_mac) = route.next_hop_mac.as_ref() {
                actions.push(Action::SetField(oxm::eth_dst(parse_mac(next_hop_mac)?)));
            }
            actions.extend(dest_port_outputs);

            let (source_ip, source_mask) = cidr_to_ipv4_masked(&route.source)?;
            let (destination_ip, destination_mask) = cidr_to_ipv4_masked(&route.destination)?;

            self.add_packet_flow(
                of,
                Self::add_flow(
                    cookie,
                    priority,
                    Match::new(vec![
                        oxm::eth_type(ETH_TYPE_IPV4),
                        oxm::ipv4_src_masked(source_ip, source_mask),
                        oxm::ipv4_dst_masked(destination_ip, destination_mask),
                    ]),
                    actions,
                ),
            )
            .await?;
        }

        Ok(())
    }

    async fn apply_router_link_flows(
        &self,
        cookie: u64,
        router: &crate::database::Router,
        switch: &crate::database::Switch,
        cidr: &str,
        port: &str,
    ) -> Result<()> {
        trace!(router = %router.name, switch = %switch.name, port = %port, "applying router link flows");
        let switch_ports = self.db.get_switch_ports_for_switch(&switch.name)?;

        self.insert_flow(
            &self.config.bridge_name,
            cookie,
            format!("priority=224,ip,nw_src={cidr},ct_state=-trk,actions=ct(table=0,zone=1)"),
        )
        .await?;

        for sp in switch_ports {
            let mut ingress_actions = Vec::new();
            if let Some(src_mac) = &router.link_mac {
                ingress_actions.push(format!("mod_dl_src:{src_mac}"));
            }

            ingress_actions.push(format!("mod_dl_dst:{}", sp.mac));
            ingress_actions.push("dec_ttl".to_string());
            ingress_actions.push(format!("output:{}", sp.interface_name));

            let mut direct_in_actions = vec!["ct(commit,zone=1)".to_string()];
            direct_in_actions.extend(ingress_actions);

            self.insert_flow(
                &self.config.bridge_name,
                cookie,
                format!(
                    "priority=230,in_port={port},ip,nw_dst={},actions={}",
                    sp.ip,
                    direct_in_actions.join(",")
                ),
            )
            .await?;
        }

        Ok(())
    }

    /// Delete all router flows for attached switches and optional link.
    ///
    /// # Errors
    /// Returns an error if the router is missing or flow deletion fails.
    pub async fn delete_router_flows(&self, name: &str) -> Result<()> {
        debug!(router = %name, "deleting router flows");
        let router = self.db.get_router(name)?;
        let cookie = Self::router_cookie(&router)?;
        let mut of = of::OF::connect(&self.config.bridge_name).await?;
        let _ = self.remove_flows(&mut of, cookie).await;

        Ok(())
    }
}

fn parse_ipv4(ip: &str) -> Result<[u8; 4]> {
    trace!(ip = %ip, "parsing ipv4");
    Ok(ip.parse::<Ipv4Addr>()?.octets())
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

fn cidr_to_ipv4_masked(cidr: &str) -> Result<([u8; 4], [u8; 4])> {
    trace!(cidr = %cidr, "parsing cidr to masked ipv4");
    let network: IpNetwork = cidr
        .parse()
        .map_err(|e| anyhow!("invalid CIDR '{cidr}': {e}"))?;

    match network {
        IpNetwork::V4(v4) => Ok((v4.network().octets(), mask_to_ipv4_mask(v4.prefix()))),
        IpNetwork::V6(_) => Err(anyhow!("IPv6 CIDRs are not supported: '{cidr}'")),
    }
}

fn mask_to_ipv4_mask(mask: u8) -> [u8; 4] {
    trace!(mask, "converting mask to ipv4 mask");
    if mask == 0 {
        return [0, 0, 0, 0];
    }
    (!0u32 << (32 - u32::from(mask))).to_be_bytes()
}

fn compute_gateway_ip(switch_ip: &str) -> Result<String> {
    trace!(switch_ip = %switch_ip, "computing gateway ip");
    let addr: std::net::Ipv4Addr = switch_ip
        .parse()
        .map_err(|e| anyhow!("invalid switch IP '{switch_ip}': {e}"))?;
    let mut octets = addr.octets();
    octets[3] = 1;
    Ok(format!(
        "{}.{}.{}.{}",
        octets[0], octets[1], octets[2], octets[3]
    ))
}

fn cidr_is_covered(cidr: &str, sources: &[&str]) -> bool {
    trace!(cidr = %cidr, source_count = sources.len(), "checking cidr coverage");
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

/// Run ovs-ofctl and return stdout on success.
async fn ovs_ofctl(args: &[&str]) -> Result<String> {
    trace!(arg_count = args.len(), "running ovs-ofctl");
    let output = tokio::process::Command::new("ovs-ofctl")
        .args(args)
        .output()
        .await?;
    if !output.status.success() {
        return Err(anyhow!(
            "ovs-ofctl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
