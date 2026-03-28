use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use openai_inference_mcp::{
    backend::OpenAIBackend, config::load_config, mcp::OpenAIInferenceMcpServer,
};
use rmcp::{ServiceExt, transport::stdio};

#[derive(Parser)]
#[command(name = "openai-inference-mcp")]
struct Cli {
    /// Path to an openai-inference-mcp.toml config file
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config(cli.config.as_deref())?;
    init_tracing(&config)?;

    let backend = OpenAIBackend::new(config)?;
    let service = OpenAIInferenceMcpServer::new(backend)
        .serve(stdio())
        .await
        .context("failed to start MCP server")?;
    service.waiting().await?;
    Ok(())
}

fn init_tracing(config: &openai_inference_mcp::config::Config) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(config.log.to_tracing_env_filter()?)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init()
        .ok();
    Ok(())
}
