import { echoEnv, closeAll } from "../../.tmp/.code-mode/sdk/index.js";

async function main() {
  const secret = (await echoEnv.getEnv({ name: "TEST_SECRET" })) as string;
  console.log(`TEST_SECRET = ${JSON.stringify(secret)}`);
  if (secret !== "from-process-env") {
    throw new Error(
      `expected TEST_SECRET = "from-process-env", got ${JSON.stringify(secret)}`
    );
  }

  const braced = (await echoEnv.getEnv({ name: "BRACED_SECRET" })) as string;
  console.log(`BRACED_SECRET = ${JSON.stringify(braced)}`);
  if (braced !== "from-braced-process-env") {
    throw new Error(
      `expected BRACED_SECRET = "from-braced-process-env", got ${JSON.stringify(braced)}`
    );
  }

  const arg = (await echoEnv.getArg({ index: 0 })) as string;
  console.log(`ARG0 = ${JSON.stringify(arg)}`);
  if (arg !== "--from-process-env-flag") {
    throw new Error(
      `expected ARG0 = "--from-process-env-flag", got ${JSON.stringify(arg)}`
    );
  }

  console.log("all env tests passed");
  await closeAll();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
