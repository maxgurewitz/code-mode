import { helloWorld, closeAll } from "../../.tmp/.code-mode/sdk/index.js";

async function main() {
  const result = await helloWorld.echo({ message: "world" });
  if (result !== "You said: world") {
    throw new Error(
      `expected helloWorld.echo() to return \"You said: world\", got ${JSON.stringify(result)}`
    );
  }
  console.log(JSON.stringify(result));
  await closeAll();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
