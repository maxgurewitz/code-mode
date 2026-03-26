use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use figment::{
    Figment,
    providers::{Env, Serialized},
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

const CONFIG_ENV_PREFIX: &str = "OPENAI_CODEX_MCP_";
const DEFAULT_OAUTH_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_API_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_OAUTH_MODEL: &str = "gpt-5.4-mini";
const DEFAULT_API_MODEL: &str = "gpt-5-mini";
const DEFAULT_USER_AGENT: &str = "openai-codex-mcp/0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    #[default]
    OAuth,
    ApiToken,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OAuth => "oauth",
            Self::ApiToken => "api_token",
        }
    }
}

impl std::fmt::Display for AuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for AuthMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "oauth" => Ok(Self::OAuth),
            "api_token" | "token" => Ok(Self::ApiToken),
            _ => Err("expected `oauth` or `api_token`".into()),
        }
    }
}

impl Serialize for AuthMode {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AuthMode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub base_url: String,
    pub model: String,
    pub timeout_ms: u64,
    pub user_agent: String,
    pub auth_mode: AuthMode,
    pub api_token: Option<String>,
    pub log: LogFilter,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_OAUTH_BASE_URL.into(),
            model: DEFAULT_OAUTH_MODEL.into(),
            timeout_ms: 120_000,
            user_agent: DEFAULT_USER_AGENT.into(),
            auth_mode: AuthMode::default(),
            api_token: None,
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
        Ok(config_root.join("openai-codex-mcp").join("config.toml"))
    }

    pub fn responses_url(&self) -> String {
        let base_url = self.effective_base_url();
        let path = match self.auth_mode {
            AuthMode::OAuth => "/codex/responses",
            AuthMode::ApiToken => "/v1/responses",
        };
        format!("{}{}", base_url.trim_end_matches('/'), path)
    }

    pub fn model_name(&self) -> &str {
        if matches!(self.auth_mode, AuthMode::ApiToken) && self.model == DEFAULT_OAUTH_MODEL {
            DEFAULT_API_MODEL
        } else {
            &self.model
        }
    }

    pub fn http_client(&self) -> Result<Client> {
        Client::builder()
            .user_agent(self.user_agent.clone())
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .build()
            .context("failed to build HTTP client")
    }

    fn effective_base_url(&self) -> &str {
        if matches!(self.auth_mode, AuthMode::ApiToken) && self.base_url == DEFAULT_OAUTH_BASE_URL {
            DEFAULT_API_BASE_URL
        } else {
            &self.base_url
        }
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
        .context("failed to load openai-codex-mcp config")
}

#[cfg(test)]
mod tests {
    use super::{AuthMode, Config};

    #[test]
    fn api_token_mode_uses_api_defaults() {
        let mut config = Config::default();
        config.auth_mode = AuthMode::ApiToken;

        assert_eq!(
            config.responses_url(),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(config.model_name(), "gpt-5-mini");
    }

    #[test]
    fn explicit_api_token_base_url_and_model_are_preserved() {
        let mut config = Config::default();
        config.auth_mode = AuthMode::ApiToken;
        config.base_url = "http://127.0.0.1:1234".into();
        config.model = "gpt-5-nano".into();

        assert_eq!(config.responses_url(), "http://127.0.0.1:1234/v1/responses");
        assert_eq!(config.model_name(), "gpt-5-nano");
    }
}
