# openai-inference-mcp

`openai-inference-mcp` is a small stdio MCP server that forwards raw OpenAI
inference requests directly to the OpenAI API with an API key.

It is intentionally thin:

- API key auth only
- direct `https://api.openai.com` calls by default
- no OAuth
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
api_key = "..."
log = "error"
```

If `api_key` is not set in config or `OPENAI_INFERENCE_MCP_API_KEY`, the server
falls back to `OPENAI_API_KEY`.

## Running

```bash
export OPENAI_API_KEY=your_key_here
openai-inference-mcp
```

## code-mode registration

```toml
[servers.openai-inference]
transport = "stdio"
command = "openai-inference-mcp"
env = { OPENAI_INFERENCE_MCP_LOG = "error", OPENAI_INFERENCE_MCP_API_KEY = "$OPENAI_API_KEY" }
```

