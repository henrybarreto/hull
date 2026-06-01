//! Shared test harness utilities.

#![allow(dead_code)]
#![allow(unreachable_pub)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::process::Command;

use anyhow::{Result, anyhow};

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
        "CLI integration tests must be run as root.\n\
         Re-run with: sudo env \"PATH=$PATH\" cargo test"
    );
}

const CLIENT_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/client/Cargo.toml");
const DAEMON_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/daemon/Cargo.toml");

/// Shared harness for CLI integration tests.
pub struct CliTestHarness {
    /// Temporary root directory for the harness.
    pub root: PathBuf,
    /// Bridge name under test.
    pub bridge: String,
    /// Temporary switch port/TAP name.
    pub port: String,
    /// Temporary switch name.
    pub switch: String,
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
            port: format!("swp-{suffix}"),
            switch: format!("sw-{suffix}"),
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
        let output = Command::new("ip")
            .args(["link", "show", &self.port])
            .output();
        if let Ok(out) = output
            && out.status.success()
        {
            let _ = Command::new("ip")
                .args(["link", "delete", &self.port])
                .status();
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
