// judge/tests_from_traces.js
//
// SKELETON — not yet exercised end-to-end (see trace_provider.js's header for the full status
// note; same caveats apply). Promptfoo supports a JS module as the `tests:` source
// (https://www.promptfoo.dev/docs/configuration/test-cases/#import-from-a-javascript-file):
// a default export function returning an array of test-case objects.
//
// This generator creates ONE test per captured trace file in ../traces/*.json — never per
// corpus/*.case.json directly, because a case with no captured trace yet has nothing to judge.
// Each test's `vars.caseId` selects the matching trace via trace_provider.js, and each of the
// four rubric dimensions is attached as its own llm-rubric assertion so a single test's pass/fail
// output shows exactly which dimension(s) failed rather than one blended verdict.

const fs = require("fs");
const path = require("path");

const RUBRICS = [
  "verse-fidelity.md",
  "slide-splitting.md",
  "slovak-naturalness.md",
  "final-reply-language.md",
];

module.exports = function generateTests() {
  const tracesDir = path.join(__dirname, "..", "traces");
  if (!fs.existsSync(tracesDir)) {
    return [];
  }

  const files = fs
    .readdirSync(tracesDir)
    .filter((f) => f.endsWith(".json"))
    .sort();

  return files.map((file) => {
    const caseId = file.replace(/\.json$/, "");
    const trace = JSON.parse(fs.readFileSync(path.join(tracesDir, file), "utf8"));

    return {
      description: `${caseId} (${trace.slice || "unknown-slice"})`,
      vars: { caseId },
      assert: RUBRICS.map((rubricFile) => ({
        type: "llm-rubric",
        // Bible-authoring cases are graded on all four dimensions; worship-crud and
        // adversarial cases have no verse content, so verse-fidelity/slide-splitting are
        // skipped for them (an emphasis/title packing question does not apply to a
        // create_library call). This filter runs at generation time, not per-assertion.
        value: `file://./rubrics/${rubricFile}`,
        provider: {
          id: "anthropic:messages:claude-opus-4-6",
          config: { temperature: 0 },
        },
      })).filter((assertion) => {
        const isBibleContentRubric =
          assertion.value.includes("verse-fidelity") ||
          assertion.value.includes("slide-splitting");
        return !isBibleContentRubric || trace.slice === "bible-authoring";
      }),
    };
  });
};
