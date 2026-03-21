import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const server = new McpServer({
  name: "echo-env",
  version: "1.0.0",
});

server.registerTool(
  "get_env",
  {
    inputSchema: { name: z.string() },
    outputSchema: { value: z.string() },
  },
  async ({ name }) => {
    const structuredContent = {
      value: process.env[name] !== undefined ? process.env[name] : `<unset: ${name}>`,
    };
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify(structuredContent),
        },
      ],
      structuredContent,
    };
  }
);

server.registerTool(
  "get_arg",
  {
    inputSchema: { index: z.number().int().nonnegative() },
    outputSchema: { value: z.string() },
  },
  async ({ index }) => {
    const structuredContent = {
      value:
        process.argv[2 + index] !== undefined
          ? process.argv[2 + index]
          : `<unset-arg: ${index}>`,
    };
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify(structuredContent),
        },
      ],
      structuredContent,
    };
  }
);

const transport = new StdioServerTransport();
await server.connect(transport);
