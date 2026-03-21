import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const server = new McpServer({
  name: "hello-world",
  version: "1.0.0",
});

server.tool("echo", { message: z.string() }, async ({ message }) => {
  return {
    content: [
      {
        type: "text",
        text: `You said: ${message}`,
      },
    ],
  };
});

const transport = new StdioServerTransport();
await server.connect(transport);
