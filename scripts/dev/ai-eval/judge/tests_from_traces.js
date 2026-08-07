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
//
// Two failure-handling rules, both #662 follow-ups:
//   1. No traces directory, or a traces directory with zero *.json files, is an ERROR, not an
//      empty test list — an empty `tests:` array makes promptfoo report 0/0 and exit 0 (green),
//      which would silently mask "the drive stage never ran" as a passing judge run.
//   2. A single truncated/corrupt trace file must NOT abort the whole generator (and therefore
//      every other case's judge run) via an unguarded JSON.parse throw. Each file is parsed in
//      its own try/catch; a parse failure becomes a test that always FAILS (via a rubric that
//      can never pass), carrying the filename and parse error in its description, so the one
//      bad trace shows up as one visible failure instead of taking the rest of the suite down.

const fs = require("fs");
const path = require("path");

const RUBRICS = [
  "verse-fidelity.md",
  "slide-splitting.md",
  "slovak-naturalness.md",
  "final-reply-language.md",
];

function parseFailureTest(file, error) {
  const caseId = file.replace(/\.json$/, "");
  return {
    description: `${caseId}: FAILED TO PARSE trace file (${error.message})`,
    vars: { caseId },
    assert: [
      {
        type: "equals",
        // Deliberately unsatisfiable: the provider's actual output can never equal this
        // sentinel, so the test always fails and surfaces the parse error in the report
        // instead of being silently dropped from the suite.
        value: `__UNPARSEABLE_TRACE__:${file}:${error.message}`,
      },
    ],
  };
}

module.exports = function generateTests() {
  const tracesDir = path.join(__dirname, "..", "traces");
  if (!fs.existsSync(tracesDir)) {
    throw new Error(
      `no traces found — expected trace JSON under ${tracesDir}; run the drive stage first ` +
        `(scripts/dev/ai-eval/run.sh --stage drive)`
    );
  }

  const files = fs
    .readdirSync(tracesDir)
    .filter((f) => f.endsWith(".json"))
    .sort();

  if (files.length === 0) {
    throw new Error(
      `no traces found — ${tracesDir} exists but contains no *.json files; run the drive stage ` +
        `first (scripts/dev/ai-eval/run.sh --stage drive)`
    );
  }

  return files.map((file) => {
    const caseId = file.replace(/\.json$/, "");
    let trace;
    try {
      trace = JSON.parse(fs.readFileSync(path.join(tracesDir, file), "utf8"));
    } catch (error) {
      return parseFailureTest(file, error);
    }

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
