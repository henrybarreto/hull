use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub bridge_name: String,
}

impl Config {
    /// Load configuration from a JSON file, falling back to defaults.
    pub fn load(path: &Path) -> Result<Self> {
        let cfg = if !path.exists() {
            Self::default()
        } else {
            let content = fs::read_to_string(path)
                .with_context(|| format!("failed to read config: {}", path.display()))?;
            serde_json::from_str(&content).context("failed to parse config JSON")?
        };

        Ok(cfg)
    }

    /// Persist configuration to a JSON file.
    pub fn save(&self, path: &Path) -> Result<()> {
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
pub fn get_root_path() -> Result<PathBuf> {
    let is_root = unsafe { libc::getuid() == 0 };
    let home = std::env::var("HOME").map(PathBuf::from).ok();

    let root_dir = if let Ok(dir) = std::env::var("HULL_PATH") {
        PathBuf::from(dir)
    } else if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(dir).join("hull")
    } else if is_root {
        PathBuf::from("/var/lib/hull")
    } else {
        home.as_ref()
            .map(|h| h.join(".local/share/hull"))
            .ok_or_else(|| anyhow!("failed to resolve home directory"))?
    };
    Ok(root_dir)
}

/// Resolve the config file path.
pub fn get_config_path(root: &Path, override_path: Option<PathBuf>) -> PathBuf {
    override_path.unwrap_or_else(|| root.join("hull.json"))
}

/// Resolve the database file path.
pub fn get_db_path(root: &Path) -> PathBuf {
    root.join("hull.db")
}
