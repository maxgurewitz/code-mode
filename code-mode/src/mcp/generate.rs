use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rmcp::{
    ServiceExt,
    model::Tool,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde::{Deserialize, Serialize};

use super::config::{Config, ServerEntry};

/// A discovered tool from a downstream MCP server.
#[derive(Debug, Serialize, Deserialize)]
struct DiscoveredTool {
    server: String,
    name: String,
    description: Option<String>,
    input_schema: serde_json::Value,
}

/// Result of discovering a server: its instructions and tools.
struct DiscoveryResult {
    instructions: Option<String>,
    tools: Vec<Tool>,
}

/// Discover tools and instructions from a single stdio MCP server.
async fn discover_server(name: &str, entry: &ServerEntry) -> Result<DiscoveryResult> {
    let command = entry
        .command
        .as_deref()
        .context("stdio server missing command")?;
    let transport =
        TokioChildProcess::new(tokio::process::Command::new(command).configure(|cmd| {
            cmd.args(&entry.args);
            cmd.envs(&entry.env);
        }))
        .with_context(|| format!("failed to spawn MCP server: {name}"))?;

    let service = ()
        .serve(transport)
        .await
        .with_context(|| format!("failed to initialize MCP client for: {name}"))?;

    let instructions = service
        .peer_info()
        .and_then(|info| info.instructions.clone());

    let tools = service
        .peer()
        .list_all_tools()
        .await
        .with_context(|| format!("failed to list tools from: {name}"))?;

    service.cancel().await.ok();

    Ok(DiscoveryResult {
        instructions,
        tools,
    })
}

/// Map a JSON Schema type string to a TypeScript type.
fn json_type_to_ts(schema_type: &str) -> &str {
    match schema_type {
        "string" => "string",
        "number" | "integer" => "number",
        "boolean" => "boolean",
        _ => "unknown",
    }
}

/// Generate TypeScript type annotation for a tool's input_schema.
fn generate_ts_args(input_schema: &serde_json::Value) -> String {
    let Some(properties) = input_schema.get("properties").and_then(|p| p.as_object()) else {
        return "args: Record<string, unknown>".into();
    };

    if properties.is_empty() {
        return String::new();
    }

    let required: Vec<&str> = input_schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut fields = Vec::new();
    for (name, schema) in properties {
        let ts_type = schema
            .get("type")
            .and_then(|t| t.as_str())
            .map(json_type_to_ts)
            .unwrap_or("unknown");
        let optional = if required.contains(&name.as_str()) {
            ""
        } else {
            "?"
        };
        fields.push(format!("{name}{optional}: {ts_type}"));
    }

    format!("args: {{ {} }}", fields.join("; "))
}

/// Convert a server name (e.g. "hello-world") to a valid JS identifier (e.g. "helloWorld").
fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '-' || ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Generate the base directory structure and TypeScript SDK.
fn generate(base_dir: &Path, servers: &[(String, DiscoveryResult)]) -> Result<()> {
    let sdk_dir = base_dir.join("sdk");
    std::fs::create_dir_all(&sdk_dir)
        .with_context(|| format!("failed to create SDK directory: {}", sdk_dir.display()))?;

    // Write package.json in base dir (for bun install)
    let package_json = serde_json::json!({
        "name": "@code-mode/root",
        "version": "1.0.0",
        "type": "module",
        "private": true,
        "dependencies": {
            "@modelcontextprotocol/sdk": "^1.0.0"
        }
    });
    std::fs::write(
        base_dir.join("package.json"),
        serde_json::to_string_pretty(&package_json)?,
    )?;
    println!("  wrote package.json");

    // Write client.ts — embed the absolute path to the code-mode binary
    let exe_path = std::env::current_exe().context("failed to determine code-mode binary path")?;
    let exe_str = exe_path.display().to_string().replace('\\', "\\\\");

    let client_ts = format!(
        r#"import {{ Client }} from "@modelcontextprotocol/sdk/client/index.js";
import {{ StdioClientTransport }} from "@modelcontextprotocol/sdk/client/stdio.js";

let client: Client | null = null;

export async function getClient(): Promise<Client> {{
  if (client) return client;
  const transport = new StdioClientTransport({{
    command: "{}",
    args: ["mcp", "serve"],
    env: Object.fromEntries(
      Object.entries(process.env).filter(
        (entry): entry is [string, string] => entry[1] !== undefined,
      ),
    ),
  }});
  client = new Client({{ name: "code-mode-sdk", version: "1.0.0" }});
  await client.connect(transport);
  return client;
}}

export async function execute(params: Record<string, unknown>): Promise<unknown> {{
  const c = await getClient();
  const result = await c.callTool({{ name: "execute", arguments: params }});
  const text = (result.content as Array<{{ type: string; text: string }}>)
    .filter((c) => c.type === "text")
    .map((c) => c.text)
    .join("");
  try {{ return JSON.parse(text); }} catch {{ return text; }}
}}

export async function closeAll(): Promise<void> {{
  if (client) {{
    await client.close();
    client = null;
  }}
}}
"#,
        exe_str
    );
    std::fs::write(sdk_dir.join("client.ts"), &client_ts)?;
    println!("  wrote sdk/client.ts");

    // Write per-server directories
    let mut index_exports = Vec::new();
    let mut manifest_tools = Vec::new();

    for (server_name, discovery) in servers {
        let server_dir = sdk_dir.join(server_name);
        std::fs::create_dir_all(&server_dir).with_context(|| {
            format!(
                "failed to create server directory: {}",
                server_dir.display()
            )
        })?;

        // Write INSTRUCTIONS.md
        let instructions = discovery
            .instructions
            .as_deref()
            .unwrap_or("No instructions provided by this server.");
        std::fs::write(
            server_dir.join("INSTRUCTIONS.md"),
            format!("# {server_name}\n\n{instructions}\n"),
        )?;
        println!("  wrote sdk/{server_name}/INSTRUCTIONS.md");

        // Write one file per tool
        let mut server_exports = Vec::new();

        for tool in &discovery.tools {
            let tool_name = tool.name.as_ref();
            let input_schema_value =
                serde_json::to_value(&*tool.input_schema).unwrap_or(serde_json::json!({}));
            let ts_args = generate_ts_args(&input_schema_value);
            let fn_name = to_camel_case(tool_name);

            let description = tool
                .description
                .as_ref()
                .map(|d| d.to_string())
                .unwrap_or_default();

            // Build the tool file
            let mut tool_code = String::from("import { execute } from \"../client.js\";\n");

            // Add description as JSDoc comment
            if !description.is_empty() {
                tool_code.push_str(&format!("\n/**\n * {description}\n */\n"));
            } else {
                tool_code.push('\n');
            }

            if ts_args.is_empty() {
                tool_code.push_str(&format!(
                    "export async function {fn_name}(): Promise<unknown> {{\n  return execute({{ type: \"{server_name}.{tool_name}\" }});\n}}\n"
                ));
            } else {
                tool_code.push_str(&format!(
                    "export async function {fn_name}({ts_args}): Promise<unknown> {{\n  return execute({{ type: \"{server_name}.{tool_name}\", ...args }});\n}}\n"
                ));
            }

            let file_name = format!("{tool_name}.ts");
            std::fs::write(server_dir.join(&file_name), &tool_code)?;
            println!("  wrote sdk/{server_name}/{file_name}");

            server_exports.push(format!("export {{ {fn_name} }} from \"./{tool_name}.js\";"));

            manifest_tools.push(DiscoveredTool {
                server: server_name.clone(),
                name: tool_name.to_string(),
                description: Some(description),
                input_schema: input_schema_value,
            });
        }

        // Write per-server index.ts
        let server_index = server_exports.join("\n") + "\n";
        std::fs::write(server_dir.join("index.ts"), &server_index)?;
        println!("  wrote sdk/{server_name}/index.ts");

        let camel = to_camel_case(server_name);
        index_exports.push(format!(
            "export * as {camel} from \"./{server_name}/index.js\";"
        ));
    }

    // Write top-level index.ts
    index_exports.push("export { execute, closeAll } from \"./client.js\";".into());
    let index_ts = index_exports.join("\n") + "\n";
    std::fs::write(sdk_dir.join("index.ts"), &index_ts)?;
    println!("  wrote sdk/index.ts");

    // Write manifest.json
    let manifest = serde_json::to_string_pretty(&manifest_tools)?;
    std::fs::write(sdk_dir.join("manifest.json"), &manifest)?;
    println!("  wrote sdk/manifest.json");

    // Write tsconfig.json
    let tsconfig = serde_json::json!({
        "compilerOptions": {
            "target": "ES2022",
            "module": "ESNext",
            "moduleResolution": "bundler",
            "strict": true,
            "esModuleInterop": true,
            "skipLibCheck": true,
            "outDir": "./dist"
        },
        "include": ["./**/*.ts"]
    });
    std::fs::write(
        sdk_dir.join("tsconfig.json"),
        serde_json::to_string_pretty(&tsconfig)?,
    )?;
    println!("  wrote sdk/tsconfig.json");

    Ok(())
}

pub async fn execute(config: &Config, base_dir: Option<&Path>) -> Result<()> {
    if config.servers.is_empty() {
        println!("no MCP servers configured — nothing to generate");
        return Ok(());
    }

    println!("found {} configured server(s)", config.servers.len());

    // Discover tools from each server
    let mut all_servers = Vec::new();

    for (name, entry) in &config.servers {
        if entry.transport != "stdio" {
            println!(
                "  skipping {} (transport: {}, only stdio supported for discovery)",
                name, entry.transport
            );
            continue;
        }

        println!("  discovering tools from: {}", name);
        match discover_server(name, entry).await {
            Ok(result) => {
                println!("    found {} tool(s)", result.tools.len());
                all_servers.push((name.clone(), result));
            }
            Err(e) => {
                eprintln!("    error discovering tools from {}: {e:#}", name);
            }
        }
    }

    if all_servers.is_empty() {
        println!("no tools discovered — nothing to generate");
        return Ok(());
    }

    // CLI --base-dir > config base_dir > default .code-mode
    let default_base = PathBuf::from(".code-mode");
    let base = base_dir
        .map(Path::to_path_buf)
        .or_else(|| config.base_dir.clone())
        .unwrap_or(default_base);
    let base = base.as_path();

    println!("generating SDK at: {}", base.display());
    generate(base, &all_servers)?;

    // Run bun install in base dir
    println!("running bun install in {}", base.display());
    let status = tokio::process::Command::new("bun")
        .arg("install")
        .current_dir(base)
        .status()
        .await
        .context("failed to run bun install")?;

    if !status.success() {
        anyhow::bail!("bun install failed with exit code: {}", status);
    }

    println!("SDK generation complete");
    Ok(())
}
