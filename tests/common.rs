use std::fs;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::sync::Arc;

use hull::config::Config;
use hull::database::Database;
use hull::interfaces::InterfaceOps;
use hull::routers::RouterOps;
use hull::switches::SwitchOps;

fn root_or_panic() {
    let uid = unsafe { libc::getuid() };
    if uid != 0 {
        panic!(
            "Flow snapshot tests must be run as root.\n\
             Re-run with: sudo env \"PATH=$PATH\" cargo test"
        );
    }
}

pub const SNAPSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots");

pub struct TestFlowsHarness {
    pub root: PathBuf,
    pub bridge: String,
    pub suffix: String,
    pub db: Arc<Database>,
    pub switch_ops: Arc<SwitchOps>,
    pub interface_ops: Arc<InterfaceOps>,
    pub router_ops: Arc<RouterOps>,
}

impl TestFlowsHarness {
    pub fn new(tag: &str) -> Self {
        root_or_panic();

        let suffix: String = (0..4)
            .map(|_| {
                let c = rand::random::<u8>() % 26;
                (b'a' + c) as char
            })
            .collect();

        let bridge = format!("ht-{}", suffix);
        let root = PathBuf::from(format!("/tmp/hull-test-{}-{}", tag, suffix));

        fs::create_dir_all(&root).expect("failed to create test root");

        let config = Config {
            bridge_name: bridge.clone(),
        };
        let config_path = root.join("hull.json");
        config.save(&config_path).expect("failed to save config");

        let db_path = root.join("hull.db");
        let db = Arc::new(Database::new(db_path));
        db.init().expect("failed to init database");

        let _ = Command::new("ovs-vsctl")
            .args(["--if-exists", "del-br", &bridge])
            .status();
        let status = Command::new("ovs-vsctl")
            .args(["add-br", &bridge])
            .status()
            .expect("failed to create OVS bridge");
        if !status.success() {
            panic!("failed to create OVS bridge '{}'", bridge);
        }

        let config_arc = Arc::new(config);
        let switch_ops = Arc::new(SwitchOps::new(db.clone(), config_arc.clone()));
        let interface_ops = Arc::new(InterfaceOps::new(db.clone()));
        let router_ops = Arc::new(RouterOps::new(db.clone(), config_arc.clone()));

        Self {
            root,
            bridge,
            suffix,
            db,
            switch_ops,
            interface_ops,
            router_ops,
        }
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

    pub fn normalize_flows(raw: &str) -> String {
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

            // Split on spaces first to get top-level groups, then split each group on commas
            let mut normalized_line = String::new();
            let groups: Vec<&str> = trimmed.split(' ').filter(|s| !s.is_empty()).collect();
            for group in groups {
                let fields: Vec<&str> = group.split(',').filter(|s| !s.is_empty()).collect();
                for field in fields {
                    let (key, value) = field.split_once('=').unwrap_or((field, ""));
                    // NOTE: The 'arp' or 'ip' fields don't have an '=' but we want to keep them
                    // as-is, so we treat them as having an empty value.
                    let norm_value = match key {
                        "cookie" => "<COOKIE>".to_string(),
                        "duration" => "<DURATION>".to_string(),
                        "n_packets" => "<N_PACKETS>".to_string(),
                        "n_bytes" => "<N_BYTES>".to_string(),
                        "idle_age" => "<IDLE_AGE>".to_string(),
                        "in_port" if value.chars().all(|c| c.is_ascii_digit()) => {
                            "<PORT>".to_string()
                        }
                        _ if Self::is_mac(value) => "<MAC>".to_string(),
                        _ => value.to_string(),
                    };

                    if normalized_line.is_empty() {
                        normalized_line.push_str(&format!("{}={}", key, norm_value));
                    } else {
                        normalized_line.push_str(&format!(", {}={}", key, norm_value));
                    }
                }
            }

            normalized.push(normalized_line);
        }

        normalized.join("\n")
    }

    pub fn diff(expected: &str, actual: &str) -> String {
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();
        let max_len = expected_lines.len().max(actual_lines.len());
        let mut out = String::new();

        for i in 0..max_len {
            let exp = expected_lines.get(i).map(|s| *s).unwrap_or("<missing>");
            let act = actual_lines.get(i).map(|s| *s).unwrap_or("<missing>");
            if exp != act {
                out.push_str(&format!(
                    "  line {}: expected: {:?}\n         actual:   {:?}\n",
                    i + 1,
                    exp,
                    act
                ));
            }
        }
        out
    }

    pub fn dump_flows(&self) -> String {
        let output = Command::new("ovs-ofctl")
            .args(["dump-flows", &self.bridge])
            .output()
            .expect("failed to run ovs-ofctl");
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("ovs-ofctl dump-flows failed:\n{}", stderr);
        }

        TestFlowsHarness::normalize_flows(&String::from_utf8_lossy(&output.stdout))
    }

    pub fn assert(&self, name: &str) {
        let snapshot_path = format!("{}/{}.txt", SNAPSHOT_DIR, name);
        let current = self.dump_flows();

        if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
            fs::write(&snapshot_path, &current).expect("failed to write snapshot");
            eprintln!("Snapshot written: {}", snapshot_path);
            return;
        }

        let expected = match fs::read_to_string(&snapshot_path) {
            Ok(s) => s,
            Err(_) => panic!(
                "Snapshot not found: {}\nRun with UPDATE_SNAPSHOTS=1 to generate.",
                snapshot_path
            ),
        };

        if current != expected {
            let diff = TestFlowsHarness::diff(&expected, &current);
            panic!("Flow snapshot mismatch for '{}':\n\n{}", name, diff);
        }
    }
}

impl Drop for TestFlowsHarness {
    fn drop(&mut self) {
        if let Ok(interfaces) = self.db.list_interfaces() {
            for iface in interfaces {
                let _ = hull::interfaces::Interface::delete(&iface.name);
            }
        }
        let _ = Command::new("ovs-vsctl")
            .args(["del-br", &self.bridge])
            .status();
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub struct CliTestHarness {
    pub root: PathBuf,
    pub bridge: String,
    pub iface: String,
    pub switch: String,
    pub router: String,
    pub port: String,
    daemon: Child,
}

impl CliTestHarness {
    pub fn new() -> Self {
        root_or_panic();
        let suffix: String = (0..4)
            .map(|_| {
                let c = rand::random::<u8>() % 26;
                (b'a' + c) as char
            })
            .collect();
        let bridge = format!("ht-{}", suffix);
        let root = PathBuf::from(format!("/tmp/hull-cli-{}", suffix));
        let _ = fs::remove_dir_all(&root);

        fs::create_dir_all(&root).expect("failed to create cli test root");

        let socket_path = root.join("hulld.sock");
        let daemon_bin = std::env::var("CARGO_BIN_EXE_hulld")
            .unwrap_or_else(|_| format!("{}/target/debug/hulld", env!("CARGO_MANIFEST_DIR")));
        let daemon = Command::new(daemon_bin)
            .arg("--log-format")
            .arg("text")
            .arg("--log-file")
            .arg(root.join("hulld.log"))
            .env("HULL_PATH", &root)
            .env("HULL_BRIDGE", &bridge)
            .spawn()
            .expect("failed to start hulld");

        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        Self {
            root,
            bridge,
            iface: format!("tap-{}", suffix),
            switch: format!("sw-{}", suffix),
            router: format!("rt-{}", suffix),
            port: format!("pt-{}", suffix),
            daemon,
        }
    }

    pub fn run(&self, args: &[&str]) -> std::process::Output {
        let hull_bin = std::env::var("CARGO_BIN_EXE_hull")
            .unwrap_or_else(|_| format!("{}/target/debug/hull", env!("CARGO_MANIFEST_DIR")));

        Command::new(hull_bin)
            .args(args)
            .env("HULL_PATH", &self.root)
            .env("HULL_BRIDGE", &self.bridge)
            .output()
            .expect("failed to run hull binary")
    }

    fn cleanup_orphans(&self) {
        let output = Command::new("ovs-vsctl")
            .arg("list-br")
            .output()
            .expect("failed to list bridges");
        let bridges = String::from_utf8_lossy(&output.stdout);
        if bridges.lines().any(|l| l == "hull0") {
            let _ = Command::new("ovs-vsctl").args(["del-br", "hull0"]).status();
        }
        if bridges.lines().any(|l| l == self.bridge) {
            let _ = Command::new("ovs-vsctl")
                .args(["del-br", &self.bridge])
                .status();
        }
        let output = Command::new("ip")
            .args(["link", "show", &self.iface])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let _ = Command::new("ip")
                    .args(["link", "delete", &self.iface])
                    .status();
            }
        }
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

pub trait HullOutputExt {
    fn assert_success(&self);
}

impl HullOutputExt for std::process::Output {
    fn assert_success(&self) {
        if !self.status.success() {
            panic!(
                "hull command failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&self.stdout),
                String::from_utf8_lossy(&self.stderr),
            );
        }
    }
}
