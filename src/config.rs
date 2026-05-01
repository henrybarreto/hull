use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::trace;

/// Loaded Hull configuration.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    /// Name of the OVS bridge managed by Hull.
    pub bridge_name: String,
}

impl Config {
    /// Load configuration from a JSON file, falling back to defaults.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read or the JSON is invalid.
    pub fn load(path: &Path) -> Result<Self> {
        trace!(path = %path.display(), "loading config");
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let cfg = serde_json::from_str(&content).context("failed to parse config JSON")?;
        Ok(cfg)
    }

    /// Persist configuration to a JSON file.
    ///
    /// # Errors
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        trace!(path = %path.display(), bridge_name = %self.bridge_name, "saving config");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bridge_name: std::env::var("HULL_BRIDGE").unwrap_or_else(|_| "hull0".to_string()),
        }
    }
}

/// Resolve the root data path for hull.
pub fn get_root_path() -> PathBuf {
    let path = std::env::var("HULL_PATH").map_or_else(
        |_| {
            std::env::var("XDG_DATA_HOME").map_or_else(
                |_| PathBuf::from("/var/lib/hull"),
                |dir| PathBuf::from(dir).join("hull"),
            )
        },
        PathBuf::from,
    );
    trace!(path = %path.display(), "resolved root path");
    path
}

/// Resolve the config file path.
pub fn get_config_path(root: &Path, override_path: Option<PathBuf>) -> PathBuf {
    let path = override_path.unwrap_or_else(|| root.join("hull.json"));
    trace!(path = %path.display(), "resolved config path");
    path
}

/// Resolve the database file path.
pub fn get_db_path(root: &Path) -> PathBuf {
    let path = root.join("hull.db");
    trace!(path = %path.display(), "resolved database path");
    path
}

/// Resolve the daemon socket path.
pub fn get_socket_path(root: &Path) -> PathBuf {
    let path = std::env::var("HULL_SOCKET").map_or_else(|_| root.join("hulld.sock"), PathBuf::from);
    trace!(path = %path.display(), "resolved socket path");
    path
}
