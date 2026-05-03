//! Shared test harness utilities.

#![allow(dead_code)]
#![allow(clippy::redundant_clone)]

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::process::Command;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use hull::config::Config;
use hull::database::Database;
use hull::interfaces::InterfaceOps;
use hull::ovs::BridgeClient;
use hull::routers::RouterOps;
use hull::switches::SwitchOps;

fn root_or_panic() {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|out| out.trim().parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    assert!(
        uid == 0,
        "Flow snapshot tests must be run as root.\n\
         Re-run with: sudo env \"PATH=$PATH\" cargo test"
    );
}

const SNAPSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots");
const CLIENT_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/client/Cargo.toml");
const DAEMON_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/daemon/Cargo.toml");

/// Shared harness for flow snapshot tests.
pub struct TestFlowsHarness {
    /// Temporary root directory for the harness.
    pub root: PathBuf,
    /// Bridge name under test.
    pub bridge: String,
    /// Random suffix used to keep resources unique.
    pub suffix: String,
    /// Shared database handle.
    pub db: Arc<Database>,
    /// Shared OVSDB client.
    pub ovs: Arc<BridgeClient>,
    /// Switch operations helper.
    pub switch_ops: Arc<SwitchOps>,
    /// Interface operations helper.
    pub interface_ops: Arc<InterfaceOps>,
    /// Router operations helper.
    pub router_ops: Arc<RouterOps>,
}

impl TestFlowsHarness {
    /// Create a flow snapshot harness.
    ///
    /// # Errors
    /// Returns an error if the temporary directory, database, or OVS bridge cannot be created.
    pub fn new(tag: &str) -> Result<Self> {
        root_or_panic();

        let suffix: String = (0..4)
            .map(|_| {
                let c = rand::random::<u8>() % 26;
                (b'a' + c) as char
            })
            .collect();

        let bridge = format!("ht-{suffix}");
        let root = PathBuf::from(format!("/tmp/hull-test-{tag}-{suffix}"));

        fs::create_dir_all(&root)?;

        let config = Config {
            bridge_name: bridge.clone(),
        };
        let config_path = root.join("hull.json");
        config.save(&config_path)?;

        let db_path = root.join("hull.db");
        let db = Arc::new(Database::new(db_path));
        db.init()?;

        let config_arc = Arc::new(config);
        let ovs = Arc::new(BridgeClient::connect()?);
        let _ = ovs.del_bridge(&bridge);
        ovs.add_bridge(&bridge)?;

        let switch_ops = Arc::new(SwitchOps::new(db.clone(), config_arc.clone(), ovs.clone()));
        let interface_ops = Arc::new(InterfaceOps::new(db.clone()));
        let router_ops = Arc::new(RouterOps::new(db.clone(), config_arc.clone(), ovs.clone()));

        Ok(Self {
            root,
            bridge,
            suffix,
            db,
            ovs,
            switch_ops,
            interface_ops,
            router_ops,
        })
    }

    fn is_mac(s: &str) -> bool {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 6 {
            return false;
        }
        parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
    }

    fn normalize_field_value(key: &str, value: &str) -> String {
        match key {
            "cookie" => "<COOKIE>".to_string(),
            "duration" => "<DURATION>".to_string(),
            "n_packets" => "<N_PACKETS>".to_string(),
            "n_bytes" => "<N_BYTES>".to_string(),
            "idle_age" => "<IDLE_AGE>".to_string(),
            "in_port" if value.chars().all(|c| c.is_ascii_digit()) => "<PORT>".to_string(),
            _ if Self::is_mac(value) => "<MAC>".to_string(),
            _ => value.to_string(),
        }
    }

    fn push_normalized_field(line: &mut String, key: &str, value: &str) {
        let _ = if line.is_empty() {
            write!(line, "{key}={value}")
        } else {
            write!(line, ", {key}={value}")
        };
    }

    fn normalize_flows(raw: &str) -> String {
        let mut normalized = Vec::new();

        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("NXST_FLOW") || trimmed.starts_with("OFPST_FLOW") {
                normalized.push(trimmed.to_string());
                continue;
            }

            let mut normalized_line = String::new();
            let groups: Vec<&str> = trimmed.split(' ').filter(|s| !s.is_empty()).collect();
            for group in groups {
                let fields: Vec<&str> = group.split(',').filter(|s| !s.is_empty()).collect();
                for field in fields {
                    let (key, value) = field.split_once('=').unwrap_or((field, ""));
                    let norm_value = Self::normalize_field_value(key, value);
                    Self::push_normalized_field(&mut normalized_line, key, &norm_value);
                }
            }

            normalized.push(normalized_line);
        }

        normalized.join("\n")
    }

    fn diff(expected: &str, actual: &str) -> String {
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();
        let max_len = expected_lines.len().max(actual_lines.len());
        let mut out = String::new();

        for i in 0..max_len {
            let exp = expected_lines.get(i).copied().unwrap_or("<missing>");
            let act = actual_lines.get(i).copied().unwrap_or("<missing>");
            if exp != act {
                let _ = write!(
                    out,
                    "  line {}: expected: {:?}\n         actual:   {:?}\n",
                    i + 1,
                    exp,
                    act
                );
            }
        }
        out
    }

    /// Dump and normalize OVS flows.
    ///
    /// # Errors
    /// Returns an error if `ovs-ofctl` cannot be executed or reports failure.
    pub fn dump_flows(&self) -> Result<String> {
        let output = Command::new("ovs-ofctl")
            .args(["dump-flows", &self.bridge])
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("ovs-ofctl dump-flows failed:\n{stderr}"));
        }

        Ok(Self::normalize_flows(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    /// Compare the current flows against a snapshot.
    ///
    /// # Errors
    /// Returns an error if the snapshot is missing, cannot be written, or does not match.
    pub fn assert(&self, name: &str) -> Result<()> {
        let snapshot_path = format!("{SNAPSHOT_DIR}/{name}.txt");
        let current = self.dump_flows()?;

        if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
            fs::write(&snapshot_path, &current)?;
            return Ok(());
        }

        let expected = fs::read_to_string(&snapshot_path).map_err(|_| {
            anyhow!("Snapshot not found: {snapshot_path}\nRun with UPDATE_SNAPSHOTS=1 to generate.")
        })?;

        if current != expected {
            let diff = Self::diff(&expected, &current);
            return Err(anyhow!("Flow snapshot mismatch for '{name}':\n\n{diff}"));
        }

        Ok(())
    }
}

impl Drop for TestFlowsHarness {
    fn drop(&mut self) {
        let bridge = self.bridge.clone();
        let ovs = self.ovs.clone();
        let _ = ovs.del_bridge(&bridge);

        if let Ok(interfaces) = self.db.list_interfaces() {
            for iface in interfaces {
                let _ = hull::interfaces::Interface::delete(&iface.name);
            }
        }

        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Shared harness for CLI integration tests.
pub struct CliTestHarness {
    /// Temporary root directory for the harness.
    pub root: PathBuf,
    /// Bridge name under test.
    pub bridge: String,
    /// Temporary TAP interface name.
    pub iface: String,
    /// Temporary switch name.
    pub switch: String,
    /// Temporary router name.
    pub router: String,
    /// Temporary port name.
    pub port: String,
    daemon: Child,
}

impl CliTestHarness {
    fn binary_path(manifest_path: &str, bin_name: &str) -> Result<PathBuf> {
        let package_dir = PathBuf::from(manifest_path)
            .parent()
            .ok_or_else(|| anyhow!("invalid manifest path: {manifest_path}"))?
            .to_path_buf();
        let binary_path = package_dir.join("target").join("debug").join(bin_name);
        if !binary_path.exists() {
            let status = Command::new("cargo")
                .args(["build", "--manifest-path", manifest_path, "--bin", bin_name])
                .status()?;
            if !status.success() {
                return Err(anyhow!(
                    "failed to build binary '{bin_name}' from '{manifest_path}'"
                ));
            }
        }

        Ok(binary_path)
    }

    /// Create a CLI integration harness.
    ///
    /// # Errors
    /// Returns an error if the daemon process cannot be spawned or the temp dir cannot be created.
    pub fn new() -> Result<Self> {
        root_or_panic();

        let suffix: String = (0..4)
            .map(|_| {
                let c = rand::random::<u8>() % 26;
                (b'a' + c) as char
            })
            .collect();
        let bridge = format!("ht-{suffix}");
        let root = PathBuf::from(format!("/tmp/hull-cli-{suffix}"));
        let _ = fs::remove_dir_all(&root);

        fs::create_dir_all(&root)?;

        let daemon = Self::spawn_daemon(&root, &bridge)?;
        let harness = Self {
            root,
            bridge,
            iface: format!("tap-{suffix}"),
            switch: format!("sw-{suffix}"),
            router: format!("rt-{suffix}"),
            port: format!("pt-{suffix}"),
            daemon,
        };
        Self::wait_for_socket(&harness.root.join("hulld.sock"))?;

        Ok(harness)
    }

    fn spawn_daemon(root: &Path, bridge: &str) -> Result<Child> {
        let daemon_bin = Self::binary_path(DAEMON_MANIFEST, "hulld")?;

        Ok(Command::new(daemon_bin)
            .arg("--log-format")
            .arg("text")
            .arg("--log-file")
            .arg(root.join("hulld.log"))
            .env("HULL_PATH", root)
            .env("HULL_BRIDGE", bridge)
            .spawn()?)
    }

    fn wait_for_socket(socket_path: &Path) -> Result<()> {
        for _ in 0..50 {
            if socket_path.exists() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        Err(anyhow!(
            "daemon socket did not appear at '{}'",
            socket_path.display()
        ))
    }

    /// Run `hull` with the harness bridge.
    ///
    /// # Errors
    /// Returns an error if the `hull` process cannot be executed.
    pub fn run(&self, args: &[&str]) -> Result<std::process::Output> {
        self.run_with_bridge(&self.bridge, args)
    }

    /// Run `hull` with a custom bridge override.
    ///
    /// # Errors
    /// Returns an error if the `hull` process cannot be executed.
    pub fn run_with_bridge(&self, bridge: &str, args: &[&str]) -> Result<std::process::Output> {
        let hull_bin = Self::binary_path(CLIENT_MANIFEST, "hull")?;

        Ok(Command::new(hull_bin)
            .args(args)
            .env("HULL_PATH", &self.root)
            .env("HULL_BRIDGE", bridge)
            .output()?)
    }

    /// Stop the daemon.
    pub fn stop_daemon(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }

    /// Start the daemon with the same root and bridge settings.
    ///
    /// # Errors
    /// Returns an error if the daemon cannot be relaunched.
    pub fn start_daemon(&mut self) -> Result<()> {
        let socket_path = self.root.join("hulld.sock");
        let _ = fs::remove_file(&socket_path);

        self.daemon = Self::spawn_daemon(&self.root, &self.bridge)?;
        Self::wait_for_socket(&socket_path)?;
        Ok(())
    }

    /// Restart the daemon with the same root and bridge settings.
    ///
    /// # Errors
    /// Returns an error if the daemon cannot be relaunched.
    pub fn restart_daemon(&mut self) -> Result<()> {
        self.stop_daemon();
        self.start_daemon()
    }

    fn cleanup_orphans(&self) {
        let bridge = self.bridge.clone();
        let root = self.root.clone();

        if let Ok(ovs) = BridgeClient::connect() {
            let _ = ovs.del_bridge("hull0");
            let _ = ovs.del_bridge(&bridge);
            Self::cleanup_config_bridge(&ovs, &root.join("hull.json"));
        }

        let output = Command::new("ip")
            .args(["link", "show", &self.iface])
            .output();
        if let Ok(out) = output
            && out.status.success()
        {
            let _ = Command::new("ip")
                .args(["link", "delete", &self.iface])
                .status();
        }
    }

    fn cleanup_config_bridge(ovs: &BridgeClient, config_path: &Path) {
        let Ok(config) = Config::load(config_path) else {
            return;
        };

        let _ = ovs.del_bridge(&config.bridge_name);
    }
}

impl Drop for CliTestHarness {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
        self.cleanup_orphans();
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Assertion helpers for CLI command output.
pub trait HullOutputExt {
    /// Assert that the command exited successfully.
    fn assert_success(&self);
}

impl HullOutputExt for std::process::Output {
    fn assert_success(&self) {
        assert!(
            self.status.success(),
            "hull command failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr),
        );
    }
}
