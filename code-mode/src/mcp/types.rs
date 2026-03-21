use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// Request parameters for the `search` tool.
///
/// Searches for available tool/method documentation so that descriptions
/// only enter the context window when relevant.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchRequest {
    /// A query string describing what capability the caller is looking for.
    pub query: String,
}

/// A single entry returned by `search`.
#[derive(Debug, Serialize)]
pub struct SearchResultEntry {
    /// The operation type identifier (e.g. "workspace.fork").
    pub name: String,
    /// Human-readable description of the operation.
    pub description: String,
    /// JSON Schema for the operation's parameters.
    pub parameters_schema: serde_json::Value,
}

/// Opaque input for the `execute` tool.
///
/// The advertised MCP schema is intentionally minimal — just `{ type: string }`
/// with additional properties allowed. Agents call `search` first to discover
/// available operation types and their full parameter schemas.
#[derive(Debug, Deserialize)]
pub struct ExecuteInput {
    #[serde(flatten)]
    pub raw: serde_json::Map<String, serde_json::Value>,
}

impl schemars::JsonSchema for ExecuteInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ExecuteInput")
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "object",
            "required": ["type"],
            "properties": {
                "type": {
                    "type": "string",
                    "description": "Operation type identifier. Use the `search` tool to discover available types and their parameters."
                }
            },
            "additionalProperties": true
        }))
        .expect("static schema is valid")
    }
}

/// The internal discriminated union used for server-side deserialization.
///
/// This type is NOT exposed in the MCP tool schema — agents discover
/// variants via `search` and the server deserializes the raw JSON into
/// this enum.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ExecuteRequest {
    /// Fork the current workspace into a child workspace.
    #[serde(rename = "workspace.fork")]
    WorkspaceFork {
        /// A human-readable description of what the child workspace will do.
        description: String,
    },

    /// Join the child workspace back into its parent, rebasing changes.
    #[serde(rename = "workspace.join")]
    WorkspaceJoin,

    /// Snapshot the current working copy as a commit.
    #[serde(rename = "workspace.snapshot")]
    WorkspaceSnapshot {
        /// Commit message for the snapshot.
        message: String,
    },

    /// Update the description on the current workspace's working-copy commit.
    #[serde(rename = "workspace.describe")]
    WorkspaceDescribe {
        /// The new description.
        message: String,
    },

    /// Perform a stateless LLM sub-call.
    #[serde(rename = "llm_query")]
    LlmQuery {
        /// The prompt to send to the LLM.
        prompt: String,
    },
}

/// The result returned by `execute`.
#[derive(Debug, Serialize)]
pub struct ExecuteResult {
    /// Whether the operation succeeded.
    pub success: bool,
    /// A human-readable message or the operation's output.
    pub message: String,
    /// Optional structured data returned by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Returns the full operation registry — all known execute variants
/// with their descriptions and parameter schemas.
pub fn operation_registry() -> Vec<SearchResultEntry> {
    vec![
        SearchResultEntry {
            name: "workspace.fork".into(),
            description: "Fork the current workspace into a child workspace.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "required": ["type", "description"],
                "properties": {
                    "type": { "const": "workspace.fork" },
                    "description": {
                        "type": "string",
                        "description": "A human-readable description of what the child workspace will do."
                    }
                }
            }),
        },
        SearchResultEntry {
            name: "workspace.join".into(),
            description: "Join the child workspace back into its parent, rebasing changes.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": { "const": "workspace.join" }
                }
            }),
        },
        SearchResultEntry {
            name: "workspace.snapshot".into(),
            description: "Snapshot the current working copy as a commit.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "required": ["type", "message"],
                "properties": {
                    "type": { "const": "workspace.snapshot" },
                    "message": {
                        "type": "string",
                        "description": "Commit message for the snapshot."
                    }
                }
            }),
        },
        SearchResultEntry {
            name: "workspace.describe".into(),
            description: "Update the description on the current workspace's working-copy commit."
                .into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "required": ["type", "message"],
                "properties": {
                    "type": { "const": "workspace.describe" },
                    "message": {
                        "type": "string",
                        "description": "The new description."
                    }
                }
            }),
        },
        SearchResultEntry {
            name: "llm_query".into(),
            description: "Perform a stateless LLM sub-call.".into(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "required": ["type", "prompt"],
                "properties": {
                    "type": { "const": "llm_query" },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt to send to the LLM."
                    }
                }
            }),
        },
    ]
}
