# Code Mode

`code-mode` is a cli which serves two purposes:

- Serves as a central gateway for aggregating mcp servers into two context efficient interfaces: `search`, and `execute`.
- Generates typescript code bindings for MCP servers's tools, allowing coding agents to access them programmatically.

This project is a reproduction of the "serverside code mode" pattern as initially [proposed by cloud flare](https://blog.cloudflare.com/code-mode-mcp/).
