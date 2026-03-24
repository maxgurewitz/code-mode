mod schema_to_ts;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rmcp::{
    ServiceExt,
    model::Tool,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use schema_to_ts::{
    Options as SchemaToTsOptions, compile_named_declaration, schema_accepts_no_args,
};
use serde::{Deserialize, Serialize};

use super::builtin;
use super::config::{Config, ServerEntry};

/// A discovered tool from a downstream MCP server.
#[derive(Debug, Serialize, Deserialize)]
struct DiscoveredTool {
    server: String,
    name: String,
    description: Option<String>,
    input_schema: serde_json::Value,
    output_schema: Option<serde_json::Value>,
}

/// Result of discovering a server: its instructions and tools.
struct DiscoveryResult {
    instructions: Option<String>,
    tools: Vec<Tool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderedFile {
    path: PathBuf,
    contents: String,
}

#[derive(Debug, Default)]
struct RenderedSdk {
    files: Vec<RenderedFile>,
    cleanup_paths: Vec<PathBuf>,
}

impl RenderedSdk {
    fn push_file(&mut self, path: impl Into<PathBuf>, contents: impl Into<String>) {
        self.files.push(RenderedFile {
            path: path.into(),
            contents: contents.into(),
        });
    }

    fn push_cleanup_path(&mut self, path: impl Into<PathBuf>) {
        self.cleanup_paths.push(path.into());
    }
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

fn to_pascal_case(s: &str) -> String {
    let camel = to_camel_case(s);
    let mut chars = camel.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn instructions_markdown(server_name: &str, instructions: Option<&str>) -> Option<String> {
    let instructions = instructions?;
    if instructions.trim().is_empty() {
        return None;
    }

    Some(format!("# {server_name}\n\n{instructions}\n"))
}

fn render_client_ts() -> String {
    format!(
        r#"import {{ Client }} from "@modelcontextprotocol/sdk/client/index.js";
import {{ StdioClientTransport }} from "@modelcontextprotocol/sdk/client/stdio.js";

let client: Client | null = null;

export async function getClient(): Promise<Client> {{
  if (client) return client;
  const transport = new StdioClientTransport({{
    command: "code-mode",
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

export async function execute<T = unknown>(params: Record<string, unknown>): Promise<T> {{
  const c = await getClient();
  const result = await c.callTool({{ name: "execute", arguments: params }});
  const structured = (result as {{ structuredContent?: T }}).structuredContent;
  if (structured !== undefined) return structured;
  const text = ((result.content ?? []) as Array<{{ type: string; text?: string }}>)
    .filter((c) => c.type === "text")
    .map((c) => c.text ?? "")
    .join("");
  try {{ return JSON.parse(text) as T; }} catch {{ return text as T; }}
}}

export async function closeAll(): Promise<void> {{
  if (client) {{
    await client.close();
    client = null;
  }}
}}
"#,
    )
}

fn render_tool_file(
    server_name: &str,
    tool: &Tool,
    schema_options: &SchemaToTsOptions,
) -> Result<(String, DiscoveredTool)> {
    let tool_name = tool.name.as_ref();
    let input_schema_value =
        serde_json::to_value(&*tool.input_schema).unwrap_or(serde_json::json!({}));
    let output_schema_value = tool
        .output_schema
        .as_ref()
        .and_then(|schema| serde_json::to_value(schema).ok());
    let fn_name = to_camel_case(tool_name);
    let type_name_prefix = to_pascal_case(tool_name);
    let args_type_name = format!("{type_name_prefix}Args");
    let result_type_name = format!("{type_name_prefix}Result");

    let description = tool
        .description
        .as_ref()
        .map(|d| d.to_string())
        .unwrap_or_default();

    let mut tool_code = String::from("import { execute } from \"../client.js\";\n");

    let args_decl = if schema_accepts_no_args(&input_schema_value) {
        None
    } else {
        Some(
            compile_named_declaration(&input_schema_value, &args_type_name, schema_options)
                .with_context(|| {
                    format!("failed to compile input schema for {server_name}.{tool_name}")
                })?,
        )
    };

    if let Some(args_decl) = &args_decl {
        tool_code.push('\n');
        tool_code.push_str(args_decl);
        tool_code.push('\n');
    }

    let result_decl = output_schema_value
        .as_ref()
        .map(|output_schema| {
            compile_named_declaration(output_schema, &result_type_name, schema_options)
                .with_context(|| {
                    format!("failed to compile output schema for {server_name}.{tool_name}")
                })
        })
        .transpose()?;

    let return_type = if let Some(result_decl) = &result_decl {
        tool_code.push('\n');
        tool_code.push_str(result_decl);
        tool_code.push('\n');
        result_type_name.clone()
    } else {
        "unknown".into()
    };

    if !description.is_empty() {
        tool_code.push_str(&format!("\n/**\n * {description}\n */\n"));
    } else {
        tool_code.push('\n');
    }

    let execute_call = if return_type == "unknown" {
        "execute".to_string()
    } else {
        format!("execute<{return_type}>")
    };

    if args_decl.is_some() {
        tool_code.push_str(&format!(
            "export async function {fn_name}(args: {args_type_name}): Promise<{return_type}> {{\n  return {execute_call}({{ type: \"{server_name}.{tool_name}\", ...args }});\n}}\n"
        ));
    } else {
        tool_code.push_str(&format!(
            "export async function {fn_name}(): Promise<{return_type}> {{\n  return {execute_call}({{ type: \"{server_name}.{tool_name}\" }});\n}}\n"
        ));
    }

    Ok((
        tool_code,
        DiscoveredTool {
            server: server_name.to_string(),
            name: tool_name.to_string(),
            description: Some(description),
            input_schema: input_schema_value,
            output_schema: output_schema_value,
        },
    ))
}

fn render_server_sdk(
    rendered: &mut RenderedSdk,
    server_name: &str,
    instructions: Option<&str>,
    tools: &[Tool],
    schema_options: &SchemaToTsOptions,
    manifest_tools: &mut Vec<DiscoveredTool>,
    include_in_manifest: bool,
) -> Result<String> {
    let instructions_path = PathBuf::from(format!("sdk/{server_name}/INSTRUCTIONS.md"));
    if let Some(instructions) = instructions_markdown(server_name, instructions) {
        rendered.push_file(&instructions_path, instructions);
    } else {
        rendered.push_cleanup_path(&instructions_path);
    }

    let mut server_exports = Vec::new();
    for tool in tools {
        let tool_name = tool.name.as_ref();
        let (tool_code, manifest_tool) = render_tool_file(server_name, tool, schema_options)?;
        let file_name = format!("sdk/{server_name}/{tool_name}.ts");
        rendered.push_file(&file_name, tool_code);
        server_exports.push(format!("export * from \"./{tool_name}.js\";"));

        if include_in_manifest {
            manifest_tools.push(manifest_tool);
        }
    }

    let server_index = server_exports.join("\n") + "\n";
    rendered.push_file(format!("sdk/{server_name}/index.ts"), server_index);

    let camel = to_camel_case(server_name);
    Ok(format!(
        "export * as {camel} from \"./{server_name}/index.js\";"
    ))
}

/// Render the TypeScript SDK into an in-memory file set.
fn render_sdk(servers: &[(String, DiscoveryResult)]) -> Result<RenderedSdk> {
    let mut rendered = RenderedSdk::default();

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
    rendered.push_file("package.json", serde_json::to_string_pretty(&package_json)?);
    rendered.push_file("sdk/client.ts", render_client_ts());

    let schema_options = SchemaToTsOptions::default();

    let mut index_exports = Vec::new();
    let mut manifest_tools = Vec::new();

    index_exports.push(render_server_sdk(
        &mut rendered,
        builtin::SYSTEM_SERVER_NAME,
        None,
        &builtin::tools(),
        &schema_options,
        &mut manifest_tools,
        false,
    )?);

    for (server_name, discovery) in servers {
        index_exports.push(render_server_sdk(
            &mut rendered,
            server_name,
            discovery.instructions.as_deref(),
            &discovery.tools,
            &schema_options,
            &mut manifest_tools,
            true,
        )?);
    }

    index_exports.push("export { execute, closeAll } from \"./client.js\";".into());
    let index_ts = index_exports.join("\n") + "\n";
    rendered.push_file("sdk/index.ts", index_ts);

    let manifest = serde_json::to_string_pretty(&manifest_tools)?;
    rendered.push_file("sdk/manifest.json", manifest);

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
    rendered.push_file(
        "sdk/tsconfig.json",
        serde_json::to_string_pretty(&tsconfig)?,
    );

    Ok(rendered)
}

fn write_rendered_sdk(base_dir: &Path, rendered: &RenderedSdk) -> Result<()> {
    std::fs::create_dir_all(base_dir).with_context(|| {
        format!(
            "failed to create SDK base directory: {}",
            base_dir.display()
        )
    })?;

    for file in &rendered.files {
        let output_path = base_dir.join(&file.path);
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory: {}", parent.display())
            })?;
        }
        std::fs::write(&output_path, &file.contents)
            .with_context(|| format!("failed to write file: {}", output_path.display()))?;
        println!("  wrote {}", file.path.display());
    }

    for path in &rendered.cleanup_paths {
        let output_path = base_dir.join(path);
        if output_path.exists() {
            std::fs::remove_file(&output_path).with_context(|| {
                format!("failed to remove stale file: {}", output_path.display())
            })?;
            println!("  removed {}", path.display());
        }
    }

    Ok(())
}

/// Generate the base directory structure and TypeScript SDK.
fn generate(base_dir: &Path, servers: &[(String, DiscoveryResult)]) -> Result<()> {
    let rendered = render_sdk(servers)?;
    write_rendered_sdk(base_dir, &rendered)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after UNIX_EPOCH")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "code-mode-generate-{name}-{}-{unique}",
                std::process::id()
            ));
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn server(name: &str, instructions: Option<&str>) -> (String, DiscoveryResult) {
        (
            name.to_string(),
            DiscoveryResult {
                instructions: instructions.map(str::to_owned),
                tools: Vec::new(),
            },
        )
    }

    fn tool_with_schemas(
        name: &str,
        description: &str,
        input_schema: serde_json::Value,
        output_schema: Option<serde_json::Value>,
    ) -> Tool {
        let input_schema = Arc::new(
            input_schema
                .as_object()
                .expect("input schema should be an object")
                .clone(),
        );
        let tool = Tool::new(name.to_string(), description.to_string(), input_schema);

        match output_schema {
            Some(output_schema) => tool.with_raw_output_schema(Arc::new(
                output_schema
                    .as_object()
                    .expect("output schema should be an object")
                    .clone(),
            )),
            None => tool,
        }
    }

    #[test]
    fn instructions_markdown_ignores_missing_and_blank_instructions() {
        assert_eq!(instructions_markdown("demo", None), None);
        assert_eq!(instructions_markdown("demo", Some("")), None);
        assert_eq!(instructions_markdown("demo", Some("   \n\t  ")), None);
    }

    fn rendered_files_map(rendered: &RenderedSdk) -> BTreeMap<String, String> {
        rendered
            .files
            .iter()
            .map(|file| {
                (
                    file.path.to_string_lossy().into_owned(),
                    file.contents.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn render_sdk_includes_instructions_when_present() -> Result<()> {
        let rendered = render_sdk(&[server("demo", Some("Use this server carefully."))])?;
        let files = rendered_files_map(&rendered);
        assert_eq!(
            files.get("sdk/demo/INSTRUCTIONS.md").map(String::as_str),
            Some("# demo\n\nUse this server carefully.\n")
        );
        assert_eq!(
            rendered.cleanup_paths,
            vec![PathBuf::from("sdk/system/INSTRUCTIONS.md")]
        );

        Ok(())
    }

    #[test]
    fn render_sdk_marks_blank_instructions_for_cleanup_without_file_io() -> Result<()> {
        let rendered = render_sdk(&[server("demo", Some("   \n"))])?;
        let files = rendered_files_map(&rendered);
        assert!(!files.contains_key("sdk/demo/INSTRUCTIONS.md"));
        assert_eq!(
            rendered.cleanup_paths,
            vec![
                PathBuf::from("sdk/system/INSTRUCTIONS.md"),
                PathBuf::from("sdk/demo/INSTRUCTIONS.md"),
            ]
        );

        Ok(())
    }

    #[test]
    fn write_rendered_sdk_removes_stale_instructions() -> Result<()> {
        let temp_dir = TestDir::new("removes-stale-instructions");
        let instructions_path = temp_dir.path().join("sdk/demo/INSTRUCTIONS.md");
        std::fs::create_dir_all(instructions_path.parent().expect("path has parent"))?;
        std::fs::write(&instructions_path, "stale instructions")?;

        let rendered = render_sdk(&[server("demo", Some("   \n"))])?;
        write_rendered_sdk(temp_dir.path(), &rendered)?;

        assert!(!instructions_path.exists());

        Ok(())
    }

    #[test]
    fn render_sdk_uses_output_schema_for_typed_return_values() -> Result<()> {
        let tools = vec![tool_with_schemas(
            "echo",
            "Echo a message back.",
            serde_json::json!({
                "type": "object",
                "required": ["message"],
                "properties": {
                    "message": { "type": "string" }
                }
            }),
            Some(serde_json::json!({
                "type": "object",
                "required": ["echoed"],
                "properties": {
                    "echoed": { "type": "string" },
                    "count": { "type": "integer" }
                }
            })),
        )];

        let rendered = render_sdk(&[(
            "demo".into(),
            DiscoveryResult {
                instructions: None,
                tools,
            },
        )])?;
        let files = rendered_files_map(&rendered);

        let content = files
            .get("sdk/demo/echo.ts")
            .expect("tool file should be rendered");
        assert!(content.contains("export interface EchoArgs {"));
        assert!(content.contains("message: string;"));
        assert!(content.contains("[k: string]: unknown;"));
        assert!(content.contains("export interface EchoResult {"));
        assert!(content.contains("count?: number;"));
        assert!(content.contains("echoed: string;"));
        assert!(
            content.contains(
                "export async function echo(args: EchoArgs): Promise<EchoResult> {\n  return execute<EchoResult>({ type: \"demo.echo\", ...args });\n}\n"
            )
        );

        let client = files
            .get("sdk/client.ts")
            .expect("client file should be rendered");
        assert!(client.contains("command: \"code-mode\""));
        assert!(client.contains("export async function execute<T = unknown>("));
        assert!(client.contains("structuredContent?: T"));

        Ok(())
    }

    #[test]
    fn render_sdk_emits_system_builtins_without_adding_them_to_manifest() -> Result<()> {
        let rendered = render_sdk(&[])?;
        let files = rendered_files_map(&rendered);

        let root_index = files
            .get("sdk/index.ts")
            .expect("root index should be rendered");
        assert!(root_index.contains("export * as system from \"./system/index.js\";"));

        let system_index = files
            .get("sdk/system/index.ts")
            .expect("system index should be rendered");
        assert!(system_index.contains("export * from \"./logs_current.js\";"));
        assert!(system_index.contains("export * from \"./logs_read.js\";"));

        let logs_current = files
            .get("sdk/system/logs_current.ts")
            .expect("system logs_current wrapper should be rendered");
        assert!(logs_current.contains("type: \"system.logs_current\""));

        let manifest = files
            .get("sdk/manifest.json")
            .expect("manifest should be rendered");
        assert!(!manifest.contains("\"server\": \"system\""));

        Ok(())
    }
}
