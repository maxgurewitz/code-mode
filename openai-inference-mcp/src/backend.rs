use anyhow::{Context, Result, bail};
use reqwest::{StatusCode, header};
use serde_json::Value;

use crate::{
    codex_cli,
    config::{AuthMode, Config},
};

const RESPONSES_PATH: &str = "/v1/responses";
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const EMBEDDINGS_PATH: &str = "/v1/embeddings";

#[derive(Debug, Clone)]
pub struct OpenAIBackend {
    config: Config,
    client: reqwest::Client,
}

impl OpenAIBackend {
    pub fn new(config: Config) -> Result<Self> {
        let client = config.http_client()?;
        Ok(Self { config, client })
    }

    pub async fn create_response(&self, body: Value) -> Result<Value> {
        self.post_json(RESPONSES_PATH, "responses", body).await
    }

    pub async fn create_chat_completion(&self, body: Value) -> Result<Value> {
        self.post_json(CHAT_COMPLETIONS_PATH, "chat.completions", body)
            .await
    }

    pub async fn create_embedding(&self, body: Value) -> Result<Value> {
        self.post_json(EMBEDDINGS_PATH, "embeddings", body).await
    }

    async fn post_json(&self, path: &str, api_name: &str, body: Value) -> Result<Value> {
        validate_body(api_name, &body)?;
        let auth = resolve_auth(&self.config)?;

        let request = self
            .client
            .post(self.config.endpoint_url(path))
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", auth.access_token()),
            )
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .json(&body);
        let response = auth
            .decorate_request(request)
            .send()
            .await
            .with_context(|| format!("failed to call OpenAI {api_name} API"))?;

        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(auth.unauthorized_error());
        }
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("OpenAI {api_name} API returned {status}: {body}");
        }

        response
            .json()
            .await
            .with_context(|| format!("failed to parse JSON from OpenAI {api_name} API"))
    }
}

pub fn validate_auth_configuration(config: &Config) -> Result<()> {
    resolve_auth(config).map(|_| ())
}

fn validate_body(api_name: &str, body: &Value) -> Result<()> {
    if !body.is_object() {
        bail!("`body` for {api_name} must be a JSON object");
    }
    if body.get("stream").and_then(Value::as_bool) == Some(true) {
        bail!("`stream: true` is not supported for {api_name} via MCP");
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum ResolvedAuth {
    OAuth(codex_cli::CodexCredential),
    ApiKey(String),
}

impl ResolvedAuth {
    fn access_token(&self) -> &str {
        match self {
            Self::OAuth(credential) => &credential.access,
            Self::ApiKey(api_key) => api_key,
        }
    }

    fn decorate_request(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::OAuth(credential) => {
                if let Some(account_id) = credential.account_id.as_deref() {
                    request.header("ChatGPT-Account-Id", account_id)
                } else {
                    request
                }
            }
            Self::ApiKey(_) => request,
        }
    }

    fn unauthorized_error(&self) -> anyhow::Error {
        match self {
            Self::OAuth(_) => codex_cli::expired_credential_error(),
            Self::ApiKey(_) => anyhow::anyhow!(
                "OpenAI API key was rejected. Check OPENAI_INFERENCE_MCP_API_KEY or OPENAI_API_KEY and try again."
            ),
        }
    }
}

fn resolve_auth(config: &Config) -> Result<ResolvedAuth> {
    match config.auth_mode {
        AuthMode::OAuth => {
            let credential = codex_cli::read_required_codex_cli_credential()?;
            if codex_cli::credential_is_expired(&credential) {
                return Err(codex_cli::expired_credential_error());
            }
            Ok(ResolvedAuth::OAuth(credential))
        }
        AuthMode::ApiKey => {
            if let Some(api_key) = config
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Ok(ResolvedAuth::ApiKey(api_key.to_owned()));
            }

            if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
                let api_key = api_key.trim();
                if !api_key.is_empty() {
                    return Ok(ResolvedAuth::ApiKey(api_key.to_owned()));
                }
            }

            bail!("OPENAI_INFERENCE_MCP_API_KEY or OPENAI_API_KEY must be set")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use anyhow::{Context, Result};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};
    use tempfile::tempdir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use super::OpenAIBackend;
    use crate::config::{AuthMode, Config};

    #[tokio::test]
    async fn responses_calls_openai_responses_endpoint() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let request = read_request(&mut stream).await.expect("request");
            assert_eq!(request.path, "/v1/responses");
            assert_eq!(
                request.headers.get("authorization").map(String::as_str),
                Some("Bearer test_key")
            );
            write_json_response(
                &mut stream,
                &json!({ "id": "resp_123", "output_text": "hi" }),
            )
            .await
            .expect("response");
        });

        let mut config = Config::default();
        config.base_url = format!("http://{addr}");
        config.api_key = Some("test_key".into());

        let backend = OpenAIBackend::new(config)?;
        let response = backend
            .create_response(json!({ "model": "gpt-5-mini", "input": "hello" }))
            .await?;
        assert_eq!(response["output_text"], json!("hi"));

        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn oauth_mode_reads_codex_cli_credentials_and_sets_account_header() -> Result<()> {
        let access = make_access_token("acct_123", 9_999_999_999);
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let access_for_server = access.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let request = read_request(&mut stream).await.expect("request");
            assert_eq!(request.path, "/v1/responses");
            assert_eq!(
                request.headers.get("authorization").map(String::as_str),
                Some(format!("Bearer {access_for_server}").as_str())
            );
            assert_eq!(
                request
                    .headers
                    .get("chatgpt-account-id")
                    .map(String::as_str),
                Some("acct_123")
            );
            write_json_response(
                &mut stream,
                &json!({ "id": "resp_oauth", "output_text": "hi from oauth" }),
            )
            .await
            .expect("response");
        });

        let codex_home = tempdir()?;
        seed_codex_home(codex_home.path(), &access, "acct_123", "person@example.com")?;
        let previous = std::env::var("CODEX_HOME").ok();
        unsafe {
            std::env::set_var("CODEX_HOME", codex_home.path());
        }

        let mut config = Config::default();
        config.base_url = format!("http://{addr}");
        config.auth_mode = AuthMode::OAuth;

        let backend = OpenAIBackend::new(config)?;
        let response = backend
            .create_response(json!({ "model": "gpt-5-mini", "input": "hello" }))
            .await?;
        assert_eq!(response["output_text"], json!("hi from oauth"));

        server.await?;
        restore_env_var("CODEX_HOME", previous);
        Ok(())
    }

    #[tokio::test]
    async fn chat_completions_calls_openai_chat_endpoint() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let request = read_request(&mut stream).await.expect("request");
            assert_eq!(request.path, "/v1/chat/completions");
            write_json_response(
                &mut stream,
                &json!({
                    "id": "chatcmpl_123",
                    "choices": [{ "message": { "content": "hi" } }]
                }),
            )
            .await
            .expect("response");
        });

        let mut config = Config::default();
        config.base_url = format!("http://{addr}");
        config.api_key = Some("test_key".into());

        let backend = OpenAIBackend::new(config)?;
        let response = backend
            .create_chat_completion(json!({
                "model": "gpt-5-mini",
                "messages": [{ "role": "user", "content": "hello" }]
            }))
            .await?;
        assert_eq!(response["choices"][0]["message"]["content"], json!("hi"));

        server.await?;
        Ok(())
    }

    #[tokio::test]
    async fn rejects_streaming_requests() -> Result<()> {
        let mut config = Config::default();
        config.api_key = Some("test_key".into());
        let backend = OpenAIBackend::new(config)?;

        let error = backend
            .create_response(json!({
                "model": "gpt-5-mini",
                "input": "hello",
                "stream": true
            }))
            .await
            .expect_err("streaming request should fail");

        assert!(error.to_string().contains("stream: true"));
        Ok(())
    }

    #[derive(Debug)]
    struct ObservedRequest {
        path: String,
        headers: HashMap<String, String>,
    }

    async fn read_request(stream: &mut TcpStream) -> Result<ObservedRequest> {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 1024];
        let mut header_end = None;
        while header_end.is_none() {
            let read = stream.read(&mut temp).await?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&temp[..read]);
            header_end = buffer.windows(4).position(|window| window == b"\r\n\r\n");
        }

        let header_end = header_end.context("missing request headers")?;
        let header_bytes = &buffer[..header_end];
        let text = std::str::from_utf8(header_bytes).context("request was not utf-8")?;
        let mut lines = text.split("\r\n");

        let request_line = lines.next().context("missing request line")?;
        let mut request_parts = request_line.split_whitespace();
        let _method = request_parts.next().context("missing method")?;
        let path = request_parts.next().context("missing path")?.to_owned();

        let headers = lines
            .filter_map(|line: &str| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            })
            .collect();

        Ok(ObservedRequest { path, headers })
    }

    async fn write_json_response(stream: &mut TcpStream, body: &Value) -> Result<()> {
        let body = serde_json::to_vec(body)?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).await?;
        stream.write_all(&body).await?;
        Ok(())
    }

    fn seed_codex_home(
        dir: &std::path::Path,
        access: &str,
        account_id: &str,
        email: &str,
    ) -> Result<()> {
        fs::write(
            dir.join("auth.json"),
            serde_json::json!({
                "auth_mode": "chatgpt",
                "last_refresh": "2026-03-24T08:56:59.779225Z",
                "tokens": {
                    "access_token": access,
                    "id_token": make_id_token(email),
                    "account_id": account_id,
                }
            })
            .to_string(),
        )?;
        Ok(())
    }

    fn restore_env_var(name: &str, previous: Option<String>) {
        if let Some(previous) = previous {
            unsafe {
                std::env::set_var(name, previous);
            }
        } else {
            unsafe {
                std::env::remove_var(name);
            }
        }
    }

    fn make_access_token(account_id: &str, exp: u64) -> String {
        make_token(json!({
            "exp": exp,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id
            }
        }))
    }

    fn make_id_token(email: &str) -> String {
        make_token(json!({ "email": email }))
    }

    fn make_token(payload: Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{payload}.")
    }
}
