# Fixtures

Test fixtures for manual QA of the `code-mode` crate.

## Quick start

From the repo root:

```sh
cargo run -- mcp generate --config code-mode/fixtures/code-mode.toml
```

This reads the fixture config, discovers tools from the configured MCP servers, and generates the SDK into `.tmp/.code-mode/sdk/`. The `.tmp/` prefix keeps test output out of version control.

## What's here

- **code-mode.toml** — Test config. Sets `base_dir = ".tmp/.code-mode"` and registers the `hello-world` MCP server (via `npx mcp-hello-world`).
- **echo-env-server.mjs** — A small MCP server that echoes environment variables back. Used to test env/secrets plumbing.
- **test-sdk.ts** / **test-env.ts** — TypeScript scripts that import the generated SDK and exercise downstream tool calls.
- **package.json** — Dependencies for the fixture MCP servers and test scripts.

## Running the test scripts

After generating the SDK:

```sh
cd code-mode/fixtures
npx tsx test-sdk.ts
npx tsx test-env.ts
```
