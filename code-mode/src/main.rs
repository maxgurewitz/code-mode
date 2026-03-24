use anyhow::Result;
use clap::{Parser, Subcommand};
use rmcp::{ServiceExt, handler::server::wrapper::Parameters, transport::stdio};

mod mcp;

use mcp::config::load_config;
use mcp::server::CodeModeServer;
use mcp::types::{ExecuteInput, SearchRequest};

#[derive(Parser)]
#[command(name = "code-mode")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// MCP-related commands
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
}

#[derive(Subcommand)]
enum McpCommands {
    /// Discover tools from configured MCP servers and generate a TypeScript SDK
    Generate {
        /// Base directory for generated output (default: .code-mode)
        #[arg(long)]
        base_dir: Option<std::path::PathBuf>,
        /// Path to code-mode.toml config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Start the Code Mode MCP server on stdio
    Serve {
        /// Path to code-mode.toml config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Call the search tool with a JSON payload
    Search {
        /// JSON payload (same as the MCP tool input)
        #[arg(short, long)]
        data: String,
    },
    /// Call the execute tool with a JSON payload
    Execute {
        /// JSON payload (same as the MCP tool input)
        #[arg(short, long)]
        data: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Mcp { command } => match command {
            McpCommands::Generate { base_dir, config } => {
                let config = load_config(config.as_deref())?;
                mcp::generate::execute(&config, base_dir.as_deref()).await?;
            }
            McpCommands::Search { data } => {
                let config = load_config(None)?;
                let server = CodeModeServer::new(&config);
                let req: SearchRequest = serde_json::from_str(&data)?;
                let result = server
                    .search(Parameters(req))
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e.message))?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            McpCommands::Execute { data } => {
                let config = load_config(None)?;
                let server = CodeModeServer::new(&config);
                let input: ExecuteInput = serde_json::from_str(&data)?;
                let result = server
                    .execute(Parameters(input))
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e.message))?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            McpCommands::Serve { config } => {
                let cfg = load_config(config.as_deref())?;
                tracing_subscriber::fmt()
                    .with_env_filter(cfg.log.to_tracing_env_filter()?)
                    .with_writer(std::io::stderr)
                    .with_ansi(false)
                    .init();

                tracing::info!("Starting Code Mode MCP server");

                let service = CodeModeServer::new(&cfg)
                    .serve(stdio())
                    .await
                    .inspect_err(|e| {
                        tracing::error!("serving error: {:?}", e);
                    })?;

                service.waiting().await?;
            }
        },
    }

    Ok(())
}
