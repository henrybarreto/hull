use crate::utils::{generate_random_mac, stable_uuid};
use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use tracing::{debug, trace};

/// A switch record stored in `SQLite`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Switch {
    /// Stable UUID for the switch.
    pub uuid: String,
    /// Human-readable switch name.
    pub name: String,
    /// IPv4 address assigned to the switch.
    pub ip: String,
    /// Network mask for the switch subnet.
    pub mask: u8,
}

/// A router record stored in `SQLite`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Router {
    /// Stable UUID for the router.
    pub uuid: String,
    /// Human-readable router name.
    pub name: String,
    /// Optional attached uplink interface name.
    pub link_name: Option<String>,
    /// Optional uplink IP address.
    pub link_ip: Option<String>,
    /// Optional uplink MAC address.
    pub link_mac: Option<String>,
}

/// A host interface record stored in `SQLite`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Interface {
    /// Stable UUID for the interface.
    pub uuid: String,
    /// Human-readable interface name.
    pub name: String,
    /// MAC address assigned to the interface.
    pub mac: String,
}

/// A port attached to a switch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwitchPort {
    /// Stable UUID for the port.
    pub uuid: String,
    /// Human-readable port name.
    pub name: String,
    /// Switch name this port belongs to.
    pub switch_name: String,
    /// Interface name backing this port.
    pub interface_name: String,
    /// IP address allocated to the port.
    pub ip: String,
    /// MAC address allocated to the port.
    pub mac: String,
}

/// A router-to-switch attachment record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouterPort {
    /// Stable UUID for the attachment.
    pub uuid: String,
    /// Router name for the attachment.
    pub router_name: String,
    /// Switch name for the attachment.
    pub switch_name: String,
}

/// A router route record stored in `SQLite`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouterRoute {
    /// Stable UUID for the route.
    pub uuid: String,
    /// Router UUID owning the route.
    pub router_uuid: String,
    /// Source subnet for the route.
    pub source: String,
    /// Destination subnet for the route.
    pub destination: String,
    /// Optional next hop IP address.
    pub next_hop: Option<String>,
    /// Optional next hop MAC address.
    pub next_hop_mac: Option<String>,
    /// Route metric.
    pub metric: u32,
}

/// `SQLite` database handle for Hull state.
pub struct Database {
    /// Path to the `SQLite` database file.
    pub db_path: PathBuf,
}

/// Collect iterator results into a Vec with anyhow errors.
fn collect<T, E>(rows: impl Iterator<Item = std::result::Result<T, E>>) -> Result<Vec<T>>
where
    E: Into<anyhow::Error>,
{
    trace!("collecting query results");
    rows.map(|r| r.map_err(Into::into)).collect()
}

/// Map a database row into a Switch.
fn switch_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Switch> {
    trace!("mapping switch row");
    Ok(Switch {
        uuid: row.get(0)?,
        name: row.get(1)?,
        ip: row.get(2)?,
        mask: row.get(3)?,
    })
}

/// Map a database row into a Router.
fn router_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Router> {
    trace!("mapping router row");
    Ok(Router {
        uuid: row.get(0)?,
        name: row.get(1)?,
        link_name: row.get(2)?,
        link_ip: row.get(3)?,
        link_mac: row.get(4)?,
    })
}

/// Map a database row into an Interface.
fn interface_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Interface> {
    trace!("mapping interface row");
    Ok(Interface {
        uuid: row.get(0)?,
        name: row.get(1)?,
        mac: row.get(2)?,
    })
}

/// Map a database row into a `SwitchPort`.
fn switch_port_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SwitchPort> {
    trace!("mapping switch port row");
    Ok(SwitchPort {
        uuid: row.get(0)?,
        name: row.get(1)?,
        switch_name: row.get(2)?,
        interface_name: row.get(3)?,
        ip: row.get(4)?,
        mac: row.get(5)?,
    })
}

/// Map a database row into a `RouterPort`.
fn router_port_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RouterPort> {
    trace!("mapping router port row");
    Ok(RouterPort {
        uuid: row.get(0)?,
        router_name: row.get(1)?,
        switch_name: row.get(2)?,
    })
}

fn router_route_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RouterRoute> {
    trace!("mapping router route row");
    Ok(RouterRoute {
        uuid: row.get(0)?,
        router_uuid: row.get(1)?,
        source: row.get(2)?,
        destination: row.get(3)?,
        next_hop: row.get(4)?,
        next_hop_mac: row.get(5)?,
        metric: row.get(6)?,
    })
}

impl Database {
    /// Create a new database handle for the given path.
    pub const fn new(path: PathBuf) -> Self {
        Self { db_path: path }
    }

    /// Open a `SQLite` connection with foreign keys enabled.
    fn open(&self) -> Result<Connection> {
        trace!(db_path = %self.db_path.display(), "opening sqlite database");
        let conn =
            Connection::open(&self.db_path).map_err(|e| anyhow!("failed to open database: {e}"))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(conn)
    }

    /// Initialize database tables if they do not exist.
    ///
    /// # Errors
    /// Returns an error if the database cannot be opened or a schema statement fails.
    pub fn init(&self) -> Result<()> {
        debug!(db_path = %self.db_path.display(), "initializing database schema");
        let conn = self.open()?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS switches (
                uuid TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                ip   TEXT NOT NULL,
                mask INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS routers (
                uuid TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                link_name TEXT,
                link_ip TEXT,
                link_mac TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS interfaces (
                uuid TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                mac  TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS switch_ports (
                uuid           TEXT PRIMARY KEY,
                name           TEXT UNIQUE NOT NULL,
                switch_uuid    TEXT NOT NULL REFERENCES switches(uuid) ON DELETE CASCADE,
                interface_uuid TEXT NOT NULL REFERENCES interfaces(uuid) ON DELETE RESTRICT,
                ip             TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS router_ports (
                uuid        TEXT PRIMARY KEY,
                router_uuid TEXT NOT NULL REFERENCES routers(uuid)  ON DELETE CASCADE,
                switch_uuid TEXT NOT NULL REFERENCES switches(uuid) ON DELETE CASCADE,
                UNIQUE (router_uuid, switch_uuid)
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS router_routes (
                uuid        TEXT PRIMARY KEY,
                router_uuid TEXT NOT NULL REFERENCES routers(uuid) ON DELETE CASCADE,
                source      TEXT NOT NULL,
                destination TEXT NOT NULL,
                next_hop    TEXT,
                next_hop_mac TEXT,
                metric      INTEGER NOT NULL DEFAULT 0,
                UNIQUE(router_uuid, source, destination)
            )",
            [],
        )?;

        self.ensure_interface_mac_column()?;
        self.ensure_router_routes_next_hop_mac_column()?;

        Ok(())
    }

    fn ensure_interface_mac_column(&self) -> Result<()> {
        trace!("ensuring interface mac column");
        let conn = self.open()?;
        let mut stmt = conn.prepare("PRAGMA table_info(interfaces)")?;
        let columns = collect(stmt.query_map([], |row| row.get::<_, String>(1))?)?;
        if !columns.iter().any(|c| c == "mac") {
            conn.execute("ALTER TABLE interfaces ADD COLUMN mac TEXT", [])?;
        }

        let mut stmt = conn.prepare("PRAGMA table_info(switch_ports)")?;
        let switch_port_columns = collect(stmt.query_map([], |row| row.get::<_, String>(1))?)?;
        let legacy_switch_port_mac = switch_port_columns.iter().any(|c| c == "mac");

        let mut stmt = conn.prepare("SELECT uuid, name, mac FROM interfaces")?;
        let interfaces = collect(stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?)?;

        for (uuid, name, mac) in interfaces {
            if let Some(mac) = mac.filter(|mac| !mac.is_empty()) {
                conn.execute(
                    "UPDATE interfaces SET mac = ?1 WHERE uuid = ?2",
                    params![mac, uuid],
                )?;
                continue;
            }

            let mac = if legacy_switch_port_mac {
                let mut stmt =
                    conn.prepare("SELECT mac FROM switch_ports WHERE interface_uuid = ?1 LIMIT 1")?;
                stmt.query_row(params![uuid], |row| row.get::<_, String>(0))
                    .ok()
                    .unwrap_or_else(generate_random_mac)
            } else {
                generate_random_mac()
            };

            debug!(interface = %name, mac = %mac, "backfilling interface mac");
            conn.execute(
                "UPDATE interfaces SET mac = ?1 WHERE uuid = ?2",
                params![mac, uuid],
            )?;
        }

        Ok(())
    }

    fn ensure_router_routes_next_hop_mac_column(&self) -> Result<()> {
        trace!("ensuring router route next_hop_mac column");
        let conn = self.open()?;
        let mut stmt = conn.prepare("PRAGMA table_info(router_routes)")?;
        let columns = collect(stmt.query_map([], |row| row.get::<_, String>(1))?)?;
        if !columns.iter().any(|c| c == "next_hop_mac") {
            conn.execute("ALTER TABLE router_routes ADD COLUMN next_hop_mac TEXT", [])?;
        }
        Ok(())
    }

    /// Create a switch record.
    ///
    /// # Errors
    /// Returns an error if the database insert fails or the switch cannot be reloaded.
    pub fn create_switch(&self, name: &str, ip: &str, mask: u8) -> Result<Switch> {
        debug!(switch = %name, ip = %ip, mask, "creating switch record");
        let uuid = stable_uuid("switch", &[name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO switches (uuid, name, ip, mask) VALUES (?1, ?2, ?3, ?4)",
            params![uuid, name, ip, mask],
        )?;
        self.get_switch(&uuid)
    }

    /// Fetch a switch by name or UUID.
    ///
    /// # Errors
    /// Returns an error if the switch does not exist or the query fails.
    pub fn get_switch(&self, name: &str) -> Result<Switch> {
        trace!(switch = %name, "fetching switch record");
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name, ip, mask FROM switches WHERE name = ?1 OR uuid = ?1")?;
        stmt.query_row(params![name], switch_from_row)
            .map_err(|e| anyhow!("switch not found: {e}"))
    }

    /// List all switches.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub fn list_switches(&self) -> Result<Vec<Switch>> {
        trace!("listing switch records");
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT uuid, name, ip, mask FROM switches")?;
        collect(stmt.query_map([], switch_from_row)?)
    }

    /// Remove a switch by name.
    ///
    /// # Errors
    /// Returns an error if the delete statement fails.
    pub fn remove_switch(&self, name: &str) -> Result<()> {
        debug!(switch = %name, "removing switch record");
        let conn = self.open()?;
        conn.execute("DELETE FROM switches WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Create a router record.
    ///
    /// # Errors
    /// Returns an error if the database insert fails or the router cannot be reloaded.
    pub fn create_router(&self, name: &str) -> Result<Router> {
        debug!(router = %name, "creating router record");
        let uuid = stable_uuid("router", &[name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO routers (uuid, name) VALUES (?1, ?2)",
            params![uuid, name],
        )?;
        self.get_router(&uuid)
    }

    /// Fetch a router by name or UUID.
    ///
    /// # Errors
    /// Returns an error if the router does not exist or the query fails.
    pub fn get_router(&self, name: &str) -> Result<Router> {
        trace!(router = %name, "fetching router record");
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT uuid, name, link_name, link_ip, link_mac FROM routers WHERE name = ?1 OR uuid = ?1",
        )?;
        stmt.query_row(params![name], router_from_row)
            .map_err(|e| anyhow!("router not found: {e}"))
    }

    /// List all routers.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub fn list_routers(&self) -> Result<Vec<Router>> {
        trace!("listing router records");
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name, link_name, link_ip, link_mac FROM routers")?;
        collect(stmt.query_map([], router_from_row)?)
    }

    ///
    /// # Errors
    /// Returns an error if the router does not exist or the query fails.
    pub fn get_router_link(&self, name: &str) -> Result<(String, String, String)> {
        trace!(router = %name, "fetching router link");
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT link_name, link_ip, link_mac FROM routers WHERE name = ?1")?;
        stmt.query_row(params![name], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        })
        .map_err(|e| anyhow!("router not found: {e}"))
    }

    /// Update router bridge-port metadata.
    ///
    /// # Errors
    /// Returns an error if the update fails.
    pub fn update_router_link(
        &self,
        router_name: &str,
        name: Option<&str>,
        ip: Option<&str>,
        mac: Option<&str>,
    ) -> Result<()> {
        debug!(router = %router_name, link_name = ?name, link_ip = ?ip, link_mac = ?mac, "updating router link");
        let conn = self.open()?;
        conn.execute(
            "UPDATE routers SET link_name = ?1, link_ip = ?2, link_mac = ?3 WHERE name = ?4",
            params![name, ip, mac, router_name],
        )?;
        Ok(())
    }

    /// Remove a router by name.
    ///
    /// # Errors
    /// Returns an error if the delete statement fails.
    pub fn remove_router(&self, name: &str) -> Result<()> {
        debug!(router = %name, "removing router record");
        let conn = self.open()?;
        conn.execute("DELETE FROM routers WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Create an interface record.
    ///
    /// # Errors
    /// Returns an error if the database insert fails or the interface cannot be reloaded.
    pub fn create_interface(&self, name: &str, mac: &str) -> Result<Interface> {
        debug!(interface = %name, "creating interface record");
        let uuid = stable_uuid("interface", &[name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO interfaces (uuid, name, mac) VALUES (?1, ?2, ?3)",
            params![uuid, name, mac],
        )?;
        self.get_interface(&uuid)
    }

    /// Fetch an interface by name or UUID.
    ///
    /// # Errors
    /// Returns an error if the interface does not exist or the query fails.
    pub fn get_interface(&self, name: &str) -> Result<Interface> {
        trace!(interface = %name, "fetching interface record");
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name, mac FROM interfaces WHERE name = ?1 OR uuid = ?1")?;
        stmt.query_row(params![name], interface_from_row)
            .map_err(|e| anyhow!("interface not found: {e}"))
    }

    /// List all interfaces.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub fn list_interfaces(&self) -> Result<Vec<Interface>> {
        trace!("listing interface records");
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT uuid, name, mac FROM interfaces")?;
        collect(stmt.query_map([], interface_from_row)?)
    }

    /// Remove an interface by name.
    ///
    /// # Errors
    /// Returns an error if the delete statement fails.
    pub fn remove_interface(&self, name: &str) -> Result<()> {
        debug!(interface = %name, "removing interface record");
        let conn = self.open()?;
        conn.execute("DELETE FROM interfaces WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Create a router-to-switch attachment.
    ///
    /// # Errors
    /// Returns an error if the insert or reload query fails.
    pub fn create_router_port(
        &self,
        router_name: &str,
        switch_name: &str,
        _ip: Option<&str>,
        _mac: Option<&str>,
    ) -> Result<RouterPort> {
        debug!(router = %router_name, switch = %switch_name, "creating router port record");
        let uuid = stable_uuid("router_port", &[router_name, switch_name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO router_ports (uuid, router_uuid, switch_uuid)
             VALUES (
                 ?1,
                 (SELECT uuid FROM routers WHERE name = ?2),
                 (SELECT uuid FROM switches WHERE name = ?3)
             )",
            params![uuid, router_name, switch_name],
        )?;

        let mut stmt = conn.prepare(
            "SELECT rp.uuid, r.name, s.name
             FROM router_ports rp
             JOIN routers r ON rp.router_uuid = r.uuid
             JOIN switches s ON rp.switch_uuid = s.uuid
             WHERE rp.uuid = ?1",
        )?;
        stmt.query_row(params![uuid], router_port_from_row)
            .map_err(|e| anyhow!("router port not found: {e}"))
    }

    /// List router ports for a router.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub fn list_router_ports_for_router(&self, router_name: &str) -> Result<Vec<RouterPort>> {
        trace!(router = %router_name, "listing router port records");
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT rp.uuid, r.name, s.name
             FROM router_ports rp
             JOIN routers r ON rp.router_uuid = r.uuid
             JOIN switches s ON rp.switch_uuid = s.uuid
             WHERE r.name = ?1",
        )?;
        collect(stmt.query_map(params![router_name], router_port_from_row)?)
    }

    /// Remove a router port by name.
    ///
    /// # Errors
    /// Returns an error if the delete statement fails.
    pub fn remove_router_port(&self, router_name: &str, switch_name: &str) -> Result<()> {
        debug!(router = %router_name, switch = %switch_name, "removing router port record");
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM router_ports 
             WHERE router_uuid = (SELECT uuid FROM routers WHERE name = ?1)
             AND switch_uuid = (SELECT uuid FROM switches WHERE name = ?2)",
            params![router_name, switch_name],
        )?;
        Ok(())
    }

    /// Create a switch port record.
    ///
    /// # Errors
    /// Returns an error if the insert or reload query fails.
    pub fn create_switch_port(
        &self,
        name: &str,
        switch_name: &str,
        interface_name: &str,
        ip: &str,
    ) -> Result<SwitchPort> {
        debug!(port = %name, switch = %switch_name, interface = %interface_name, "creating switch port record");
        let uuid = stable_uuid("switch_port", &[name]).to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO switch_ports (uuid, name, switch_uuid, interface_uuid, ip)
             VALUES (
                 ?1, ?2,
                 (SELECT uuid FROM switches WHERE name = ?3),
                 (SELECT uuid FROM interfaces WHERE name = ?4),
                 ?5
             )",
            params![uuid, name, switch_name, interface_name, ip],
        )?;
        self.get_switch_port(name)
    }

    /// Fetch a switch port by name.
    ///
    /// # Errors
    /// Returns an error if the switch port does not exist or the query fails.
    pub fn get_switch_port(&self, name: &str) -> Result<SwitchPort> {
        trace!(port = %name, "fetching switch port record");
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT p.uuid, p.name, s.name, i.name, p.ip, i.mac
             FROM switch_ports p
             JOIN interfaces i ON p.interface_uuid = i.uuid
             JOIN switches s ON p.switch_uuid = s.uuid
             WHERE p.name = ?1 OR p.uuid = ?1",
        )?;
        stmt.query_row(params![name], switch_port_from_row)
            .map_err(|e| anyhow!("switch port not found: {e}"))
    }

    /// List all switch ports.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub fn list_switch_ports(&self) -> Result<Vec<SwitchPort>> {
        trace!("listing switch port records");
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT p.uuid, p.name, s.name, i.name, p.ip, i.mac
             FROM switch_ports p
             JOIN interfaces i ON p.interface_uuid = i.uuid
             JOIN switches s ON p.switch_uuid = s.uuid",
        )?;
        collect(stmt.query_map([], switch_port_from_row)?)
    }

    /// List switch ports for a switch.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub fn get_switch_ports_for_switch(&self, switch_name: &str) -> Result<Vec<SwitchPort>> {
        trace!(switch = %switch_name, "listing switch port records for switch");
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT p.uuid, p.name, s.name, i.name, p.ip, i.mac
             FROM switch_ports p
             JOIN interfaces i ON p.interface_uuid = i.uuid
             JOIN switches s ON p.switch_uuid = s.uuid
             WHERE s.name = ?1",
        )?;
        collect(stmt.query_map(params![switch_name], switch_port_from_row)?)
    }

    /// Remove a switch port by name.
    ///
    /// # Errors
    /// Returns an error if the delete statement fails.
    pub fn remove_switch_port(&self, name: &str) -> Result<()> {
        debug!(port = %name, "removing switch port record");
        let conn = self.open()?;
        conn.execute("DELETE FROM switch_ports WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Create a router route.
    ///
    /// # Errors
    /// Returns an error if the insert or reload query fails.
    pub fn create_route(
        &self,
        router_uuid: &str,
        source: &str,
        destination: &str,
        next_hop: Option<&str>,
        next_hop_mac: Option<&str>,
        metric: u32,
    ) -> Result<RouterRoute> {
        debug!(router_uuid = %router_uuid, source = %source, destination = %destination, metric, "creating route record");
        let uuid = stable_uuid("route", &[router_uuid, source, destination]).to_string();
        let conn = self.open()?;
        if let Some(next_hop) = next_hop
            && next_hop_mac.is_none()
        {
            return Err(anyhow!(
                "route with next hop '{next_hop}' requires a next-hop MAC"
            ));
        }
        conn.execute(
            "INSERT INTO router_routes (uuid, router_uuid, source, destination, next_hop, next_hop_mac, metric)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![uuid, router_uuid, source, destination, next_hop, next_hop_mac, metric],
        )?;

        let mut stmt = conn.prepare(
            "SELECT uuid, router_uuid, source, destination, next_hop, next_hop_mac, metric
             FROM router_routes
             WHERE uuid = ?1",
        )?;
        stmt.query_row(params![uuid], router_route_from_row)
            .map_err(|e| anyhow!("route not found: {e}"))
    }

    /// List routes for a router.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    pub fn list_routes_for_router(&self, router_uuid: &str) -> Result<Vec<RouterRoute>> {
        trace!(router_uuid = %router_uuid, "listing route records for router");
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT uuid, router_uuid, source, destination, next_hop, next_hop_mac, metric
             FROM router_routes
             WHERE router_uuid = ?1
             ORDER BY metric ASC",
        )?;
        collect(stmt.query_map(params![router_uuid], router_route_from_row)?)
    }

    /// Remove a router route.
    ///
    /// # Errors
    /// Returns an error if the delete statement fails.
    pub fn remove_route(&self, router_uuid: &str, source: &str, destination: &str) -> Result<()> {
        debug!(router_uuid = %router_uuid, source = %source, destination = %destination, "removing route record");
        let conn = self.open()?;
        conn.execute(
            "DELETE FROM router_routes
             WHERE router_uuid = ?1
             AND source = ?2
             AND destination = ?3",
            params![router_uuid, source, destination],
        )?;
        Ok(())
    }
}
