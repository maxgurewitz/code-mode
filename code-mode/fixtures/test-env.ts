import { echoEnv, closeAll } from "../../.code-mode/sdk/index.js";

async function main() {
  // TEST_SECRET is set in both .mcp.json ("from-mcp-json") and XDG secrets ("from-xdg").
  // .mcp.json should win.
  const secret = (await echoEnv.getEnv({ name: "TEST_SECRET" })) as string;
  console.log(`TEST_SECRET = ${JSON.stringify(secret)}`);
  if (secret !== "from-mcp-json") {
    throw new Error(
      `expected TEST_SECRET = "from-mcp-json", got ${JSON.stringify(secret)}`
    );
  }

  // XDG_ONLY_VAR is only in XDG secrets — should pass through.
  const xdgOnly = (await echoEnv.getEnv({ name: "XDG_ONLY_VAR" })) as string;
  console.log(`XDG_ONLY_VAR = ${JSON.stringify(xdgOnly)}`);
  if (xdgOnly !== "xdg-value") {
    throw new Error(
      `expected XDG_ONLY_VAR = "xdg-value", got ${JSON.stringify(xdgOnly)}`
    );
  }

  console.log("all env tests passed");
  await closeAll();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
