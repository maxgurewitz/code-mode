# openai-codex-mcp

`openai-codex-mcp` is a local stdio MCP server that:

- supports `oauth` auth from `CODEX_HOME/auth.json` or `~/.codex/auth.json`
- supports `api_token` auth from `OPENAI_CODEX_MCP_API_TOKEN`
- never writes or refreshes those credentials
- tells the user to run `codex login` if OAuth credentials are missing or expired
- routes `oauth` requests to the experimental ChatGPT Codex backend
- routes `api_token` requests to the OpenAI Responses API
- exposes two MCP tools: `codex_infer` and `codex_response`

## Config

By default the binary reads `~/.config/openai-codex-mcp/config.toml`.

You can also override fields with `OPENAI_CODEX_MCP_*` environment variables or pass `--config /path/to/config.toml`.

```toml
# oauth default
base_url = "https://chatgpt.com/backend-api"
model = "gpt-5.4-mini"
timeout_ms = 120000
user_agent = "openai-codex-mcp/0.1.0"
auth_mode = "oauth"
# Only used when auth_mode = "api_token".
# If you leave base_url/model at their oauth defaults, api_token mode
# automatically switches to:
# base_url = "https://api.openai.com"
# model = "gpt-5-mini"
# api_token = "..."
log = "error"
```

`auth_mode` can also be overridden with `--auth-mode oauth` or `--auth-mode api_token`.

## Running

If Codex CLI is already logged in, the default OAuth mode works as-is:

```bash
openai-codex-mcp
```

If you want to use a raw bearer token instead, set:

```bash
export OPENAI_CODEX_MCP_AUTH_MODE=api_token
export OPENAI_CODEX_MCP_API_TOKEN=your_token_here
openai-codex-mcp
```

That mode uses `https://api.openai.com/v1/responses` by default.

If OAuth credentials are missing, the server exits with an error telling you to run:

```bash
codex login
```

## code-mode registration

Add this downstream server to `code-mode.toml`:

```toml
[servers.openai-codex]
transport = "stdio"
command = "openai-codex-mcp"
env = { OPENAI_CODEX_MCP_LOG = "error" }
```

Or configure token auth explicitly:

```toml
[servers.openai-codex]
transport = "stdio"
command = "openai-codex-mcp"
args = ["--auth-mode", "api_token"]
env = { OPENAI_CODEX_MCP_API_TOKEN = "$OPENAI_CODEX_TOKEN", OPENAI_CODEX_MCP_LOG = "error" }
```

Then call:

- `openai-codex.codex_infer`
- `openai-codex.codex_response`

## Notes

- `oauth` mode targets `https://chatgpt.com/backend-api/codex/responses`.
- `api_token` mode targets `https://api.openai.com/v1/responses`.
- `api_token` is the preferred non-OAuth name because the server sends it as a bearer token; `token` is accepted as an alias for the mode value.
