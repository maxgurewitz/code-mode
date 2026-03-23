use std::collections::HashMap;
use std::path::Path;
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

fn empty_config() -> Config {
    Config {
        base_dir: None,
        servers: HashMap::new(),
    }
}

fn find_local_config_path(start_dir: &Path) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .map(|dir| dir.join("code-mode.toml"))
        .find(|path| path.is_file())
}

fn load_default_config(start_dir: &Path, global_path: &Path) -> Result<Config> {
    let mut config = empty_config();
    load_toml(global_path, &mut config)?;
    if let Some(local_path) = find_local_config_path(start_dir) {
        load_toml(&local_path, &mut config)?;
    }
    Ok(config)
}

fn is_var_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_var_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn interpolate_string(input: &str, variables: &HashMap<String, String>) -> String {
    let mut output = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        if ch != '$' {
            output.push(ch);
            index += 1;
            continue;
        }

        let Some(next) = chars.get(index + 1).copied() else {
            output.push('$');
            index += 1;
            continue;
        };

        if next == '$' {
            output.push('$');
            index += 2;
            continue;
        }

        if next == '{' {
            let mut cursor = index + 2;
            let mut name = String::new();
            while cursor < chars.len() && chars[cursor] != '}' {
                name.push(chars[cursor]);
                cursor += 1;
            }

            if cursor < chars.len()
                && !name.is_empty()
                && is_var_start(name.chars().next().unwrap())
                && name.chars().all(is_var_continue)
            {
                output.push_str(variables.get(&name).map(String::as_str).unwrap_or_default());
                index = cursor + 1;
                continue;
            }

            output.push('$');
            output.push('{');
            output.push_str(&name);
            if cursor < chars.len() {
                output.push('}');
                index = cursor + 1;
            } else {
                index = cursor;
            }
            continue;
        }

        if is_var_start(next) {
            let mut cursor = index + 1;
            let mut name = String::new();
            while cursor < chars.len() && is_var_continue(chars[cursor]) {
                name.push(chars[cursor]);
                cursor += 1;
            }

            output.push_str(variables.get(&name).map(String::as_str).unwrap_or_default());
            index = cursor;
            continue;
        }

        output.push('$');
        index += 1;
    }

    output
}

fn string_needs_interpolation(input: &str) -> bool {
    input.contains('$')
}

fn config_needs_interpolation(config: &Config) -> bool {
    config
        .base_dir
        .as_ref()
        .map(|path| string_needs_interpolation(&path.to_string_lossy()))
        .unwrap_or(false)
        || config.servers.values().any(|entry| {
            entry
                .command
                .as_deref()
                .map(string_needs_interpolation)
                .unwrap_or(false)
                || entry.args.iter().any(|arg| string_needs_interpolation(arg))
                || entry
                    .env
                    .values()
                    .any(|value| string_needs_interpolation(value))
                || entry
                    .url
                    .as_deref()
                    .map(string_needs_interpolation)
                    .unwrap_or(false)
                || entry
                    .headers
                    .values()
                    .any(|value| string_needs_interpolation(value))
        })
}

fn interpolate_config(config: &mut Config, variables: &HashMap<String, String>) {
    if let Some(base_dir) = &config.base_dir {
        config.base_dir = Some(PathBuf::from(interpolate_string(
            &base_dir.to_string_lossy(),
            variables,
        )));
    }

    for entry in config.servers.values_mut() {
        entry.command = entry
            .command
            .as_ref()
            .map(|command| interpolate_string(command, variables));
        entry.args = entry
            .args
            .iter()
            .map(|arg| interpolate_string(arg, variables))
            .collect();
        entry.env = entry
            .env
            .iter()
            .map(|(key, value)| (key.clone(), interpolate_string(value, variables)))
            .collect();
        entry.url = entry
            .url
            .as_ref()
            .map(|url| interpolate_string(url, variables));
        entry.headers = entry
            .headers
            .iter()
            .map(|(key, value)| (key.clone(), interpolate_string(value, variables)))
            .collect();
    }
}

/// Load and merge config. When `config_path` is provided, only that file is
/// used. Otherwise merges `~/.config/code-mode/code-mode.toml` (global) with
/// the nearest ancestor `code-mode.toml` from the current directory, with the
/// local config overriding global settings.
pub fn load_config(config_path: Option<&Path>) -> Result<Config> {
    let mut config = if let Some(path) = config_path {
        anyhow::ensure!(path.exists(), "config file not found: {}", path.display());
        let mut config = empty_config();
        load_toml(path, &mut config)?;
        config
    } else {
        let start_dir = std::env::current_dir().context("failed to determine current directory")?;
        load_default_config(&start_dir, &config_dir()?.join("code-mode.toml"))?
    };

    if config_needs_interpolation(&config) {
        let variables: HashMap<String, String> = std::env::vars().collect();
        interpolate_config(&mut config, &variables);
    }

    for (name, entry) in &config.servers {
        validate_entry(name, entry)?;
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::{
        Config, ServerEntry, config_needs_interpolation, find_local_config_path,
        interpolate_config, interpolate_string, load_default_config,
    };
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_entry() -> ServerEntry {
        ServerEntry {
            transport: "stdio".into(),
            command: Some("$CMD".into()),
            args: vec![
                "$ARG".into(),
                "${BRACED}".into(),
                "$$DOLLAR".into(),
                "prefix-$ARG-suffix".into(),
                "$(not-a-var)".into(),
            ],
            env: HashMap::from([
                ("FIRST".into(), "$ENV_ONE".into()),
                ("SECOND".into(), "${ENV_TWO}".into()),
            ]),
            url: Some("https://example.com/$ARG".into()),
            headers: HashMap::from([("Authorization".into(), "Bearer $TOKEN".into())]),
        }
    }

    #[test]
    fn interpolates_supported_env_variable_patterns() {
        let variables = HashMap::from([
            ("NAME".into(), "world".into()),
            ("PATH_SEGMENT".into(), "bin".into()),
        ]);

        assert_eq!(
            interpolate_string("hello $NAME/${PATH_SEGMENT}", &variables),
            "hello world/bin"
        );
    }

    #[test]
    fn leaves_unknown_or_unsupported_patterns_safe() {
        let variables = HashMap::from([("KNOWN".into(), "value".into())]);

        assert_eq!(interpolate_string("$$KNOWN", &variables), "$KNOWN");
        assert_eq!(interpolate_string("$(date)", &variables), "$(date)");
        assert_eq!(
            interpolate_string("${NOT-VALID}", &variables),
            "${NOT-VALID}"
        );
        assert_eq!(interpolate_string("$UNKNOWN", &variables), "");
    }

    #[test]
    fn interpolates_config_fields_used_by_mcp_servers() {
        let mut config = Config {
            base_dir: Some(PathBuf::from("$BASE_DIR")),
            servers: HashMap::from([("fixture".into(), test_entry())]),
        };

        let variables = HashMap::from([
            ("BASE_DIR".into(), ".tmp/.code-mode".into()),
            ("CMD".into(), "node".into()),
            ("ARG".into(), "script.mjs".into()),
            ("BRACED".into(), "--flag".into()),
            ("ENV_ONE".into(), "secret".into()),
            ("ENV_TWO".into(), "another".into()),
            ("TOKEN".into(), "abc123".into()),
        ]);

        assert!(config_needs_interpolation(&config));
        interpolate_config(&mut config, &variables);

        assert_eq!(config.base_dir, Some(PathBuf::from(".tmp/.code-mode")));
        let entry = config.servers.get("fixture").unwrap();
        assert_eq!(entry.command.as_deref(), Some("node"));
        assert_eq!(
            entry.args,
            vec![
                "script.mjs",
                "--flag",
                "$DOLLAR",
                "prefix-script.mjs-suffix",
                "$(not-a-var)",
            ]
        );
        assert_eq!(entry.env.get("FIRST").map(String::as_str), Some("secret"));
        assert_eq!(entry.env.get("SECOND").map(String::as_str), Some("another"));
        assert_eq!(
            entry.headers.get("Authorization").map(String::as_str),
            Some("Bearer abc123")
        );
        assert_eq!(entry.url.as_deref(), Some("https://example.com/script.mjs"));
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "code-mode-config-tests-{name}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_nearest_local_config_in_ancestor_directory() {
        let temp_dir = unique_temp_dir("find-local-config");
        let workspace_dir = temp_dir.join("workspace");
        let crate_dir = workspace_dir.join("crate");
        let nested_dir = crate_dir.join("src/bin");

        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(workspace_dir.join("code-mode.toml"), "").unwrap();
        fs::write(crate_dir.join("code-mode.toml"), "").unwrap();

        let discovered = find_local_config_path(&nested_dir);
        assert_eq!(discovered, Some(crate_dir.join("code-mode.toml")));

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn load_default_config_merges_global_and_nearest_local_config() {
        let temp_dir = unique_temp_dir("load-default-config");
        let config_home = temp_dir.join("xdg/code-mode");
        let workspace_dir = temp_dir.join("workspace");
        let nested_dir = workspace_dir.join(".workspaces/task");

        fs::create_dir_all(&config_home).unwrap();
        fs::create_dir_all(&nested_dir).unwrap();

        fs::write(
            config_home.join("code-mode.toml"),
            r#"
base_dir = "global-sdk"

[servers.global]
command = "global-command"
"#,
        )
        .unwrap();
        fs::write(
            workspace_dir.join("code-mode.toml"),
            r#"
base_dir = "local-sdk"

[servers.local]
command = "local-command"
"#,
        )
        .unwrap();

        let config = load_default_config(&nested_dir, &config_home.join("code-mode.toml")).unwrap();

        assert_eq!(config.base_dir, Some(PathBuf::from("local-sdk")));
        assert!(config.servers.contains_key("global"));
        assert!(config.servers.contains_key("local"));

        fs::remove_dir_all(temp_dir).unwrap();
    }
}
