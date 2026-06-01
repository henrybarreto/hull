use crate::utils::{generate_random_mac, stable_uuid};
use anyhow::{Result, anyhow};
use ipnetwork::IpNetwork;
use rusqlite::{Connection, params};
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Switch {
    pub uuid: String,
    pub name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Subnet {
    pub uuid: String,
    pub switch_uuid: String,
    pub name: String,
    pub cidr: String,
    pub gateway_ip: String,
    pub gateway_mac: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwitchPort {
    pub uuid: String,
    pub switch_uuid: String,
    pub subnet_uuid: String,
    pub name: String,
    pub tap_name: String,
    pub ip: String,
    pub mac: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Router {
    pub uuid: String,
    pub name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouterPort {
    pub uuid: String,
    pub router_name: String,
    pub switch_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouterRoute {
    pub uuid: String,
    pub router_name: String,
    pub source: String,
    pub destination: String,
    pub next_hop: Option<String>,
    pub next_hop_mac: Option<String>,
    pub metric: u32,
}

pub struct Database {
    pub db_path: PathBuf,
}

impl Database {
    pub const fn new(path: PathBuf) -> Self {
        Self { db_path: path }
    }

    fn open(&self) -> Result<Connection> {
        let conn =
            Connection::open(&self.db_path).map_err(|e| anyhow!("failed to open database: {e}"))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(conn)
    }

    pub fn init(&self) -> Result<()> {
        let conn = self.open()?;

        // Migration: legacy table name `networks` -> `switches`.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS switches (
                uuid TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL
            )",
            [],
        )?;
        let has_networks = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='networks' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .is_ok();
        let has_switches = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='switches' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .is_ok();
        if has_networks && has_switches {
            let switch_count: i64 =
                conn.query_row("SELECT COUNT(*) FROM switches", [], |row| row.get(0))?;
            if switch_count == 0 {
                conn.execute(
                    "INSERT INTO switches (uuid, name) SELECT uuid, name FROM networks",
                    [],
                )?;
            }
        }
        let _ = conn.execute("DROP TABLE IF EXISTS networks", []);
        let _ = conn.execute("DROP TABLE IF EXISTS endpoints", []);
        let _ = conn.execute("DROP TABLE IF EXISTS switch_ports", []);
        let _ = conn.execute("DROP TABLE IF EXISTS interfaces", []);
        let _ = conn.execute("DROP TABLE IF EXISTS subnets", []);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS subnets (
                uuid TEXT PRIMARY KEY,
                switch_uuid TEXT NOT NULL REFERENCES switches(uuid) ON DELETE CASCADE,
                name TEXT NOT NULL,
                cidr TEXT NOT NULL,
                gateway_ip TEXT NOT NULL,
                gateway_mac TEXT NOT NULL,
                UNIQUE(switch_uuid, name),
                UNIQUE(switch_uuid, cidr)
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS switch_ports (
                uuid TEXT PRIMARY KEY,
                switch_uuid TEXT NOT NULL REFERENCES switches(uuid) ON DELETE CASCADE,
                subnet_uuid TEXT NOT NULL REFERENCES subnets(uuid) ON DELETE CASCADE,
                name TEXT NOT NULL,
                tap_name TEXT NOT NULL,
                ip TEXT NOT NULL,
                mac TEXT NOT NULL,
                UNIQUE(switch_uuid, name),
                UNIQUE(subnet_uuid, ip),
                UNIQUE(tap_name)
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS routers (
                uuid TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS router_ports (
                uuid TEXT PRIMARY KEY,
                router_uuid TEXT NOT NULL REFERENCES routers(uuid) ON DELETE CASCADE,
                switch_uuid TEXT NOT NULL REFERENCES switches(uuid) ON DELETE CASCADE,
                UNIQUE(router_uuid, switch_uuid)
            )",
            [],
        )?;
        let _ = conn.execute("DROP TABLE IF EXISTS router_uplinks", []);
        conn.execute(
            "CREATE TABLE IF NOT EXISTS router_routes (
                uuid TEXT PRIMARY KEY,
                router_uuid TEXT NOT NULL REFERENCES routers(uuid) ON DELETE CASCADE,
                source TEXT NOT NULL,
                destination TEXT NOT NULL,
                next_hop TEXT,
                next_hop_mac TEXT,
                metric INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        Ok(())
    }

    pub fn create_switch(&self, name: &str) -> Result<Switch> {
        let uuid = stable_uuid("switch", &[name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO switches (uuid, name) VALUES (?1, ?2)",
            params![uuid, name],
        )?;
        self.get_switch(name)
    }

    pub fn get_switch(&self, id: &str) -> Result<Switch> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name FROM switches WHERE name = ?1 OR uuid = ?1")?;
        stmt.query_row(params![id], |row| {
            Ok(Switch {
                uuid: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| anyhow!("switch not found: {e}"))
    }

    pub fn list_switches(&self) -> Result<Vec<Switch>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT uuid, name FROM switches")?;
        let rows = stmt.query_map([], |row| {
            Ok(Switch {
                uuid: row.get(0)?,
                name: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn remove_switch(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM switches WHERE name = ?1", params![name])?;
        Ok(())
    }

    pub fn create_subnet(
        &self,
        switch_name: &str,
        name: &str,
        cidr: &str,
        gateway_ip: &str,
        gateway_mac: &str,
    ) -> Result<Subnet> {
        let net = self.get_switch(switch_name)?;
        let uuid = stable_uuid("subnet", &[&net.uuid, name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO subnets (uuid, switch_uuid, name, cidr, gateway_ip, gateway_mac)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![uuid, net.uuid, name, cidr, gateway_ip, gateway_mac],
        )?;
        self.get_subnet(switch_name, name)
    }

    pub fn get_subnet(&self, switch_name: &str, name: &str) -> Result<Subnet> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT s.uuid, s.switch_uuid, s.name, s.cidr, s.gateway_ip, s.gateway_mac
             FROM subnets s JOIN switches n ON n.uuid = s.switch_uuid
             WHERE n.name = ?1 AND s.name = ?2",
        )?;
        stmt.query_row(params![switch_name, name], |row| {
            Ok(Subnet {
                uuid: row.get(0)?,
                switch_uuid: row.get(1)?,
                name: row.get(2)?,
                cidr: row.get(3)?,
                gateway_ip: row.get(4)?,
                gateway_mac: row.get(5)?,
            })
        })
        .map_err(|e| anyhow!("subnet not found: {e}"))
    }

    pub fn list_subnets(&self, switch_name: &str) -> Result<Vec<Subnet>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT s.uuid, s.switch_uuid, s.name, s.cidr, s.gateway_ip, s.gateway_mac
             FROM subnets s JOIN switches n ON n.uuid = s.switch_uuid
             WHERE n.name = ?1",
        )?;
        let rows = stmt.query_map(params![switch_name], |row| {
            Ok(Subnet {
                uuid: row.get(0)?,
                switch_uuid: row.get(1)?,
                name: row.get(2)?,
                cidr: row.get(3)?,
                gateway_ip: row.get(4)?,
                gateway_mac: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn remove_subnet(&self, switch_name: &str, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM subnets
             WHERE uuid IN (
               SELECT s.uuid FROM subnets s JOIN switches n ON n.uuid = s.switch_uuid
               WHERE n.name = ?1 AND s.name = ?2
             )",
            params![switch_name, name],
        )?;
        Ok(())
    }

    pub fn create_switch_port(
        &self,
        switch_name: &str,
        subnet_name: &str,
        name: &str,
        tap: &str,
        ip: &str,
        mac: &str,
    ) -> Result<SwitchPort> {
        let subnet = self.get_subnet(switch_name, subnet_name)?;
        let uuid = stable_uuid("switch-port", &[&subnet.uuid, name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO switch_ports (uuid, switch_uuid, subnet_uuid, name, tap_name, ip, mac)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![uuid, subnet.switch_uuid, subnet.uuid, name, tap, ip, mac],
        )?;
        self.get_switch_port(switch_name, name)
    }

    pub fn get_switch_port(&self, switch_name: &str, name: &str) -> Result<SwitchPort> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT e.uuid, e.switch_uuid, e.subnet_uuid, e.name, e.tap_name, e.ip, e.mac
             FROM switch_ports e
             JOIN switches n ON n.uuid = e.switch_uuid
             WHERE n.name = ?1 AND e.name = ?2",
        )?;
        stmt.query_row(params![switch_name, name], |row| {
            Ok(SwitchPort {
                uuid: row.get(0)?,
                switch_uuid: row.get(1)?,
                subnet_uuid: row.get(2)?,
                name: row.get(3)?,
                tap_name: row.get(4)?,
                ip: row.get(5)?,
                mac: row.get(6)?,
            })
        })
        .map_err(|e| anyhow!("switch port not found: {e}"))
    }

    pub fn list_switch_ports(&self, switch_name: &str) -> Result<Vec<SwitchPort>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT e.uuid, e.switch_uuid, e.subnet_uuid, e.name, e.tap_name, e.ip, e.mac
             FROM switch_ports e
             JOIN switches n ON n.uuid = e.switch_uuid
             WHERE n.name = ?1",
        )?;
        let rows = stmt.query_map(params![switch_name], |row| {
            Ok(SwitchPort {
                uuid: row.get(0)?,
                switch_uuid: row.get(1)?,
                subnet_uuid: row.get(2)?,
                name: row.get(3)?,
                tap_name: row.get(4)?,
                ip: row.get(5)?,
                mac: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn list_all_switch_ports(&self) -> Result<Vec<SwitchPort>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT e.uuid, e.switch_uuid, e.subnet_uuid, e.name, e.tap_name, e.ip, e.mac
             FROM switch_ports e",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SwitchPort {
                uuid: row.get(0)?,
                switch_uuid: row.get(1)?,
                subnet_uuid: row.get(2)?,
                name: row.get(3)?,
                tap_name: row.get(4)?,
                ip: row.get(5)?,
                mac: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn remove_switch_port(&self, switch_name: &str, name: &str) -> Result<()> {
        let switch_port = self.get_switch_port(switch_name, name)?;
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM switch_ports WHERE uuid = ?1",
            params![switch_port.uuid],
        )?;
        Ok(())
    }

    pub fn list_subnets_for_switch_uuid(&self, switch_uuid: &str) -> Result<Vec<Subnet>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT uuid, switch_uuid, name, cidr, gateway_ip, gateway_mac FROM subnets WHERE switch_uuid = ?1",
        )?;
        let rows = stmt.query_map(params![switch_uuid], |row| {
            Ok(Subnet {
                uuid: row.get(0)?,
                switch_uuid: row.get(1)?,
                name: row.get(2)?,
                cidr: row.get(3)?,
                gateway_ip: row.get(4)?,
                gateway_mac: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_subnet_by_uuid(&self, subnet_uuid: &str) -> Result<Subnet> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT uuid, switch_uuid, name, cidr, gateway_ip, gateway_mac FROM subnets WHERE uuid = ?1",
        )?;
        stmt.query_row(params![subnet_uuid], |row| {
            Ok(Subnet {
                uuid: row.get(0)?,
                switch_uuid: row.get(1)?,
                name: row.get(2)?,
                cidr: row.get(3)?,
                gateway_ip: row.get(4)?,
                gateway_mac: row.get(5)?,
            })
        })
        .map_err(|e| anyhow!("subnet not found: {e}"))
    }

    pub fn allocate_switch_port_ip(&self, switch_name: &str, subnet_name: &str) -> Result<String> {
        let subnet = self.get_subnet(switch_name, subnet_name)?;
        let network: IpNetwork = subnet
            .cidr
            .parse()
            .map_err(|e| anyhow!("invalid subnet cidr '{}': {e}", subnet.cidr))?;

        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT ip FROM switch_ports WHERE subnet_uuid = ?1")?;
        let used: std::collections::HashSet<String> = stmt
            .query_map(params![subnet.uuid], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .collect();

        for ip in &network {
            let ip_s = ip.to_string();
            if ip == network.network() {
                continue;
            }
            if let IpNetwork::V4(v4) = network
                && ip == std::net::IpAddr::V4(v4.broadcast())
            {
                continue;
            }
            if ip_s == subnet.gateway_ip {
                continue;
            }
            if !used.contains(&ip_s) {
                return Ok(ip_s);
            }
        }

        Err(anyhow!(
            "no available switch port IP in subnet '{}'",
            subnet.name
        ))
    }

    pub fn reset_schema(&self) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DROP TABLE IF EXISTS switch_ports", [])?;
        conn.execute("DROP TABLE IF EXISTS interfaces", [])?;
        conn.execute("DROP TABLE IF EXISTS subnets", [])?;
        conn.execute("DROP TABLE IF EXISTS switches", [])?;
        conn.execute("DROP TABLE IF EXISTS router_routes", [])?;
        conn.execute("DROP TABLE IF EXISTS router_uplinks", [])?;
        conn.execute("DROP TABLE IF EXISTS router_ports", [])?;
        conn.execute("DROP TABLE IF EXISTS switch_ports", [])?;
        conn.execute("DROP TABLE IF EXISTS routers", [])?;
        conn.execute("DROP TABLE IF EXISTS switches", [])?;
        Ok(())
    }
}

impl Database {
    pub fn create_router(&self, name: &str) -> Result<Router> {
        let uuid = stable_uuid("router", &[name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO routers (uuid, name) VALUES (?1, ?2)",
            params![uuid, name],
        )?;
        self.get_router(name)
    }

    pub fn get_router(&self, name: &str) -> Result<Router> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name FROM routers WHERE name = ?1 OR uuid = ?1")?;
        stmt.query_row(params![name], |row| {
            Ok(Router {
                uuid: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| anyhow!("router not found: {e}"))
    }

    pub fn list_routers(&self) -> Result<Vec<Router>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT uuid, name FROM routers")?;
        let rows = stmt.query_map([], |row| {
            Ok(Router {
                uuid: row.get(0)?,
                name: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn remove_router(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM routers WHERE name = ?1", params![name])?;
        Ok(())
    }

    pub fn create_router_port(&self, router: &str, switch: &str) -> Result<RouterPort> {
        let router = self.get_router(router)?;
        let switch = self.get_switch(switch)?;
        let uuid = stable_uuid("router-port", &[&router.uuid, &switch.uuid]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO router_ports (uuid, router_uuid, switch_uuid) VALUES (?1, ?2, ?3)",
            params![uuid, router.uuid, switch.uuid],
        )?;
        self.get_router_port_by_ids(&router.uuid, &switch.uuid)
    }

    pub fn remove_router_port(&self, router: &str, switch: &str) -> Result<()> {
        let router = self.get_router(router)?;
        let switch = self.get_switch(switch)?;
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM router_ports WHERE router_uuid = ?1 AND switch_uuid = ?2",
            params![router.uuid, switch.uuid],
        )?;
        Ok(())
    }

    pub fn list_router_ports_for_router(&self, router: &str) -> Result<Vec<RouterPort>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT rp.uuid, r.name, n.name
             FROM router_ports rp
             JOIN routers r ON r.uuid = rp.router_uuid
             JOIN switches n ON n.uuid = rp.switch_uuid
             WHERE r.name = ?1",
        )?;
        let rows = stmt.query_map(params![router], |row| {
            Ok(RouterPort {
                uuid: row.get(0)?,
                router_name: row.get(1)?,
                switch_name: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn list_attached_switch_uuids(&self) -> Result<Vec<String>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT DISTINCT switch_uuid FROM router_ports")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn get_router_port_by_ids(&self, router_uuid: &str, switch_uuid: &str) -> Result<RouterPort> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT rp.uuid, r.name, n.name
             FROM router_ports rp
             JOIN routers r ON r.uuid = rp.router_uuid
             JOIN switches n ON n.uuid = rp.switch_uuid
             WHERE rp.router_uuid = ?1 AND rp.switch_uuid = ?2",
        )?;
        stmt.query_row(params![router_uuid, switch_uuid], |row| {
            Ok(RouterPort {
                uuid: row.get(0)?,
                router_name: row.get(1)?,
                switch_name: row.get(2)?,
            })
        })
        .map_err(|e| anyhow!("router port not found: {e}"))
    }

    pub fn create_router_route(
        &self,
        router: &str,
        source: &str,
        destination: &str,
        next_hop: Option<&str>,
        next_hop_mac: Option<&str>,
        metric: u32,
    ) -> Result<RouterRoute> {
        let router = self.get_router(router)?;
        let uuid = stable_uuid("router-route", &[&router.uuid, source, destination]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO router_routes (uuid, router_uuid, source, destination, next_hop, next_hop_mac, metric)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![uuid, router.uuid, source, destination, next_hop, next_hop_mac, metric],
        )?;
        self.get_router_route_by_uuid(&uuid)
    }

    pub fn remove_router_route(&self, router: &str, source: &str, destination: &str) -> Result<()> {
        let router = self.get_router(router)?;
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM router_routes WHERE router_uuid = ?1 AND source = ?2 AND destination = ?3",
            params![router.uuid, source, destination],
        )?;
        Ok(())
    }

    pub fn list_router_routes(&self, router: &str) -> Result<Vec<RouterRoute>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT rr.uuid, r.name, rr.source, rr.destination, rr.next_hop, rr.next_hop_mac, rr.metric
             FROM router_routes rr
             JOIN routers r ON r.uuid = rr.router_uuid
             WHERE r.name = ?1",
        )?;
        let rows = stmt.query_map(params![router], |row| {
            Ok(RouterRoute {
                uuid: row.get(0)?,
                router_name: row.get(1)?,
                source: row.get(2)?,
                destination: row.get(3)?,
                next_hop: row.get(4)?,
                next_hop_mac: row.get(5)?,
                metric: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn get_router_route_by_uuid(&self, uuid: &str) -> Result<RouterRoute> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT rr.uuid, r.name, rr.source, rr.destination, rr.next_hop, rr.next_hop_mac, rr.metric
             FROM router_routes rr
             JOIN routers r ON r.uuid = rr.router_uuid
             WHERE rr.uuid = ?1",
        )?;
        stmt.query_row(params![uuid], |row| {
            Ok(RouterRoute {
                uuid: row.get(0)?,
                router_name: row.get(1)?,
                source: row.get(2)?,
                destination: row.get(3)?,
                next_hop: row.get(4)?,
                next_hop_mac: row.get(5)?,
                metric: row.get(6)?,
            })
        })
        .map_err(|e| anyhow!("router route not found: {e}"))
    }
}

pub fn ensure_mac_or_generate(mac: Option<&str>) -> String {
    mac.map_or_else(generate_random_mac, std::string::ToString::to_string)
}
