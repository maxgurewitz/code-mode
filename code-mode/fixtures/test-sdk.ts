import { helloWorld, closeAll } from "../../.code-mode/sdk/index.js";

async function main() {
  const result = await helloWorld.echo({ message: "world" });
  console.log(JSON.stringify(result));
  await closeAll();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
