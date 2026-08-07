// judge/trace_provider.js
//
// SKELETON — not yet exercised end-to-end. Written now so promptfooconfig.yaml is a complete,
// structurally valid config rather than a stub with a dangling provider reference; wiring this
// against REAL captured traces is report §8 step 10 (a later ticket), once the ai_eval.rs driver
// (step 8) and the golden-trace capture (step 9) both exist. Do not `npm install promptfoo` or
// attempt to run this on this box as part of this PR — no network installs during a live event
// (see README "Status").
//
// Promptfoo custom providers are plain modules exporting a class with an `id()` and a `callApi()`
// method (https://www.promptfoo.dev/docs/providers/custom-api/). This provider does NOT call any
// model — it is a pass-through that loads the PRE-CAPTURED trace for one case (never drives a
// live candidate) and reshapes it into the flat text promptfooconfig.yaml's rubric prompts grade.
//
// Expected trace JSON shape, written by the future ai_eval.rs driver to
// scripts/dev/ai-eval/traces/<caseId>.json (and, once captured, to golden/<caseId>.json for the
// claude-opus-4-6 reference — same shape, different directory):
//
// {
//   "caseId": "ba-01-verbatim-single-verse-seb",
//   "slice": "bible-authoring",
//   "candidate": { "model": "qwen3-8b", "apiUrl": "http://10.77.8.134:8787/v1" },
//   "userMessage": "...",
//   "conversation": [
//     { "role": "system", "content": "..." },
//     { "role": "user", "content": "..." },
//     { "role": "assistant", "content": null, "toolCalls": [ { "id": "...", "type": "function",
//         "function": { "name": "load_bible_verses", "arguments": "{...}" } } ] },
//     { "role": "tool", "toolCallId": "...", "name": "load_bible_verses", "content": "[...]" },
//     { "role": "assistant", "content": "Vytvoril som prezentáciu ..." }
//   ],
//   "toolCalls": [
//     { "iteration": 0, "tool": "load_bible_verses", "arguments": { "...": "..." },
//       "result": { "...": "..." } },
//     { "iteration": 1, "tool": "create_bible_presentation", "arguments": { "...": "..." },
//       "result": { "id": "...", "name": "...", "slide_count": 1 } }
//   ],
//   "finalResponse": "...",
//   "iterations": 2,
//   "durationMs": 18342,
//   "layer1": {
//     "schemaValid": true,
//     "validationErrors": [],
//     "verbatimVerses": [ { "ref": "Ján 3:16", "translation": "SEB", "pass": true } ],
//     "deleteGate": "n/a",
//     "sequencingOk": true
//   }
// }
//
// vars.caseId selects which traces/<caseId>.json to load. The provider returns `output` as a
// compact text rendering (sermon input, ordered tool calls with args/results, final reply) that
// the llm-rubric assertion in promptfooconfig.yaml grades against each rubric file in
// judge/rubrics/*.md — it is NEVER used to generate a NEW response, only to re-present a captured
// one for judging.

const fs = require("fs");
const path = require("path");

class TraceProvider {
  id() {
    return "presenter-ai-eval-trace-provider";
  }

  async callApi(prompt, context) {
    const caseId = context && context.vars && context.vars.caseId;
    if (!caseId) {
      throw new Error(
        "trace_provider.js: vars.caseId is required (set per-test in promptfooconfig.yaml / tests_from_traces.js)"
      );
    }

    const tracesDir = path.join(__dirname, "..", "traces");
    const tracePath = path.join(tracesDir, `${caseId}.json`);
    if (!fs.existsSync(tracePath)) {
      throw new Error(
        `trace_provider.js: no trace at ${tracePath} — run the drive stage first (ai_eval.rs, not yet built; see report §8 step 8). This provider NEVER falls back to calling a live model.`
      );
    }

    const trace = JSON.parse(fs.readFileSync(tracePath, "utf8"));
    const toolLines = (trace.toolCalls || [])
      .map(
        (tc) =>
          `  [${tc.iteration}] ${tc.tool}(${JSON.stringify(tc.arguments)}) -> ${JSON.stringify(
            tc.result
          )}`
      )
      .join("\n");

    const rendered = [
      `USER MESSAGE:\n${trace.userMessage || ""}`,
      `TOOL CALLS:\n${toolLines || "  (none)"}`,
      `FINAL REPLY:\n${trace.finalResponse || ""}`,
    ].join("\n\n");

    return { output: rendered };
  }
}

module.exports = TraceProvider;
