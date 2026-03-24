import { closeAll, helloWorld, system } from "../../.tmp/.code-mode/sdk/index.js";

async function main() {
  const result = await helloWorld.echo({ message: "logs" });
  if (result.text !== "You said: logs") {
    throw new Error(
      `expected helloWorld.echo().text to return \"You said: logs\", got ${JSON.stringify(result)}`
    );
  }

  const current = await system.logsCurrent({ server: "hello-world" });
  const session = current.sessions.find((entry) => entry.server === "hello-world");
  if (!session) {
    throw new Error(`expected an active hello-world log session, got ${JSON.stringify(current)}`);
  }
  if (!session.active) {
    throw new Error(`expected hello-world log session to be active, got ${JSON.stringify(session)}`);
  }
  if (!session.log_path.endsWith(".stderr.log")) {
    throw new Error(`expected stderr log path, got ${JSON.stringify(session)}`);
  }

  const chunk = await system.logsRead({ session_id: session.session_id, max_bytes: 8192 });
  if (!chunk.text.includes("hello-world echo called: logs")) {
    throw new Error(`expected captured stderr output, got ${JSON.stringify(chunk)}`);
  }
  if (!chunk.log_path.endsWith(".stderr.log")) {
    throw new Error(`expected stderr log path on chunk, got ${JSON.stringify(chunk)}`);
  }

  console.log(
    JSON.stringify({
      session_id: session.session_id,
      log_path: chunk.log_path,
      eof: chunk.eof,
    })
  );
  await closeAll();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
