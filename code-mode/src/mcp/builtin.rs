use std::sync::Arc;

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, Tool},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use super::downstream::{DownstreamManager, LogReadResult, LogSessionSummary};
use super::types::{SearchResultEntry, wrap_execute_schema};

pub const SYSTEM_SERVER_NAME: &str = "system";
const LOGS_CURRENT_TOOL: &str = "logs_current";
const LOGS_READ_TOOL: &str = "logs_read";
const DEFAULT_LOG_BYTES: usize = 8 * 1024;
const MAX_LOG_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
struct LogsCurrentArgs {
    server: Option<String>,
}

#[derive(Debug, Serialize)]
struct LogsCurrentResult {
    sessions: Vec<LogSessionSummary>,
}

#[derive(Debug, Deserialize)]
struct LogsReadArgs {
    session_id: String,
    offset: Option<u64>,
    max_bytes: Option<usize>,
}

pub fn tools() -> Vec<Tool> {
    vec![
        tool(
            LOGS_CURRENT_TOOL,
            "List active downstream MCP log sessions and their associated log files.",
            logs_current_input_schema(),
            logs_current_output_schema(),
        ),
        tool(
            LOGS_READ_TOOL,
            "Read a chunk from a downstream MCP log file by session id.",
            logs_read_input_schema(),
            logs_read_output_schema(),
        ),
    ]
}

pub fn search_registry() -> Vec<SearchResultEntry> {
    tools()
        .into_iter()
        .map(|tool| {
            let name = format!("{}.{}", SYSTEM_SERVER_NAME, tool.name.as_ref());
            SearchResultEntry {
                name: name.clone(),
                description: tool
                    .description
                    .as_ref()
                    .map(|description| description.to_string())
                    .unwrap_or_else(|| format!("Call {name}.")),
                parameters_schema: wrap_execute_schema(&name, &tool_input_schema(&tool)),
            }
        })
        .collect()
}

pub fn has_tool(tool_name: &str) -> bool {
    matches!(tool_name, LOGS_CURRENT_TOOL | LOGS_READ_TOOL)
}

pub async fn execute(
    tool_name: &str,
    arguments: Map<String, Value>,
    downstream: &DownstreamManager,
) -> Result<CallToolResult, McpError> {
    match tool_name {
        LOGS_CURRENT_TOOL => execute_logs_current(arguments, downstream).await,
        LOGS_READ_TOOL => execute_logs_read(arguments, downstream).await,
        _ => Err(McpError::invalid_params(
            format!("unknown built-in operation: {SYSTEM_SERVER_NAME}.{tool_name}"),
            None,
        )),
    }
}

async fn execute_logs_current(
    arguments: Map<String, Value>,
    downstream: &DownstreamManager,
) -> Result<CallToolResult, McpError> {
    let args: LogsCurrentArgs = parse_args(LOGS_CURRENT_TOOL, arguments)?;
    let sessions = downstream
        .current_log_sessions(args.server.as_deref())
        .await;
    structured_result(&LogsCurrentResult { sessions })
}

async fn execute_logs_read(
    arguments: Map<String, Value>,
    downstream: &DownstreamManager,
) -> Result<CallToolResult, McpError> {
    let args: LogsReadArgs = parse_args(LOGS_READ_TOOL, arguments)?;
    let max_bytes = args
        .max_bytes
        .unwrap_or(DEFAULT_LOG_BYTES)
        .clamp(1, MAX_LOG_BYTES);
    let result: LogReadResult = downstream
        .read_log(&args.session_id, args.offset.unwrap_or(0), max_bytes)
        .await
        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
    structured_result(&result)
}

fn structured_result<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_value(value)
        .map_err(|e| McpError::internal_error(format!("failed to serialize result: {e}"), None))?;
    Ok(CallToolResult::structured(json))
}

fn parse_args<T: DeserializeOwned>(
    tool_name: &str,
    arguments: Map<String, Value>,
) -> Result<T, McpError> {
    serde_json::from_value(Value::Object(arguments)).map_err(|e| {
        McpError::invalid_params(
            format!(
                "invalid parameters for {}.{}: {e}",
                SYSTEM_SERVER_NAME, tool_name
            ),
            None,
        )
    })
}

fn tool(name: &str, description: &str, input_schema: Value, output_schema: Value) -> Tool {
    Tool::new(
        name.to_string(),
        description.to_string(),
        Arc::new(expect_schema_object(input_schema)),
    )
    .with_raw_output_schema(Arc::new(expect_schema_object(output_schema)))
}

fn tool_input_schema(tool: &Tool) -> Value {
    Value::Object((*tool.input_schema).clone())
}

fn expect_schema_object(schema: Value) -> serde_json::Map<String, Value> {
    schema
        .as_object()
        .cloned()
        .expect("built-in schema should always be an object")
}

fn logs_current_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "server": {
                "type": "string",
                "description": "Optional downstream server name to filter active log sessions."
            }
        },
        "additionalProperties": false
    })
}

fn logs_current_output_schema() -> Value {
    json!({
        "type": "object",
        "required": ["sessions"],
        "properties": {
            "sessions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["server", "session_id", "log_path", "started_at_unix_ms", "active"],
                    "properties": {
                        "server": { "type": "string" },
                        "session_id": { "type": "string" },
                        "log_path": { "type": "string" },
                        "pid": { "type": "integer" },
                        "started_at_unix_ms": { "type": "integer" },
                        "active": { "type": "boolean" }
                    },
                    "additionalProperties": false
                }
            }
        },
        "additionalProperties": false
    })
}

fn logs_read_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["session_id"],
        "properties": {
            "session_id": {
                "type": "string",
                "description": "The session id returned by system.logs_current."
            },
            "offset": {
                "type": "integer",
                "minimum": 0,
                "description": "Byte offset to start reading from. Defaults to 0."
            },
            "max_bytes": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LOG_BYTES,
                "description": "Maximum number of bytes to read. Defaults to 8192."
            }
        },
        "additionalProperties": false
    })
}

fn logs_read_output_schema() -> Value {
    json!({
        "type": "object",
        "required": ["server", "session_id", "log_path", "offset", "next_offset", "eof", "text"],
        "properties": {
            "server": { "type": "string" },
            "session_id": { "type": "string" },
            "log_path": { "type": "string" },
            "offset": { "type": "integer" },
            "next_offset": { "type": "integer" },
            "eof": { "type": "boolean" },
            "text": { "type": "string" }
        },
        "additionalProperties": false
    })
}
