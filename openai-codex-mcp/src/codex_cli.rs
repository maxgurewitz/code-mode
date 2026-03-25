use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::jwt;

const CODEX_AUTH_FILENAME: &str = "auth.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCredential {
    pub access: String,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub expires: u64,
}

#[derive(Debug, Deserialize)]
struct CodexCliAuthFile {
    tokens: Option<CodexCliTokens>,
    last_refresh: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexCliTokens {
    access_token: Option<String>,
    id_token: Option<String>,
    account_id: Option<String>,
}

pub fn resolve_codex_auth_path() -> PathBuf {
    resolve_codex_home().join(CODEX_AUTH_FILENAME)
}

pub fn read_required_codex_cli_credential() -> Result<CodexCredential> {
    let auth_path = resolve_codex_auth_path();
    match read_codex_cli_credential_at(&auth_path)? {
        Some(credential) => Ok(credential),
        None => bail!(
            "OpenAI Codex credentials were not found at {}. Run `codex login` and try again.",
            auth_path.display()
        ),
    }
}

pub fn read_codex_cli_credential_at(path: &Path) -> Result<Option<CodexCredential>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: CodexCliAuthFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(tokens) = parsed.tokens else {
        return Ok(None);
    };

    let Some(access) = tokens.access_token.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let fallback_expiry = fallback_expiry_ms(path, parsed.last_refresh.as_deref());
    let expires = jwt::extract_expiry_ms(&access).unwrap_or(fallback_expiry);
    let email = tokens
        .id_token
        .as_deref()
        .and_then(jwt::extract_email)
        .or_else(|| jwt::extract_email(&access));
    let account_id = tokens
        .account_id
        .or_else(|| jwt::extract_account_id(&access));

    Ok(Some(CodexCredential {
        access,
        account_id,
        email,
        expires,
    }))
}

pub fn credential_is_expired(credential: &CodexCredential) -> bool {
    credential.expires <= now_ms()
}

pub fn expired_credential_error() -> anyhow::Error {
    anyhow::anyhow!(
        "OpenAI Codex credentials appear to be expired. Run `codex login` and try again."
    )
}

fn resolve_codex_home() -> PathBuf {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

fn fallback_expiry_ms(path: &Path, last_refresh: Option<&str>) -> u64 {
    if let Some(last_refresh) = last_refresh {
        if let Ok(parsed) = time::OffsetDateTime::parse(
            last_refresh,
            &time::format_description::well_known::Rfc3339,
        ) {
            return (parsed.unix_timestamp_nanos() / 1_000_000) as u64
                + Duration::from_secs(3600).as_millis() as u64;
        }
    }

    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(to_unix_ms)
        .unwrap_or_else(now_ms)
        .saturating_add(Duration::from_secs(3600).as_millis() as u64)
}

fn to_unix_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn now_ms() -> u64 {
    to_unix_ms(SystemTime::now()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{read_codex_cli_credential_at, read_required_codex_cli_credential};

    #[test]
    fn reads_codex_cli_credentials_from_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        fs::write(
            &path,
            serde_json::json!({
                "tokens": {
                    "access_token": "eyJhbGciOiJub25lIn0.eyJleHAiOjEyMywiaHR0cHM6Ly9hcGkub3BlbmFpLmNvbS9hdXRoIjp7ImNoYXRncHRfYWNjb3VudF9pZCI6ImFjY3QifX0.",
                    "id_token": "eyJhbGciOiJub25lIn0.eyJlbWFpbCI6Im1lQGV4YW1wbGUuY29tIn0."
                }
            })
            .to_string(),
        )
        .expect("write");

        let credential = read_codex_cli_credential_at(&path)
            .expect("read")
            .expect("credential");

        assert_eq!(credential.account_id.as_deref(), Some("acct"));
        assert_eq!(credential.email.as_deref(), Some("me@example.com"));
        assert_eq!(credential.expires, 123_000);
    }

    #[test]
    fn missing_credentials_message_mentions_codex_login() {
        let previous = std::env::var("CODEX_HOME").ok();
        let dir = tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("CODEX_HOME", dir.path());
        }

        let error = read_required_codex_cli_credential().expect_err("should fail");
        assert!(error.to_string().contains("codex login"));

        if let Some(previous) = previous {
            unsafe {
                std::env::set_var("CODEX_HOME", previous);
            }
        } else {
            unsafe {
                std::env::remove_var("CODEX_HOME");
            }
        }
    }
}
