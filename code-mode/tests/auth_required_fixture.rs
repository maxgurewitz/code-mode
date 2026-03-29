use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;
use tempfile::tempdir;
use tokio::{fs, process::Command};

#[tokio::test]
async fn generated_sdk_emits_auth_required_event_and_exits_77() -> Result<()> {
    let temp = tempdir()?;
    let config_path = temp.path().join("code-mode.toml");
    let fixture_server = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("auth-required-server.mjs");

    let toml = format!(
        r#"
base_dir = ".tmp/.code-mode"

[servers.github]
command = "node"
args = [{fixture_server:?}]
"#,
        fixture_server = fixture_server.display().to_string(),
    );
    fs::write(&config_path, toml).await?;

    let generate = Command::new(env!("CARGO_BIN_EXE_code-mode"))
        .arg("mcp")
        .arg("generate")
        .arg("--config")
        .arg(&config_path)
        .current_dir(temp.path())
        .output()
        .await
        .context("failed to run code-mode generate")?;

    if !generate.status.success() {
        return Err(anyhow::anyhow!(
            "code-mode generate failed: {}",
            String::from_utf8_lossy(&generate.stderr)
        ));
    }

    fs::write(
        temp.path().join("test-auth-required.ts"),
        r#"import { github } from "./.tmp/.code-mode/sdk/index.js";

await github.createIssue({ title: "Auth-required fixture" });
process.exit(99);
"#,
    )
    .await?;

    let code_mode_bin = PathBuf::from(env!("CARGO_BIN_EXE_code-mode"));
    let code_mode_dir = code_mode_bin.parent().context("code-mode binary parent")?;
    let path_separator = if cfg!(windows) { ";" } else { ":" };
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let path_env = if inherited_path.is_empty() {
        code_mode_dir.display().to_string()
    } else {
        format!("{}{}{}", code_mode_dir.display(), path_separator, inherited_path)
    };

    let output = Command::new("bun")
        .arg("run")
        .arg("test-auth-required.ts")
        .env("PATH", path_env)
        .current_dir(temp.path())
        .output()
        .await
        .context("failed to run generated auth fixture script")?;

    assert_eq!(output.status.code(), Some(77));

    let stderr = String::from_utf8(output.stderr)?;
    let auth_line = stderr
        .lines()
        .find(|line| line.contains(r#""type":"auth_required""#))
        .context("missing auth_required event in stderr")?;
    let event: Value = serde_json::from_str(auth_line)?;

    assert_eq!(event["type"], "auth_required");
    assert_eq!(event["service"], "github");
    assert_eq!(event["server"], "github");
    assert_eq!(event["tool"], "create_issue");
    assert_eq!(event["reason"], "missing_connection");
    assert_eq!(event["message"], "Connect GitHub to continue");
    assert_eq!(event["url"], "https://example.test/connect/github");
    assert_eq!(event["retryable"], true);

    Ok(())
}
