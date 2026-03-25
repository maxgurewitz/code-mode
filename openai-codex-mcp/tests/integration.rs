use std::{collections::HashMap, fs};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use openai_codex_mcp::{
    backend::{CodexBackend, CodexInferRequest},
    config::Config,
};
use rmcp::{
    RoleClient, ServiceExt,
    model::CallToolRequestParams,
    service::RunningService,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

#[tokio::test]
async fn backend_reads_codex_cli_credentials_and_calls_backend() -> Result<()> {
    let access = make_access_token("acct_123", 9_999_999_999);
    let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let requests_for_server = std::sync::Arc::clone(&requests);
    let access_for_server = access.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let request = read_request(&mut stream).await.expect("request");
        requests_for_server.lock().await.push(request);
        write_response(
            &mut stream,
            "200 OK",
            "text/event-stream",
            &sse_body("Hello from Codex auth"),
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
    config.timeout_ms = 5_000;

    let backend = CodexBackend::new(config)?;
    let result = backend
        .infer(CodexInferRequest {
            prompt: "say hi".into(),
            model: None,
            instructions: None,
            reasoning_effort: None,
        })
        .await?;

    assert_eq!(result.text, "Hello from Codex auth");
    let seen = requests.lock().await;
    let expected = format!("Bearer {access_for_server}");
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some(expected.as_str())
    );

    server.await?;
    restore_codex_home(previous);
    Ok(())
}

#[tokio::test]
async fn stdio_server_accepts_mcp_tool_calls_with_codex_home() -> Result<()> {
    let access = make_access_token("acct_123", 9_999_999_999);
    let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let requests_for_server = std::sync::Arc::clone(&requests);
    let access_for_server = access.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let request = read_request(&mut stream).await.expect("request");
        requests_for_server.lock().await.push(request);
        write_response(
            &mut stream,
            "200 OK",
            "text/event-stream",
            &sse_body("Hello from MCP"),
        )
        .await
        .expect("response");
    });

    let codex_home = tempdir()?;
    seed_codex_home(codex_home.path(), &access, "acct_123", "person@example.com")?;

    let binary = env!("CARGO_BIN_EXE_openai-codex-mcp");
    let (transport, _stderr) =
        TokioChildProcess::builder(tokio::process::Command::new(binary).configure(|cmd| {
            cmd.env("CODEX_HOME", codex_home.path());
            cmd.env("OPENAI_CODEX_MCP_BASE_URL", format!("http://{addr}"));
            cmd.env("OPENAI_CODEX_MCP_LOG", "error");
        }))
        .spawn()
        .context("failed to spawn MCP child process")?;

    let client: RunningService<RoleClient, ()> = ().serve(transport).await?;
    let result = client
        .call_tool(
            CallToolRequestParams::new("codex_infer").with_arguments(
                json!({
                    "prompt": "Say hello",
                })
                .as_object()
                .expect("tool args object")
                .clone(),
            ),
        )
        .await?;

    let structured = result.structured_content.context("structured content")?;
    assert_eq!(structured["text"], json!("Hello from MCP"));

    let seen = requests.lock().await;
    let expected = format!("Bearer {access_for_server}");
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some(expected.as_str())
    );

    drop(client);
    server.await?;
    Ok(())
}

#[tokio::test]
async fn binary_errors_when_codex_auth_is_missing() -> Result<()> {
    let codex_home = tempdir()?;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_openai-codex-mcp"))
        .env("CODEX_HOME", codex_home.path())
        .output()
        .await
        .context("failed to run server")?;

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("codex login"));
    Ok(())
}

fn seed_codex_home(
    home: &std::path::Path,
    access: &str,
    account_id: &str,
    email: &str,
) -> Result<()> {
    fs::create_dir_all(home)?;
    fs::write(
        home.join("auth.json"),
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

fn restore_codex_home(previous: Option<String>) {
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

#[derive(Debug, Clone)]
struct ObservedRequest {
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
        header_end = find_header_end(&buffer);
    }

    let header_end = header_end.context("request headers were incomplete")?;
    let header_text = String::from_utf8(buffer[..header_end].to_vec())?;
    let mut headers = HashMap::new();
    for line in header_text.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    Ok(ObservedRequest { headers })
}

async fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn sse_body(text: &str) -> String {
    [
        format!(
            "event: response.output_text.delta\ndata: {}\n",
            json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": text,
            })
        ),
        "\n".to_owned(),
        format!(
            "event: response.completed\ndata: {}\n",
            json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "model": "gpt-5.4-mini",
                    "status": "completed",
                    "usage": {
                        "input_tokens": 12,
                        "output_tokens": 3,
                    },
                    "output": [{
                        "type": "message",
                        "id": "msg_1",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": text,
                        }]
                    }]
                }
            })
        ),
        "\n".to_owned(),
    ]
    .concat()
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
