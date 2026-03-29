import path from "node:path";
import { fileURLToPath } from "node:url";

const fixtureDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(fixtureDir, "../..");
const generatedSdkPath = path.join(repoRoot, ".tmp", ".code-mode", "sdk", "index.js");
type ResponsesCreateResult = {
  id?: string;
  model?: string;
  output_text?: string;
  output?: Array<{
    content?: Array<{
      text?: string;
    }>;
  }>;
};

async function main() {
  const authMode = process.env.OPENAI_INFERENCE_MCP_AUTH_MODE?.toLowerCase();
  const usesOauth = authMode === "oauth";
  if (!usesOauth && !process.env.OPENAI_API_KEY && !process.env.OPENAI_INFERENCE_MCP_API_KEY) {
    throw new Error(
      "set OPENAI_API_KEY, OPENAI_INFERENCE_MCP_API_KEY, or OPENAI_INFERENCE_MCP_AUTH_MODE=oauth before running this fixture"
    );
  }

  const model = process.env.OPENAI_INFERENCE_FIXTURE_MODEL ?? "gpt-5-mini";
  const { closeAll, openaiInference } = await import(generatedSdkPath);

  try {
    const response = (await openaiInference.responsesCreate({
      body: {
        model,
        input: "Reply with exactly fixture-ok",
        instructions: "Return exactly fixture-ok.",
        reasoning: { effort: "low" },
        max_output_tokens: 64,
        store: false,
      },
    })) as ResponsesCreateResult;

    const text =
      response.output_text ??
      response.output?.flatMap((item) => item.content ?? []).map((item) => item.text ?? "").join("");

    if (text?.trim() !== "fixture-ok") {
      throw new Error(
        `expected output_text to be fixture-ok, got ${JSON.stringify(response, null, 2)}`
      );
    }

    console.log(
      JSON.stringify({
        id: response.id,
        model: response.model,
        output_text: text,
      })
    );
  } finally {
    await closeAll();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
