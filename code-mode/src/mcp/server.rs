use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters, model::*, tool, tool_handler, tool_router,
};
use tracing::info;

use super::config::Config;
use super::downstream::DownstreamManager;
use super::types::{
    ExecuteInput, ExecuteRequest, ExecuteResult, SearchRequest, SearchResultEntry,
    operation_registry,
};

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
    downstream_registry: Arc<Vec<SearchResultEntry>>,
}

#[tool_router]
impl CodeModeServer {
    pub fn new(config: &Config) -> Self {
        let downstream = DownstreamManager::from_config(config);
        let downstream_registry = load_manifest_registry();

        Self {
            tool_router: Self::tool_router(),
            downstream: Arc::new(downstream),
            downstream_registry: Arc::new(downstream_registry),
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

        let static_results = operation_registry();
        let results: Vec<_> = static_results
            .iter()
            .chain(self.downstream_registry.iter())
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
        // Extract the type string first (before consuming input)
        let type_str = input
            .raw
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| McpError::invalid_params("missing 'type' field", None))?;

        // Try static types first
        let raw_value = serde_json::Value::Object(input.raw.clone());
        if let Ok(req) = serde_json::from_value::<ExecuteRequest>(raw_value) {
            return self.handle_static_execute(req);
        }

        // Try downstream routing: check for server.tool pattern
        if let Some((server, tool)) = type_str.split_once('.') {
            if self.downstream.has_server(server) {
                let mut args = input.raw;
                args.remove("type");

                let result = self
                    .downstream
                    .call_tool(server, tool, args)
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                return Ok(result);
            }
        }

        Err(McpError::invalid_params(
            format!("unknown operation type: {type_str}"),
            None,
        ))
    }
}

impl CodeModeServer {
    fn handle_static_execute(&self, req: ExecuteRequest) -> Result<CallToolResult, McpError> {
        let result = match req {
            ExecuteRequest::WorkspaceFork { ref description } => {
                info!(description = %description, "execute: workspace.fork");
                ExecuteResult {
                    success: true,
                    message: format!(
                        "workspace.fork: not yet implemented (description: {description})"
                    ),
                    data: None,
                }
            }
            ExecuteRequest::WorkspaceJoin => {
                info!("execute: workspace.join");
                ExecuteResult {
                    success: true,
                    message: "workspace.join: not yet implemented".into(),
                    data: None,
                }
            }
            ExecuteRequest::WorkspaceSnapshot { ref message } => {
                info!(message = %message, "execute: workspace.snapshot");
                ExecuteResult {
                    success: true,
                    message: format!(
                        "workspace.snapshot: not yet implemented (message: {message})"
                    ),
                    data: None,
                }
            }
            ExecuteRequest::WorkspaceDescribe { ref message } => {
                info!(message = %message, "execute: workspace.describe");
                ExecuteResult {
                    success: true,
                    message: format!(
                        "workspace.describe: not yet implemented (message: {message})"
                    ),
                    data: None,
                }
            }
            ExecuteRequest::LlmQuery { ref prompt } => {
                info!(prompt_len = prompt.len(), "execute: llm_query");
                ExecuteResult {
                    success: true,
                    message: "llm_query: not yet implemented".into(),
                    data: None,
                }
            }
        };

        let json = serde_json::to_string_pretty(&result).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

/// Load downstream tool registry from the generated manifest.json.
fn load_manifest_registry() -> Vec<SearchResultEntry> {
    let manifest_path = std::path::Path::new(".code-mode/sdk/manifest.json");
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
                parameters_schema: t.input_schema,
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
