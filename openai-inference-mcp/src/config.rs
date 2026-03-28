use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use figment::{
    Figment,
    providers::{Env, Serialized},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

const CONFIG_ENV_PREFIX: &str = "OPENAI_INFERENCE_MCP_";
const DEFAULT_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_USER_AGENT: &str = "openai-inference-mcp/0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub base_url: String,
    pub timeout_ms: u64,
    pub user_agent: String,
    pub api_key: Option<String>,
    pub log: LogFilter,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            timeout_ms: 120_000,
            user_agent: DEFAULT_USER_AGENT.into(),
            api_key: None,
            log: LogFilter::default(),
        }
    }
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        let config_root = dirs::config_dir().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        });
        Ok(config_root.join("openai-inference-mcp").join("config.toml"))
    }

    pub fn endpoint_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    pub fn http_client(&self) -> Result<Client> {
        Client::builder()
            .user_agent(self.user_agent.clone())
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .build()
            .context("failed to build HTTP client")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct LogFilter(String);

impl Default for LogFilter {
    fn default() -> Self {
        Self("error".into())
    }
}

impl LogFilter {
    pub fn to_tracing_env_filter(&self) -> Result<EnvFilter> {
        EnvFilter::builder()
            .with_default_directive(LevelFilter::ERROR.into())
            .parse(&self.0)
            .with_context(|| format!("invalid log filter {:?}", self.0))
    }
}

pub fn load_config(path: Option<&Path>) -> Result<Config> {
    let mut config = Config::default();
    let config_path = if let Some(path) = path {
        Some(path.to_path_buf())
    } else {
        Some(Config::config_path()?)
    };
    if let Some(config_path) = config_path.filter(|path| path.is_file()) {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        config = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", config_path.display()))?;
    }

    Figment::from(Serialized::defaults(config))
        .merge(Env::prefixed(CONFIG_ENV_PREFIX))
        .extract::<Config>()
        .context("failed to load openai-inference-mcp config")
}
