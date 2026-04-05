use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Switch {
    pub uuid: String,
    pub name: String,
    pub ip: String,
    pub mask: u8,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Router {
    pub uuid: String,
    pub name: String,
    pub link_name: Option<String>,
    pub link_ip: Option<String>,
    pub link_mac: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Interface {
    pub uuid: String,
    pub name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwitchPort {
    pub uuid: String,
    pub name: String,
    pub switch_name: String,
    pub interface_name: String,
    pub ip: String,
    pub mac: String,
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
    pub router_uuid: String,
    pub source: String,
    pub destination: String,
    pub next_hop: Option<String>,
    pub metric: u32,
}

pub struct Database {
    pub db_path: PathBuf,
}

/// Collect iterator results into a Vec with anyhow errors.
fn collect<T, E>(rows: impl Iterator<Item = std::result::Result<T, E>>) -> Result<Vec<T>>
where
    E: Into<anyhow::Error>,
{
    rows.map(|r| r.map_err(Into::into)).collect()
}

/// Map a database row into a Switch.
fn switch_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Switch> {
    Ok(Switch {
        uuid: row.get(0)?,
        name: row.get(1)?,
        ip: row.get(2)?,
        mask: row.get(3)?,
    })
}

/// Map a database row into a Router.
fn router_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Router> {
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
    Ok(Interface {
        uuid: row.get(0)?,
        name: row.get(1)?,
    })
}

/// Map a database row into a SwitchPort.
fn switch_port_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SwitchPort> {
    Ok(SwitchPort {
        uuid: row.get(0)?,
        name: row.get(1)?,
        switch_name: row.get(2)?,
        interface_name: row.get(3)?,
        ip: row.get(4)?,
        mac: row.get(5)?,
    })
}

/// Map a database row into a RouterPort.
fn router_port_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RouterPort> {
    Ok(RouterPort {
        uuid: row.get(0)?,
        router_name: row.get(1)?,
        switch_name: row.get(2)?,
    })
}

fn router_route_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RouterRoute> {
    Ok(RouterRoute {
        uuid: row.get(0)?,
        router_uuid: row.get(1)?,
        source: row.get(2)?,
        destination: row.get(3)?,
        next_hop: row.get(4)?,
        metric: row.get(5)?,
    })
}

impl Database {
    /// Create a new database handle for the given path.
    pub fn new(path: PathBuf) -> Self {
        Self { db_path: path }
    }

    /// Open a SQLite connection with foreign keys enabled.
    fn open(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .map_err(|e| anyhow!("failed to open database: {}", e))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(conn)
    }

    /// Initialize database tables if they do not exist.
    pub fn init(&self) -> Result<()> {
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
                name TEXT UNIQUE NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS switch_ports (
                uuid           TEXT PRIMARY KEY,
                name           TEXT UNIQUE NOT NULL,
                switch_uuid    TEXT NOT NULL REFERENCES switches(uuid) ON DELETE CASCADE,
                interface_uuid TEXT NOT NULL REFERENCES interfaces(uuid) ON DELETE RESTRICT,
                ip             TEXT NOT NULL,
                mac            TEXT NOT NULL
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
                metric      INTEGER NOT NULL DEFAULT 0,
                UNIQUE(router_uuid, source, destination)
            )",
            [],
        )?;

        Ok(())
    }

    /// Create a switch record.
    pub fn create_switch(&self, name: &str, ip: &str, mask: u8) -> Result<Switch> {
        let uuid = uuid::Uuid::new_v4().to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO switches (uuid, name, ip, mask) VALUES (?1, ?2, ?3, ?4)",
            params![uuid, name, ip, mask],
        )?;
        self.get_switch(&uuid)
    }

    /// Fetch a switch by name or UUID.
    pub fn get_switch(&self, name: &str) -> Result<Switch> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name, ip, mask FROM switches WHERE name = ?1 OR uuid = ?1")?;
        stmt.query_row(params![name], switch_from_row)
            .map_err(|e| anyhow!("switch not found: {}", e))
    }

    /// List all switches.
    pub fn list_switches(&self) -> Result<Vec<Switch>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT uuid, name, ip, mask FROM switches")?;
        collect(stmt.query_map([], switch_from_row)?)
    }

    /// Remove a switch by name.
    pub fn remove_switch(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM switches WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Create a router record.
    pub fn create_router(&self, name: &str) -> Result<Router> {
        let uuid = uuid::Uuid::new_v4().to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO routers (uuid, name) VALUES (?1, ?2)",
            params![uuid, name],
        )?;
        self.get_router(&uuid)
    }

    /// Fetch a router by name or UUID.
    pub fn get_router(&self, name: &str) -> Result<Router> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT uuid, name, link_name, link_ip, link_mac FROM routers WHERE name = ?1 OR uuid = ?1",
        )?;
        stmt.query_row(params![name], router_from_row)
            .map_err(|e| anyhow!("router not found: {}", e))
    }

    /// List all routers.
    pub fn list_routers(&self) -> Result<Vec<Router>> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name, link_name, link_ip, link_mac FROM routers")?;
        collect(stmt.query_map([], router_from_row)?)
    }

    pub fn get_router_link(&self, name: &str) -> Result<(String, String, String)> {
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
        .map_err(|e| anyhow!("router not found: {}", e))
    }

    /// Update router uplink metadata.
    pub fn update_router_link(
        &self,
        router_name: &str,
        name: Option<&str>,
        ip: Option<&str>,
        mac: Option<&str>,
    ) -> Result<()> {
        let conn = self.open()?;
        conn.execute(
            "UPDATE routers SET link_name = ?1, link_ip = ?2, link_mac = ?3 WHERE name = ?4",
            params![name, ip, mac, router_name],
        )?;
        Ok(())
    }

    /// Remove a router by name.
    pub fn remove_router(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM routers WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Create an interface record.
    pub fn create_interface(&self, name: &str) -> Result<Interface> {
        let uuid = uuid::Uuid::new_v4().to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO interfaces (uuid, name) VALUES (?1, ?2)",
            params![uuid, name],
        )?;
        self.get_interface(&uuid)
    }

    /// Fetch an interface by name or UUID.
    pub fn get_interface(&self, name: &str) -> Result<Interface> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT uuid, name FROM interfaces WHERE name = ?1 OR uuid = ?1")?;
        stmt.query_row(params![name], interface_from_row)
            .map_err(|e| anyhow!("interface not found: {}", e))
    }

    /// List all interfaces.
    pub fn list_interfaces(&self) -> Result<Vec<Interface>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT uuid, name FROM interfaces")?;
        collect(stmt.query_map([], interface_from_row)?)
    }

    /// Remove an interface by name.
    pub fn remove_interface(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM interfaces WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// Create a router-to-switch attachment.
    pub fn create_router_port(
        &self,
        router_name: &str,
        switch_name: &str,
        _ip: Option<&str>,
        _mac: Option<&str>,
    ) -> Result<RouterPort> {
        let uuid = uuid::Uuid::new_v4().to_string();
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
            .map_err(|e| anyhow!("router port not found: {}", e))
    }

    /// List router ports for a router.
    pub fn list_router_ports_for_router(&self, router_name: &str) -> Result<Vec<RouterPort>> {
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
    pub fn remove_router_port(&self, router_name: &str, switch_name: &str) -> Result<()> {
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
    pub fn create_switch_port(
        &self,
        name: &str,
        switch_name: &str,
        interface_name: &str,
        ip: &str,
        mac: &str,
    ) -> Result<SwitchPort> {
        let uuid = uuid::Uuid::new_v4().to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO switch_ports (uuid, name, switch_uuid, interface_uuid, ip, mac)
             VALUES (
                 ?1, ?2,
                 (SELECT uuid FROM switches WHERE name = ?3),
                 (SELECT uuid FROM interfaces WHERE name = ?4),
                 ?5, ?6
             )",
            params![uuid, name, switch_name, interface_name, ip, mac],
        )?;
        self.get_switch_port(name)
    }

    /// Fetch a switch port by name.
    pub fn get_switch_port(&self, name: &str) -> Result<SwitchPort> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT p.uuid, p.name, s.name, i.name, p.ip, p.mac
             FROM switch_ports p
             JOIN interfaces i ON p.interface_uuid = i.uuid
             JOIN switches s ON p.switch_uuid = s.uuid
             WHERE p.name = ?1 OR p.uuid = ?1",
        )?;
        stmt.query_row(params![name], switch_port_from_row)
            .map_err(|e| anyhow!("switch port not found: {}", e))
    }

    /// List all switch ports.
    pub fn list_switch_ports(&self) -> Result<Vec<SwitchPort>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT p.uuid, p.name, s.name, i.name, p.ip, p.mac
             FROM switch_ports p
             JOIN interfaces i ON p.interface_uuid = i.uuid
             JOIN switches s ON p.switch_uuid = s.uuid",
        )?;
        collect(stmt.query_map([], switch_port_from_row)?)
    }

    /// List switch ports for a switch.
    pub fn get_switch_ports_for_switch(&self, switch_name: &str) -> Result<Vec<SwitchPort>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT p.uuid, p.name, s.name, i.name, p.ip, p.mac
             FROM switch_ports p
             JOIN interfaces i ON p.interface_uuid = i.uuid
             JOIN switches s ON p.switch_uuid = s.uuid
             WHERE s.name = ?1",
        )?;
        collect(stmt.query_map(params![switch_name], switch_port_from_row)?)
    }

    /// Remove a switch port by name.
    pub fn remove_switch_port(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM switch_ports WHERE name = ?1", params![name])?;
        Ok(())
    }

    pub fn create_route(
        &self,
        router_uuid: &str,
        source: &str,
        destination: &str,
        next_hop: Option<&str>,
        metric: u32,
    ) -> Result<RouterRoute> {
        let uuid = uuid::Uuid::new_v4().to_string();
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO router_routes (uuid, router_uuid, source, destination, next_hop, metric)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![uuid, router_uuid, source, destination, next_hop, metric],
        )?;

        let mut stmt = conn.prepare(
            "SELECT uuid, router_uuid, source, destination, next_hop, metric
             FROM router_routes
             WHERE uuid = ?1",
        )?;
        stmt.query_row(params![uuid], router_route_from_row)
            .map_err(|e| anyhow!("route not found: {}", e))
    }

    pub fn list_routes_for_router(&self, router_uuid: &str) -> Result<Vec<RouterRoute>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT uuid, router_uuid, source, destination, next_hop, metric
             FROM router_routes
             WHERE router_uuid = ?1
             ORDER BY metric ASC",
        )?;
        collect(stmt.query_map(params![router_uuid], router_route_from_row)?)
    }

    pub fn remove_route(&self, router_uuid: &str, source: &str, destination: &str) -> Result<()> {
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
