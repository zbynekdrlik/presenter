# Mutation Testing & Test-Integrity CI Gates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add mutation testing and test-integrity checks to the CI pipeline so that weak or degenerate tests are caught before deploy.

**Architecture:** A new `mutation` job in `pipeline.yml` runs `cargo-mutants --workspace` after the `test` job, in parallel with `coverage` and `quality`. Test-integrity checks (empty tests, false positives, low assertion density) are added to the existing `quality-check.sh` script. The `build` job is gated on all three passing.

**Tech Stack:** cargo-mutants, ripgrep, bash, GitHub Actions

**Spec:** `docs/superpowers/specs/2026-04-06-mutation-testing-ci-gates-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `.github/workflows/pipeline.yml` | Modify | Add `mutation` job, update `build` dependencies |
| `scripts/dev/quality-check.sh` | Modify | Add test-integrity checks (empty tests, false positives, assertion density) |

No new files are created. No crate code changes.

---

### Task 1: Add test-integrity checks to quality-check.sh

**Files:**
- Modify: `scripts/dev/quality-check.sh` (add checks after existing check #14, before the "Emit results" section)

- [ ] **Step 1: Write the empty-test-body check**

Add this after the `# 14) cargo check warnings` section (line ~364) and before the `# Emit results` comment (line ~366) in `scripts/dev/quality-check.sh`:

```bash
# 15) Test integrity: no assertion-free test functions
if command -v rg >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
  test_integrity=$(python3 - "$ROOT_DIR" <<'TEST_PY'
import re, sys, os, json

root = sys.argv[1]
issues = []

test_attr = re.compile(r'^\s*#\[test\]')
fn_start = re.compile(r'^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)')
assert_kw = re.compile(r'\b(assert!|assert_eq!|assert_ne!|assert_matches!|panic!|expect\(|should_panic|unwrap_err)')
false_positive = re.compile(r'assert!\(\s*true\s*\)')

for dirpath, _, filenames in os.walk(os.path.join(root, "crates")):
    for name in filenames:
        if not name.endswith(".rs"):
            continue
        path = os.path.join(dirpath, name)
        rel = os.path.relpath(path, root)
        with open(path, "r", encoding="utf-8") as f:
            lines = f.readlines()

        i = 0
        while i < len(lines):
            if test_attr.match(lines[i]):
                # Find the fn line
                j = i + 1
                while j < len(lines) and not fn_start.match(lines[j]):
                    j += 1
                if j >= len(lines):
                    i = j
                    continue
                fn_match = fn_start.match(lines[j])
                fn_name = fn_match.group(1)
                fn_line = j + 1

                # Find function body (brace matching)
                k = j
                brace = 0
                started = False
                body_lines = []
                while k < len(lines):
                    brace += lines[k].count("{")
                    brace -= lines[k].count("}")
                    if "{" in lines[k]:
                        started = True
                    if started:
                        body_lines.append(lines[k])
                    if started and brace == 0:
                        break
                    k += 1

                body = "".join(body_lines)

                # Check: no assertions at all
                if not assert_kw.search(body):
                    issues.append({"file": rel, "line": fn_line, "fn": fn_name, "type": "no_assertions"})

                # Check: assert!(true)
                if false_positive.search(body):
                    issues.append({"file": rel, "line": fn_line, "fn": fn_name, "type": "false_positive"})

                # Check: low assertion density (>20 body lines, <=1 assertion)
                body_line_count = len([l for l in body_lines if l.strip()])
                assertion_count = len(assert_kw.findall(body))
                if body_line_count > 20 and assertion_count <= 1:
                    issues.append({"file": rel, "line": fn_line, "fn": fn_name, "type": "low_density",
                                   "body_lines": body_line_count, "assertions": assertion_count})

                i = k + 1
            else:
                i += 1

print(json.dumps(issues))
TEST_PY
  )

  if [[ -n "$test_integrity" && "$test_integrity" != "[]" ]]; then
    while IFS= read -r row; do
      file=$(echo "$row" | jq -r '.file')
      line=$(echo "$row" | jq -r '.line')
      fn_name=$(echo "$row" | jq -r '.fn')
      issue_type=$(echo "$row" | jq -r '.type')
      case "$issue_type" in
        no_assertions)
          fail "Test integrity: ${file}:${line} fn ${fn_name} has no assertions"
          ;;
        false_positive)
          fail "Test integrity: ${file}:${line} fn ${fn_name} uses assert!(true)"
          ;;
        low_density)
          body_lines=$(echo "$row" | jq -r '.body_lines')
          assertions=$(echo "$row" | jq -r '.assertions')
          warn "Test integrity: ${file}:${line} fn ${fn_name} has ${assertions} assertion(s) in ${body_lines} lines"
          ;;
      esac
    done < <(echo "$test_integrity" | jq -c '.[]')
  fi
fi
```

- [ ] **Step 2: Run the quality check locally to verify no existing tests fail the new checks**

Run:
```bash
cd /home/newlevel/devel/presenter/presenter-dev2
bash scripts/dev/quality-check.sh --strict 2>&1 | tail -30
```

Expected: No new failures from the test-integrity check (we verified earlier that no `assert!(true)` exists and tests have real assertions). If any failures appear, they are real issues in existing tests that must be fixed before proceeding.

- [ ] **Step 3: Fix any existing test-integrity violations found in Step 2**

If any test functions were flagged as having no assertions or false positives, fix them now. Add real assertions or remove degenerate tests. Do not suppress the check.

- [ ] **Step 4: Commit**

```bash
git add scripts/dev/quality-check.sh
git commit -m "ci: add test-integrity checks to quality-check.sh

Detect empty test bodies, assert!(true) false positives, and low
assertion density in #[test] functions. Enforced in strict mode."
```

---

### Task 2: Add mutation testing job to pipeline.yml

**Files:**
- Modify: `.github/workflows/pipeline.yml` (add `mutation` job after `test`, update `build` dependencies)

- [ ] **Step 1: Add the mutation job**

Add this new job section in `.github/workflows/pipeline.yml` after the `test` job (after line 288, before the `# Quality Checks` section). Insert it between the `test` job and the `quality` job:

```yaml
  # ============================================
  # Mutation Testing
  # ============================================
  mutation:
    name: Mutation Testing
    runs-on: ubuntu-latest
    needs: test
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v4

      - name: Install system dependencies
        run: sudo apt-get update -qq && sudo apt-get install -y -qq protobuf-compiler cmake nasm

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Setup Rust cache
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: ci
          cache-on-failure: false

      - name: Cache cargo-mutants binary
        uses: actions/cache@v4
        with:
          path: ~/.cargo/bin/cargo-mutants
          key: cargo-mutants-${{ runner.os }}-v1
          restore-keys: cargo-mutants-${{ runner.os }}-

      - name: Install cargo-mutants
        run: command -v cargo-mutants >/dev/null 2>&1 || cargo install cargo-mutants --locked

      - name: Run mutation testing
        run: cargo mutants --workspace --timeout 120 --no-shuffle
```

- [ ] **Step 2: Update the build job dependencies**

Change the `build` job's `needs` list from:

```yaml
    needs:
      - test
      - quality
```

to:

```yaml
    needs:
      - test
      - quality
      - mutation
```

- [ ] **Step 3: Verify the YAML is valid**

Run:
```bash
cd /home/newlevel/devel/presenter/presenter-dev2
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/pipeline.yml'))" && echo "YAML valid"
```

Expected: `YAML valid`

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/pipeline.yml
git commit -m "ci: add mutation testing job to pipeline

cargo-mutants runs full workspace with 120s per-mutant timeout.
Build job is now gated on mutation testing passing."
```

---

### Task 3: Run cargo fmt, push, and monitor CI

**Files:** None (verification only)

- [ ] **Step 1: Run cargo fmt check**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all --check
```

Expected: No formatting issues (we only changed YAML and bash).

- [ ] **Step 2: Run npm lint**

```bash
npm run lint 2>/dev/null || echo "no npm lint configured"
```

- [ ] **Step 3: Commit spec and plan docs**

```bash
git add docs/superpowers/specs/2026-04-06-mutation-testing-ci-gates-design.md
git add docs/superpowers/plans/2026-04-06-mutation-testing-ci-gates.md
git commit -m "docs: add mutation testing and test-integrity gates spec and plan"
```

- [ ] **Step 4: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 5: Monitor CI run**

```bash
gh run list --branch dev --limit 3
```

Watch the run until all jobs reach terminal state. Pay special attention to:
- `Mutation Testing` job — this is the new job. If it fails with surviving mutants, note which mutants survived and fix the tests.
- `Quality Checks` job — verify the new test-integrity checks pass.
- All other jobs should be unaffected.

- [ ] **Step 6: Handle surviving mutants (if any)**

If `cargo mutants` reports surviving mutants, the output will show which code mutations weren't caught by tests. For each surviving mutant:

1. Read the mutant description (e.g., "replace `>` with `>=` in foo::bar at line 42")
2. Write a test that specifically catches that mutation
3. Verify the test passes normally and would fail with the mutation
4. Commit: `git commit -m "test: add assertions to catch mutation in <module>"`
5. Push and re-monitor CI

Repeat until zero surviving mutants.

- [ ] **Step 7: If mutation testing job times out**

If the 45-minute job timeout is hit, check the CI logs for how many mutants were tested and how many remain. Options:
- If most mutants completed and only a few remain: the timeout is too tight. Do NOT increase it without investigating which mutants are slow.
- If many mutants are timing out at the 120s per-mutant limit: some code paths may have infinite loops when mutated. This is expected — the per-mutant timeout handles it.

---

### Task 4: Verify and report

- [ ] **Step 1: Confirm all CI jobs are green**

```bash
gh run list --branch dev --limit 1
```

All jobs must be green, including the new `Mutation Testing` job.

- [ ] **Step 2: Verify mutation job ran full workspace**

Check the CI logs for the mutation job. The output should show:
- Number of mutants tested
- Number killed (tests caught the mutation)
- Number survived (zero expected)
- Number timed out (acceptable — these are mutations that cause infinite loops)

- [ ] **Step 3: Verify test-integrity checks ran**

Check the `Quality Checks` job logs. The quality-check.sh output should show no test-integrity failures.

- [ ] **Step 4: Done**

All CI jobs green, mutation testing catching weak tests, test-integrity checks enforced. Report completion.
