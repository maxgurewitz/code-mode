import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const server = new McpServer({
  name: "hello-world",
  version: "1.0.0",
});

server.registerTool(
  "echo",
  {
    inputSchema: { message: z.string() },
    outputSchema: { text: z.string() },
  },
  async ({ message }) => {
    const structuredContent = { text: `You said: ${message}` };
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
