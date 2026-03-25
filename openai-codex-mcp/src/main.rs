use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use openai_codex_mcp::{codex_cli, config::load_config, mcp::OpenAICodexMcpServer};
use rmcp::{ServiceExt, transport::stdio};

#[derive(Parser)]
#[command(name = "openai-codex-mcp")]
struct Cli {
    /// Path to an openai-codex-mcp.toml config file
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config(cli.config.as_deref())?;
    init_tracing(&config)?;

    codex_cli::read_required_codex_cli_credential()?;

    let backend = openai_codex_mcp::backend::CodexBackend::new(config)?;
    let service = OpenAICodexMcpServer::new(backend)
        .serve(stdio())
        .await
        .context("failed to start MCP server")?;
    service.waiting().await?;
    Ok(())
}

fn init_tracing(config: &openai_codex_mcp::config::Config) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(config.log.to_tracing_env_filter()?)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init()
        .ok();
    Ok(())
}
