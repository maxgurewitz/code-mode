import { mkdir, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const fixtureDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(fixtureDir, "../..");
const codexHome = path.join(repoRoot, ".tmp", "openai-codex-home-fixture");
const manifestPath = path.join(repoRoot, "openai-codex-mcp", "Cargo.toml");
const accountId = "acct_fixture";
const accessToken = makeJwt({
  exp: Math.floor(Date.now() / 1000) + 60 * 60,
  "https://api.openai.com/auth": {
    chatgpt_account_id: accountId,
  },
});

async function main() {
  await seedCodexHome();

  const backend = await startFakeCodexBackend();
  process.env.OPENAI_CODEX_MCP_MANIFEST_PATH = manifestPath;
  process.env.OPENAI_CODEX_HOME = codexHome;
  process.env.OPENAI_CODEX_MCP_BASE_URL = backend.baseUrl;

  const { closeAll, openaiCodex } = await import("../../.tmp/.code-mode/sdk/index.js");

  try {
    const infer = await openaiCodex.codexInfer({
      prompt: "Say fixture hello",
    });
    if (infer.text !== "Fixture hello") {
      throw new Error(
        `expected codexInfer().text to return "Fixture hello", got ${JSON.stringify(infer)}`
      );
    }

    const response = await openaiCodex.codexResponse({
      input: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "Return raw events" }],
        },
      ],
      include_raw_events: true,
    });
    if (response.output_text !== "Fixture hello") {
      throw new Error(
        `expected codexResponse().output_text to return "Fixture hello", got ${JSON.stringify(
          response
        )}`
      );
    }
    if (!Array.isArray(response.raw_events) || response.raw_events.length === 0) {
      throw new Error(`expected raw_events to be present, got ${JSON.stringify(response)}`);
    }

    console.log(
      JSON.stringify({
        infer,
        response_id: response.response_id,
        raw_events: response.raw_events.length,
      })
    );
  } finally {
    await closeAll();
    await backend.close();
  }
}

async function seedCodexHome() {
  await rm(codexHome, { recursive: true, force: true });
  await mkdir(codexHome, { recursive: true });
  await writeFile(
    path.join(codexHome, "auth.json"),
    JSON.stringify(
      {
        auth_mode: "chatgpt",
        last_refresh: "2026-03-24T08:56:59.779225Z",
        tokens: {
          access_token: accessToken,
          id_token: makeJwt({ email: "fixtures@example.com" }),
          account_id: accountId,
        },
      },
      null,
      2
    )
  );
}

async function startFakeCodexBackend() {
  const server = createServer((req, res) => {
    if (req.url === "/codex/responses" && req.method === "POST") {
      const auth = req.headers.authorization;
      if (auth !== `Bearer ${accessToken}`) {
        res.writeHead(401, { "content-type": "text/plain" });
        res.end("unauthorized");
        return;
      }

      res.writeHead(200, {
        "content-type": "text/event-stream; charset=utf-8",
        connection: "close",
      });
      res.write(
        `event: response.output_text.delta\n` +
          `data: ${JSON.stringify({
            type: "response.output_text.delta",
            item_id: "msg_fixture",
            output_index: 0,
            content_index: 0,
            delta: "Fixture hello",
          })}\n\n`
      );
      res.write(
        `event: response.completed\n` +
          `data: ${JSON.stringify({
            type: "response.completed",
            response: {
              id: "resp_fixture",
              model: "gpt-5.4-mini",
              status: "completed",
              usage: {
                input_tokens: 5,
                output_tokens: 2,
              },
              output: [
                {
                  type: "message",
                  id: "msg_fixture",
                  role: "assistant",
                  content: [{ type: "output_text", text: "Fixture hello" }],
                },
              ],
            },
          })}\n\n`
      );
      res.end();
      return;
    }

    res.writeHead(404, { "content-type": "text/plain" });
    res.end("not found");
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve());
  });

  const address = server.address();
  if (!address || typeof address === "string") {
    throw new Error(`expected TCP address, got ${JSON.stringify(address)}`);
  }

  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    close: async () => await new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  };
}

function makeJwt(payload: Record<string, unknown>) {
  const header = Buffer.from(JSON.stringify({ alg: "none", typ: "JWT" })).toString("base64url");
  const body = Buffer.from(JSON.stringify(payload)).toString("base64url");
  return `${header}.${body}.`;
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
