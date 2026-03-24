use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

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
    /// The operation type identifier (e.g. "server.tool").
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

pub fn wrap_execute_schema(type_name: &str, input_schema: &Value) -> Value {
    let mut schema = match input_schema {
        Value::Object(schema) => schema.clone(),
        _ => Map::new(),
    };

    if !matches!(schema.get("type"), Some(Value::String(schema_type)) if schema_type == "object") {
        schema.insert("type".into(), Value::String("object".into()));
    }

    let mut properties = match schema.remove("properties") {
        Some(Value::Object(properties)) => properties,
        _ => Map::new(),
    };
    properties.insert("type".into(), json!({ "const": type_name }));
    schema.insert("properties".into(), Value::Object(properties));

    let mut required = match schema.remove("required") {
        Some(Value::Array(required)) => required,
        _ => Vec::new(),
    };
    if !required
        .iter()
        .any(|value| matches!(value, Value::String(item) if item == "type"))
    {
        required.insert(0, Value::String("type".into()));
    }
    schema.insert("required".into(), Value::Array(required));

    if !schema.contains_key("additionalProperties") {
        schema.insert("additionalProperties".into(), Value::Bool(true));
    }

    Value::Object(schema)
}
