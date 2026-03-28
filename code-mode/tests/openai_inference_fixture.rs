use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::Command,
};

#[tokio::test]
async fn code_mode_can_call_openai_inference_fixture_server() -> Result<()> {
    let api_key = "fixture_openai_key";
    let api_key_for_backend = api_key.to_owned();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let backend = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let request = read_request(&mut stream).await.expect("request");
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some(format!("Bearer {api_key_for_backend}").as_str())
        );
        assert_eq!(request.path, "/v1/responses");
        write_response(
            &mut stream,
            "200 OK",
            "application/json",
            &json!({
                "id": "resp_fixture",
                "model": "gpt-5-mini",
                "output_text": "Hello through openai-inference-mcp"
            })
            .to_string(),
        )
        .await
        .expect("response");
    });

    let temp = tempdir()?;
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?
        .join("openai-inference-mcp")
        .join("Cargo.toml");
    let config_path = temp.path().join("code-mode.toml");
    let toml = format!(
        r#"
base_dir = ".tmp/.code-mode"

[servers.openai-inference]
command = "cargo"
args = ["run", "-q", "--manifest-path", {manifest_path:?}]
env = {{ OPENAI_INFERENCE_MCP_API_KEY = {api_key:?}, OPENAI_INFERENCE_MCP_BASE_URL = {base_url:?}, OPENAI_INFERENCE_MCP_LOG = "error" }}
"#,
        manifest_path = manifest_path.display().to_string(),
        api_key = api_key,
        base_url = format!("http://{addr}"),
    );
    tokio::fs::write(&config_path, toml).await?;

    let output = Command::new(env!("CARGO_BIN_EXE_code-mode"))
        .arg("mcp")
        .arg("execute")
        .arg("--data")
        .arg(r#"{"type":"openai-inference.responses_create","body":{"model":"gpt-5-mini","input":"Say hello"}}"#)
        .current_dir(temp.path())
        .output()
        .await
        .context("failed to run code-mode execute")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "code-mode failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let result: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        result["structuredContent"]["output_text"],
        json!("Hello through openai-inference-mcp")
    );

    backend.await?;
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

    let header_end = header_end.context("incomplete request")?;
    let header_text = String::from_utf8(buffer[..header_end].to_vec())?;
    let request_line = header_text.lines().next().context("request line missing")?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .context("request path missing")?
        .to_owned();
    let mut headers = HashMap::new();
    for line in header_text.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    Ok(ObservedRequest { path, headers })
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
