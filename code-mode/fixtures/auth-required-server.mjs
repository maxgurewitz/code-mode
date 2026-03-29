const tool = {
  name: "create_issue",
  description: "Create a GitHub issue.",
  inputSchema: {
    type: "object",
    required: ["title"],
    properties: {
      title: { type: "string" },
    },
    additionalProperties: false,
  },
  outputSchema: {
    type: "object",
    required: ["id"],
    properties: {
      id: { type: "string" },
    },
    additionalProperties: false,
  },
};

let buffer = "";

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function handleRequest(message) {
  if (message.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        protocolVersion: message.params?.protocolVersion ?? "2025-03-26",
        capabilities: { tools: {} },
        serverInfo: {
          name: "auth-required-fixture",
          version: "1.0.0",
        },
        instructions: "Fixture server that always requires GitHub auth.",
      },
    });
    return;
  }

  if (message.method === "notifications/initialized") {
    return;
  }

  if (message.method === "tools/list") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      result: {
        tools: [tool],
      },
    });
    return;
  }

  if (message.method === "tools/call") {
    send({
      jsonrpc: "2.0",
      id: message.id,
      error: {
        code: -32603,
        message: "Connect GitHub to continue",
        data: {
          type: "auth_required",
          service: "github",
          reason: "missing_connection",
          message: "Connect GitHub to continue",
          url: "https://example.test/connect/github",
          retryable: true,
        },
      },
    });
    return;
  }

  send({
    jsonrpc: "2.0",
    id: message.id,
    error: {
      code: -32601,
      message: `Method not found: ${message.method}`,
    },
  });
}

process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;

  while (true) {
    const newlineIndex = buffer.indexOf("\n");
    if (newlineIndex === -1) {
      break;
    }

    const line = buffer.slice(0, newlineIndex).replace(/\r$/, "");
    buffer = buffer.slice(newlineIndex + 1);

    if (!line) {
      continue;
    }

    handleRequest(JSON.parse(line));
  }
});
