use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub base_dir: Option<PathBuf>,
    #[serde(default)]
    pub servers: HashMap<String, ServerEntry>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerEntry {
    #[serde(default = "default_transport")]
    pub transport: String,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_transport() -> String {
    "stdio".into()
}

/// Resolves the global config directory: `$XDG_CONFIG_HOME/code-mode/`
/// (default `~/.config/code-mode/`).
pub fn config_dir() -> Result<PathBuf> {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = dirs::home_dir().expect("could not determine home directory");
            home.join(".config")
        });
    Ok(config_home.join("code-mode"))
}

/// Validates that each server entry has the fields required by its transport type.
fn validate_entry(name: &str, entry: &ServerEntry) -> Result<()> {
    match entry.transport.as_str() {
        "stdio" => {
            anyhow::ensure!(
                entry.command.is_some(),
                "server \"{name}\": stdio transport requires a \"command\" field"
            );
            anyhow::ensure!(
                entry.url.is_none(),
                "server \"{name}\": stdio transport should not have a \"url\" field"
            );
        }
        "http" | "sse" => {
            anyhow::ensure!(
                entry.url.is_some(),
                "server \"{name}\": {transport} transport requires a \"url\" field",
                transport = entry.transport
            );
            anyhow::ensure!(
                entry.command.is_none(),
                "server \"{name}\": {transport} transport should not have a \"command\" field",
                transport = entry.transport
            );
        }
        other => {
            anyhow::bail!(
                "server \"{name}\": unknown transport type \"{other}\" \
                 (expected \"stdio\", \"http\", or \"sse\")"
            );
        }
    }
    Ok(())
}

use std::path::Path;

fn load_toml(path: &Path, config: &mut Config) -> Result<()> {
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let cfg: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if cfg.base_dir.is_some() {
            config.base_dir = cfg.base_dir;
        }
        config.servers.extend(cfg.servers);
    }
    Ok(())
}

/// Load and merge config. When `config_path` is provided, only that file is
/// used. Otherwise merges `~/.config/code-mode/code-mode.toml` (global) with
/// `./code-mode.toml` (local), local overrides global.
pub fn load_config(config_path: Option<&Path>) -> Result<Config> {
    let mut config = Config {
        base_dir: None,
        servers: HashMap::new(),
    };

    if let Some(path) = config_path {
        anyhow::ensure!(path.exists(), "config file not found: {}", path.display());
        load_toml(path, &mut config)?;
    } else {
        // Home config (low priority)
        load_toml(&config_dir()?.join("code-mode.toml"), &mut config)?;
        // Local config (high priority — overwrites home)
        let local_path = std::env::current_dir()
            .context("failed to determine current directory")?
            .join("code-mode.toml");
        load_toml(&local_path, &mut config)?;
    }

    for (name, entry) in &config.servers {
        validate_entry(name, entry)?;
    }

    Ok(config)
}
