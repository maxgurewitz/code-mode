# openai-codex-mcp

`openai-codex-mcp` is a local stdio MCP server that:

- reads OpenAI Codex credentials from `CODEX_HOME/auth.json` or `~/.codex/auth.json`
- never writes or refreshes those credentials
- tells the user to run `codex login` if credentials are missing or expired
- calls the experimental ChatGPT Codex backend
- exposes two MCP tools: `codex_infer` and `codex_response`

## Config

By default the binary reads `~/.config/openai-codex-mcp/config.toml`.

You can also override fields with `OPENAI_CODEX_MCP_*` environment variables or pass `--config /path/to/config.toml`.

```toml
base_url = "https://chatgpt.com/backend-api"
model = "gpt-5.4-mini"
timeout_ms = 120000
user_agent = "openai-codex-mcp/0.1.0"
log = "error"
```

## Running

If Codex CLI is already logged in:

```bash
openai-codex-mcp
```

If credentials are missing, the server exits with an error telling you to run:

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

Then call:

- `openai-codex.codex_infer`
- `openai-codex.codex_response`

## Notes

- The backend adapter is experimental and targets `https://chatgpt.com/backend-api/codex/responses`.
- Authentication is read-only from the Codex CLI homedir; there is no separate login or auth storage in this crate.
