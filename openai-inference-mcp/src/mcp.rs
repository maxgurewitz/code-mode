use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::backend::OpenAIBackend;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RawOpenAIRequestInput {
    pub body: Value,
}

#[derive(Clone)]
pub struct OpenAIInferenceMcpServer {
    tool_router: ToolRouter<OpenAIInferenceMcpServer>,
    backend: Arc<OpenAIBackend>,
}

#[tool_router]
impl OpenAIInferenceMcpServer {
    pub fn new(backend: OpenAIBackend) -> Self {
        Self {
            tool_router: Self::tool_router(),
            backend: Arc::new(backend),
        }
    }

    #[tool(
        description = "POST a raw JSON body to OpenAI's /v1/responses endpoint and return the raw JSON response."
    )]
    pub async fn responses_create(
        &self,
        Parameters(input): Parameters<RawOpenAIRequestInput>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .backend
            .create_response(input.body)
            .await
            .map_err(|error| McpError::internal_error(format_error_chain(&error), None))?;
        Ok(CallToolResult::structured(result))
    }

    #[tool(
        description = "POST a raw JSON body to OpenAI's /v1/chat/completions endpoint and return the raw JSON response."
    )]
    pub async fn chat_completions_create(
        &self,
        Parameters(input): Parameters<RawOpenAIRequestInput>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .backend
            .create_chat_completion(input.body)
            .await
            .map_err(|error| McpError::internal_error(format_error_chain(&error), None))?;
        Ok(CallToolResult::structured(result))
    }

    #[tool(
        description = "POST a raw JSON body to OpenAI's /v1/embeddings endpoint and return the raw JSON response."
    )]
    pub async fn embeddings_create(
        &self,
        Parameters(input): Parameters<RawOpenAIRequestInput>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .backend
            .create_embedding(input.body)
            .await
            .map_err(|error| McpError::internal_error(format_error_chain(&error), None))?;
        Ok(CallToolResult::structured(result))
    }
}

#[tool_handler]
impl ServerHandler for OpenAIInferenceMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "openai-inference-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Direct OpenAI inference MCP server. Use the raw JSON body accepted by each OpenAI endpoint and pass non-streaming requests.",
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
