use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::{Peer, RunningService},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use tokio::sync::Mutex;

use super::config::{Config, ServerEntry};

/// A live connection to a downstream MCP server.
struct LiveConnection {
    peer: Peer<RoleClient>,
    /// Keep the RunningService alive so the connection isn't dropped.
    _service: RunningService<RoleClient, ()>,
}

/// Manages connections to downstream MCP servers for tool routing.
pub struct DownstreamManager {
    configs: HashMap<String, ServerEntry>,
    connections: Mutex<HashMap<String, Arc<LiveConnection>>>,
}

impl DownstreamManager {
    /// Build a manager from the merged config, keeping only stdio servers.
    pub fn from_config(config: &Config) -> Self {
        let configs: HashMap<String, ServerEntry> = config
            .servers
            .iter()
            .filter(|(_, e)| e.transport == "stdio" && e.command.is_some())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Self {
            configs,
            connections: Mutex::new(HashMap::new()),
        }
    }

    /// Connect to a downstream server (or return a cached connection).
    async fn get_connection(&self, server_name: &str) -> Result<Arc<LiveConnection>> {
        let mut connections = self.connections.lock().await;

        if let Some(conn) = connections.get(server_name) {
            if !conn._service.is_closed() {
                return Ok(Arc::clone(conn));
            }
            // Connection is dead, remove it
            connections.remove(server_name);
        }

        let entry = self
            .configs
            .get(server_name)
            .with_context(|| format!("no downstream server configured: {server_name}"))?;

        let command = entry.command.as_deref().unwrap();

        let transport = TokioChildProcess::new(
            tokio::process::Command::new(command).configure(|cmd| {
                cmd.args(&entry.args);
                cmd.envs(&entry.env);
            }),
        )
        .with_context(|| format!("failed to spawn downstream server: {server_name}"))?;

        let service = ()
            .serve(transport)
            .await
            .with_context(|| format!("failed to initialize downstream client: {server_name}"))?;

        let peer = service.peer().clone();
        let conn = Arc::new(LiveConnection {
            peer,
            _service: service,
        });

        connections.insert(server_name.to_string(), Arc::clone(&conn));
        Ok(conn)
    }

    /// Check if a server name is known.
    pub fn has_server(&self, server_name: &str) -> bool {
        self.configs.contains_key(server_name)
    }

    /// Call a tool on a downstream server.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResult> {
        if !self.configs.contains_key(server_name) {
            bail!("unknown downstream server: {server_name}");
        }

        let conn = self.get_connection(server_name).await?;

        let params = CallToolRequestParams::new(tool_name.to_string()).with_arguments(arguments);

        let result = conn
            .peer
            .call_tool(params)
            .await
            .with_context(|| format!("failed to call {server_name}.{tool_name}"))?;

        Ok(result)
    }
}
