# Code Mode

`code-mode` is a cli which serves two purposes:

- Serves as a central gateway for aggregating mcp servers into two context efficient interfaces: `search`, and `execute`.
- Generates typescript code bindings for MCP servers's tools, allowing coding agents to access them programmatically.

This project is a reproduction of the "serverside code mode" pattern as initially [proposed by cloud flare](https://blog.cloudflare.com/code-mode-mcp/).

## Config

`code-mode` reads configuration from TOML.

- If you pass `--config /path/to/code-mode.toml`, only that file is used.
- Otherwise `code-mode` merges `~/.config/code-mode/code-mode.toml` with the nearest ancestor `code-mode.toml` discovered from the current directory.
- Local config wins over global config when the same top-level keys or server names are defined in both places.

The top-level schema is:

```toml
base_dir = ".code-mode"

[servers.<name>]
transport = "stdio" # optional, defaults to "stdio"

# stdio transport
command = "node"
args = ["./path/to/server.mjs"]
env = { API_KEY = "$OPENAI_API_KEY" }

# http / sse transport
url = "https://example.com/mcp"
headers = { Authorization = "Bearer $API_TOKEN" }
```

### Top-Level Fields

- `base_dir` is optional and controls where `mcp generate` writes the generated SDK. If omitted, the default is `.code-mode`.
- `servers` is a map keyed by server name. Each entry defines one downstream MCP server.

### Server Entry Schema

Every server entry supports these fields:

- `transport`: optional string, one of `stdio`, `http`, or `sse`. Defaults to `stdio`.
- `command`: string, required for `stdio`, invalid for `http` and `sse`.
- `args`: optional array of strings, only used for `stdio`.
- `env`: optional string-to-string map, only used for `stdio`.
- `url`: string, required for `http` and `sse`, invalid for `stdio`.
- `headers`: optional string-to-string map, used for `http` and `sse`.

Validation rules:

- `stdio` servers must set `command` and must not set `url`.
- `http` and `sse` servers must set `url` and must not set `command`.
- Any other `transport` value is rejected.

### Env Var Expansion

String interpolation is applied after config files are merged. Expansion currently applies to:

- `base_dir`
- `command`
- every item in `args`
- every value in `env`
- `url`
- every value in `headers`

Supported patterns:

- `$NAME`
- `${NAME}`
- `$$` for a literal `$`

Behavior details:

- Unknown variables expand to an empty string.
- Only simple shell-variable references are expanded.
- Shell features such as command substitution like `$(...)` are not executed.
- Unsupported `${...}` forms are left as-is.
- This is only string interpolation. After expansion, `code-mode` launches downstream servers normally using the resulting `command`, `args`, and `env` values.
- This makes it easy to remap variables, for example `env = { API_KEY = "$OPENAI_API_KEY" }`, or pass them through as arguments, for example `args = ["$LOCAL_MCP_SERVER"]`.

Variable lookup comes from:

- the current process environment

`code-mode` does not inspect shell startup files. If you want a value from something like `.zshrc` or `.bashrc` to be available for interpolation, it needs to already be present in the environment of the `code-mode` process.

When `code-mode` is launched through the generated SDK client, that child process inherits the parent script's environment, so exported variables continue to work there too.

Example:

```toml
base_dir = "$CODE_MODE_OUTPUT"

[servers.local-tools]
command = "node"
args = ["$LOCAL_MCP_SERVER", "--project", "${PROJECT_ROOT}"]
env = { API_KEY = "$OPENAI_API_KEY", LOG_LEVEL = "$$debug" }
```
