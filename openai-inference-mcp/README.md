# openai-inference-mcp

`openai-inference-mcp` is a small stdio MCP server that forwards raw OpenAI
inference requests directly to the OpenAI API.

It is intentionally thin:

- supports `api_key` auth from `OPENAI_INFERENCE_MCP_API_KEY` or `OPENAI_API_KEY`
- supports `oauth` auth from `CODEX_HOME/auth.json` or `~/.codex/auth.json`
- direct `https://api.openai.com` calls by default
- no Codex backend
- no gateway-specific translation layer

## Tools

- `responses_create` -> `POST /v1/responses`
- `chat_completions_create` -> `POST /v1/chat/completions`
- `embeddings_create` -> `POST /v1/embeddings`

Each tool accepts a single `body` field containing the raw JSON request body and
returns the raw JSON response body.

Streaming requests are rejected for now. Pass non-streaming request bodies.

## Config

By default the binary reads `~/.config/openai-inference-mcp/config.toml`.

You can also override fields with `OPENAI_INFERENCE_MCP_*` environment
variables.

```toml
base_url = "https://api.openai.com"
timeout_ms = 120000
user_agent = "openai-inference-mcp/0.1.0"
auth_mode = "api_key"
api_key = "..."
log = "error"
```

`auth_mode` can also be overridden with `--auth-mode oauth` or `--auth-mode api_key`.

If `api_key` is not set in config or `OPENAI_INFERENCE_MCP_API_KEY`, the server
falls back to `OPENAI_API_KEY`.

## Running

```bash
export OPENAI_API_KEY=your_key_here
openai-inference-mcp
```

Or, if Codex CLI is already logged in:

```bash
openai-inference-mcp --auth-mode oauth
```

If OAuth credentials are missing or expired, the server exits with an error
telling you to run:

```bash
codex login
```

## code-mode registration

```toml
[servers.openai-inference]
transport = "stdio"
command = "openai-inference-mcp"
env = { OPENAI_INFERENCE_MCP_LOG = "error", OPENAI_INFERENCE_MCP_API_KEY = "$OPENAI_API_KEY" }
```

Or configure OAuth explicitly:

```toml
[servers.openai-inference]
transport = "stdio"
command = "openai-inference-mcp"
env = { OPENAI_INFERENCE_MCP_LOG = "error", OPENAI_INFERENCE_MCP_AUTH_MODE = "oauth" }
```
