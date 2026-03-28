# Fixtures

Test fixtures for manual QA of the `code-mode` crate.

## Quick start

From the repo root:

```sh
cargo run -- mcp generate --config code-mode/fixtures/code-mode.toml
```

This reads the fixture config, discovers tools from the configured MCP servers, and generates the SDK into `.tmp/.code-mode/sdk/`. The `.tmp/` prefix keeps test output out of version control.

The fixture config uses environment-variable interpolation for both server `args` and `env`. To exercise that path, export the variables referenced by `code-mode.toml` before running the fixture commands.

## What's here

- **code-mode.toml** — Test config. Sets `base_dir = ".tmp/.code-mode"` and registers local MCP servers whose `args` and `env` values are interpolated from environment variables.
- **hello-world-server.mjs** — A small MCP server that provides the `hello-world.echo` tool used by the SDK fixture test.
- **echo-env-server.mjs** — A small MCP server that echoes environment variables and startup args back. Used to test interpolation plumbing.
- **test-sdk.ts** / **test-env.ts** / **test-openai-codex.ts** / **test-openai-inference.ts** — TypeScript scripts that import the generated SDK and exercise downstream tool calls.
- **package.json** — Dependencies for the fixture MCP servers and test scripts.

## Running the test scripts

After generating the SDK and installing the fixture dependencies:

```sh
cd code-mode/fixtures
bun install
bun run typecheck
bun run test-sdk.ts
bun run test-env.ts
bun run test-openai-codex.ts
bun run test-openai-inference.ts
```

## OpenAI Codex fixture env

The `openai-codex-mcp` fixture entry uses the new Rust crate as a downstream stdio server. Before generating the SDK or running `test-openai-codex.ts`, export:

```sh
export OPENAI_CODEX_MCP_MANIFEST_PATH="$PWD/openai-codex-mcp/Cargo.toml"
```

`test-openai-codex.ts` will create a fake `CODEX_HOME` under `.tmp/`, seed `auth.json`, start a fake local Codex backend, and set the remaining env vars before it calls the generated SDK.

## OpenAI inference fixture env

The `openai-inference-mcp` fixture entry uses the direct OpenAI MCP crate. Before generating the SDK, export:

```sh
export OPENAI_INFERENCE_MCP_MANIFEST_PATH="$PWD/openai-inference-mcp/Cargo.toml"
```

For a live smoke test, also export one of:

```sh
export OPENAI_API_KEY="..."
# or
export OPENAI_INFERENCE_MCP_API_KEY="..."
```

Then run:

```sh
cd code-mode/fixtures
bun run test-openai-inference.ts
```
