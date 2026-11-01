// Companion refuses a dev module whose installed @companion-module/base is outside its
// supported range (5.1.1: 1 - 1.14.x), and the deploy runs `npm install` with no lockfile.
// A caret range therefore turns a new upstream release into a dead module on the next deploy.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");

const SUPPORTED_MINOR = "1.14";
const moduleDir = path.join(__dirname, "..");
const repoRoot = path.join(moduleDir, "..", "..", "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(moduleDir, "package.json"), "utf8"));

test("the module base cannot resolve past what Companion accepts", () => {
  const spec = packageJson.dependencies["@companion-module/base"];
  assert.ok(spec.startsWith(`~${SUPPORTED_MINOR}.`), `got ${spec}`);
});

test("the deploy fails when the module does not connect", () => {
  const script = fs.readFileSync(path.join(repoRoot, "scripts/deploy/deploy_companion_module.sh"), "utf8");
  assert.match(script, /sport = :18175/);
  assert.match(script, /did not connect/);
});
