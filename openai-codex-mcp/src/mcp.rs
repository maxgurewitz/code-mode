use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::{CodexBackend, CodexInferRequest, CodexResponseRequest};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CodexInferToolInput {
    pub prompt: String,
    pub model: Option<String>,
    pub instructions: Option<String>,
    #[schemars(with = "Option<String>")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CodexInferToolOutput {
    pub text: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CodexResponseToolInput {
    pub model: Option<String>,
    pub input: Value,
    pub instructions: Option<String>,
    #[schemars(with = "Option<String>")]
    pub reasoning_effort: Option<String>,
    pub include_raw_events: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct CodexResponseToolOutput {
    pub output_text: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_events: Option<Vec<Value>>,
}

#[derive(Clone)]
pub struct OpenAICodexMcpServer {
    tool_router: ToolRouter<OpenAICodexMcpServer>,
    backend: Arc<CodexBackend>,
}

#[tool_router]
impl OpenAICodexMcpServer {
    pub fn new(backend: CodexBackend) -> Self {
        Self {
            tool_router: Self::tool_router(),
            backend: Arc::new(backend),
        }
    }

    #[tool(
        description = "Run a single prompt against the OpenAI Codex backend and return plain text output."
    )]
    pub async fn codex_infer(
        &self,
        Parameters(input): Parameters<CodexInferToolInput>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .backend
            .infer(CodexInferRequest {
                prompt: input.prompt,
                model: input.model,
                instructions: input.instructions,
                reasoning_effort: input.reasoning_effort,
            })
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        structured_result(&CodexInferToolOutput {
            text: result.text,
            model: result.model,
            response_id: result.response_id,
            finish_reason: result.finish_reason,
            usage: result.usage,
        })
    }

    #[tool(
        description = "Run a Responses-style text request against the OpenAI Codex backend and return normalized output."
    )]
    pub async fn codex_response(
        &self,
        Parameters(input): Parameters<CodexResponseToolInput>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .backend
            .response(CodexResponseRequest {
                model: input.model,
                input: input.input,
                instructions: input.instructions,
                reasoning_effort: input.reasoning_effort,
                include_raw_events: input.include_raw_events.unwrap_or(false),
            })
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;

        structured_result(&CodexResponseToolOutput {
            output_text: result.text,
            model: result.model,
            response_id: result.response_id,
            finish_reason: result.finish_reason,
            usage: result.usage,
            raw_events: result.raw_events,
        })
    }
}

#[tool_handler]
impl ServerHandler for OpenAICodexMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "openai-codex-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Local OpenAI Codex MCP server. Use `codex_infer` for simple prompt-in text-out calls or `codex_response` for a richer Responses-style input.",
            )
    }
}

fn structured_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_value(value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::structured(json))
}
