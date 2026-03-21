import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const server = new McpServer({
  name: "echo-env",
  version: "1.0.0",
});

server.tool("get_env", { name: z.string() }, async ({ name }) => {
  const value = process.env[name];
  return {
    content: [
      {
        type: "text",
        text: value !== undefined ? value : `<unset: ${name}>`,
      },
    ],
  };
});

server.tool("get_arg", { index: z.number().int().nonnegative() }, async ({ index }) => {
  const value = process.argv[2 + index];
  return {
    content: [
      {
        type: "text",
        text: value !== undefined ? value : `<unset-arg: ${index}>`,
      },
    ],
  };
});

const transport = new StdioServerTransport();
await server.connect(transport);
