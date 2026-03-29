use std::{path::PathBuf, sync::Arc};

use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters, model::*, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::builtin::{self, SYSTEM_SERVER_NAME};
use super::config::Config;
use super::downstream::{DownstreamCallError, DownstreamManager};
use super::types::{ExecuteInput, SearchRequest, SearchResultEntry, wrap_execute_schema};

/// The Code Mode MCP server.
///
/// Exposes exactly two tools to agents:
/// - `search`: returns documentation for available operations on demand,
///   so that operation schemas only enter the context window when relevant.
/// - `execute`: a single entry-point with an intentionally minimal schema
///   (`{ type: string }`). Agents call `search` first to discover the full
///   parameter schemas for each operation type.
///
/// Routes `<server>.<tool>` type patterns to downstream MCP servers.
#[derive(Clone)]
pub struct CodeModeServer {
    tool_router: ToolRouter<CodeModeServer>,
    downstream: Arc<DownstreamManager>,
    registry: Arc<Vec<SearchResultEntry>>,
}

#[derive(Debug, Serialize)]
struct GatewayErrorData {
    #[serde(rename = "type")]
    kind: &'static str,
    service: String,
    server: String,
    tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    retryable: bool,
}

#[derive(Debug, Deserialize)]
struct AuthRequiredPayload {
    #[serde(rename = "type")]
    kind: String,
    service: Option<String>,
    reason: Option<String>,
    message: Option<String>,
    url: Option<String>,
    retryable: Option<bool>,
}

#[tool_router]
impl CodeModeServer {
    pub fn new(config: &Config) -> Self {
        let downstream = DownstreamManager::from_config(config);
        let mut registry = builtin::search_registry();
        registry.extend(load_manifest_registry(config));

        Self {
            tool_router: Self::tool_router(),
            downstream: Arc::new(downstream),
            registry: Arc::new(registry),
        }
    }

    /// Search for available tool/method documentation.
    ///
    /// Returns descriptions and parameter schemas for operations matching the
    /// query, so that documentation only enters the context window when relevant.
    #[tool(
        description = "Search for available tool/method documentation. Returns operation names, descriptions, and parameter schemas matching the query."
    )]
    pub async fn search(
        &self,
        Parameters(req): Parameters<SearchRequest>,
    ) -> Result<CallToolResult, McpError> {
        info!(query = %req.query, "search called");

        let query = req.query.to_lowercase();

        let results: Vec<_> = self
            .registry
            .iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&query)
                    || entry.description.to_lowercase().contains(&query)
            })
            .collect();

        let json = serde_json::to_string_pretty(&results).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Execute a code-mode-mediated operation.
    ///
    /// The advertised schema is intentionally minimal — just `{ type: string }`.
    /// Agents should call `search` first to discover operation types and their
    /// full parameter schemas.
    #[tool(
        description = "Execute a code-mode-mediated operation. Pass a JSON object with a `type` field. Use `search` to discover available operation types and their required parameters."
    )]
    pub async fn execute(
        &self,
        Parameters(input): Parameters<ExecuteInput>,
    ) -> Result<CallToolResult, McpError> {
        let type_str = input
            .raw
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| McpError::invalid_params("missing 'type' field", None))?;

        if let Some((server, tool)) = type_str.split_once('.') {
            let mut args = input.raw;
            args.remove("type");

            if server == SYSTEM_SERVER_NAME && builtin::has_tool(tool) {
                return builtin::execute(tool, args, &self.downstream).await;
            }

            if self.downstream.has_server(server) {
                let result = self
                    .downstream
                    .call_tool(server, tool, args)
                    .await
                    .map_err(|e| classify_downstream_error(server, tool, e))?;

                return Ok(result);
            }
        }

        Err(McpError::invalid_params(
            format!("unknown operation type: {type_str}"),
            None,
        ))
    }
}

fn classify_downstream_error(server: &str, tool: &str, error: DownstreamCallError) -> McpError {
    match error {
        DownstreamCallError::Tool(error) => {
            if let Some(payload) = parse_auth_required_payload(error.data.as_ref()) {
                let message = payload.message.unwrap_or_else(|| error.message.to_string());
                let data = GatewayErrorData {
                    kind: "auth_required",
                    service: payload.service.unwrap_or_else(|| server.to_string()),
                    server: server.to_string(),
                    tool: tool.to_string(),
                    reason: payload.reason,
                    message: message.clone(),
                    url: payload.url,
                    retryable: payload.retryable.unwrap_or(true),
                };
                return McpError::new(error.code, message, Some(serde_json::json!(data)));
            }

            let message = error.message.to_string();
            let data = GatewayErrorData {
                kind: "upstream_error",
                service: server.to_string(),
                server: server.to_string(),
                tool: tool.to_string(),
                reason: None,
                message: message.clone(),
                url: None,
                retryable: false,
            };
            McpError::new(error.code, message, Some(serde_json::json!(data)))
        }
        DownstreamCallError::Other(error) => {
            let message = format_error_chain(&error);
            let data = GatewayErrorData {
                kind: "upstream_error",
                service: server.to_string(),
                server: server.to_string(),
                tool: tool.to_string(),
                reason: None,
                message: message.clone(),
                url: None,
                retryable: false,
            };
            McpError::internal_error(message, Some(serde_json::json!(data)))
        }
    }
}

fn parse_auth_required_payload(data: Option<&serde_json::Value>) -> Option<AuthRequiredPayload> {
    let payload = serde_json::from_value::<AuthRequiredPayload>(data?.clone()).ok()?;
    if payload.kind == "auth_required" {
        Some(payload)
    } else {
        None
    }
}

/// Load downstream tool registry from the generated manifest.json.
fn load_manifest_registry(config: &Config) -> Vec<SearchResultEntry> {
    let base_dir = config
        .base_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(".code-mode"));
    let base_dir = if base_dir.is_absolute() {
        base_dir
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(base_dir)
    };
    let manifest_path = base_dir.join("sdk/manifest.json");
    let Ok(content) = std::fs::read_to_string(manifest_path) else {
        return Vec::new();
    };

    #[derive(serde::Deserialize)]
    struct ManifestTool {
        server: String,
        name: String,
        description: Option<String>,
        input_schema: serde_json::Value,
    }

    let Ok(tools) = serde_json::from_str::<Vec<ManifestTool>>(&content) else {
        return Vec::new();
    };

    tools
        .into_iter()
        .map(|t| {
            let op_name = format!("{}.{}", t.server, t.name);
            let description = t
                .description
                .unwrap_or_else(|| format!("Call {op_name} on downstream server"));
            SearchResultEntry {
                name: op_name,
                description,
                parameters_schema: wrap_execute_schema(
                    &format!("{}.{}", t.server, t.name),
                    &t.input_schema,
                ),
            }
        })
        .collect()
}

#[tool_handler]
impl ServerHandler for CodeModeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("code-mode", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Code Mode MCP server. Use `search` to discover available operations \
             and their parameter schemas, then `execute` to run them.",
            )
    }
}

fn format_error_chain(error: &anyhow::Error) -> String {
    let mut messages = Vec::new();
    for cause in error.chain() {
        let message = cause.to_string();
        if messages.last() != Some(&message) {
            messages.push(message);
        }
    }
    messages.join(": ")
}

#[cfg(test)]
mod tests {
    use super::format_error_chain;

    #[test]
    fn formats_nested_error_causes() {
        use anyhow::{Context, anyhow};

        let error = Err::<(), _>(anyhow!("root cause"))
            .context("outer context")
            .unwrap_err();
        let message = format_error_chain(&error);
        assert_eq!(message, "outer context: root cause");
    }
}
