#!/usr/bin/env python3
"""Function-length gate for quality-check.sh section 7.

Walks the `crates/` tree under a given repo root and reports Rust functions
that exceed the warn (>80) and hard-fail (>120) line caps.

Scoping (the abs/rel regression this file guards against — see #374):
  `QC_TARGETS` carries the changed-file list from `git diff --name-only`, which
  yields RELATIVE paths (e.g. `crates/presenter-ndi/src/pipeline/consumers.rs`).
  `os.walk` yields ABSOLUTE paths. The scope filter MUST compare on the relative
  form (`os.path.relpath(path, root)`), or it silently skips EVERY file and the
  gate becomes a no-op. Comparing the absolute `path` against relative targets
  was the bug that disabled this gate in every scoped CI run (fixed in 5f5fe9f).

Output: a single JSON object on stdout:
  {"violations": [...], "warnings": [...]}
Each entry: {"file": <rel>, "start": <1-based line>, "length": <lines>, "fn": <name>}

Env:
  QC_TARGETS       newline-separated RELATIVE file paths to scope to (empty = all)
  QC_FN_ADVISORY   "1" => never hard-fail (>120 demoted to warning); for
                   no-diff (advisory) runs where unchanged files would
                   otherwise hard-fail a version-bump-only commit.

Usage: fn_length_check.py <repo_root>
"""

import json
import os
import re
import sys

fn_start = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)\s*\(")

# Exempt patterns per CLAUDE.md:
# - Migration up()/down() functions (declarative schema)
# - render_*_ui functions (Leptos HTML-like DSL)
# - build_router functions (route declarations)
# - Leptos #[component] functions (UI renders — the view! DSL is HTML-like)
EXEMPT_FN_NAMES = {"up", "down", "build_router"}
EXEMPT_FN_PREFIXES = ("render_",)


def is_exempt_function(fn_name, filepath):
    # Migration files - exempt all functions
    if "/presenter-migration/" in filepath:
        return True
    # Specific exempt function names
    if fn_name in EXEMPT_FN_NAMES:
        return True
    # UI render functions
    for prefix in EXEMPT_FN_PREFIXES:
        if fn_name.startswith(prefix):
            return True
    return False


def preceded_by_component(lines, idx):
    # A Leptos #[component] function is a UI render (per CLAUDE.md). The
    # attribute sits directly above the fn, possibly with other attributes or
    # doc comments interleaved. Walk upward over blank/comment/attribute lines;
    # if we reach #[component] it's exempt, any other code line stops the scan.
    j = idx - 1
    while j >= 0:
        s = lines[j].strip()
        if not s:
            j -= 1
            continue
        if s.startswith("#["):
            if "component" in s:
                return True
            j -= 1
            continue
        if s.startswith("//") or s.startswith("/*") or s.startswith("*"):
            j -= 1
            continue
        return False
    return False


def analyze(root, targets, advisory):
    violations = []  # > 120 lines (hard fail)
    warnings = []  # > 80 lines (warning)
    for dirpath, _, filenames in os.walk(os.path.join(root, "crates")):
        for name in filenames:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(dirpath, name)
            # `targets` come from `git diff --name-only` as RELATIVE paths, but
            # os.walk yields ABSOLUTE paths — compare on the relative form or the
            # scope filter silently skips EVERY file (the gate becomes a no-op).
            rel = os.path.relpath(path, root)
            if targets and rel not in targets:
                continue
            with open(path, "r", encoding="utf-8") as f:
                lines = f.readlines()
            i = 0
            while i < len(lines):
                match = fn_start.match(lines[i])
                if match:
                    fn_name = match.group(1)
                    # find first '{'
                    j = i
                    brace = 0
                    started = False
                    while j < len(lines):
                        brace += lines[j].count("{")
                        brace -= lines[j].count("}")
                        if "{" in lines[j]:
                            started = True
                        if started and brace == 0:
                            length = j - i + 1
                            if not is_exempt_function(fn_name, path) and not preceded_by_component(lines, i):
                                entry = {"file": rel, "start": i + 1, "length": length, "fn": fn_name}
                                # Advisory mode = no diff vs base (e.g. a version-bump-only
                                # commit) -> ALL repo files are scanned. Pre-existing long
                                # functions in unchanged files must not hard-fail such a
                                # commit (mirrors the file-size check's is_changed logic);
                                # they are reported as warnings instead. With a real diff,
                                # targets are exactly the changed files and >120 hard-fails.
                                if length > 120 and not advisory:
                                    violations.append(entry)
                                elif length > 80:
                                    warnings.append(entry)
                            i = j
                            break
                        j += 1
                i += 1
    violations.sort(key=lambda v: (-v["length"], v["file"], v["start"]))
    warnings.sort(key=lambda v: (-v["length"], v["file"], v["start"]))
    return {"violations": violations, "warnings": warnings}


def main():
    if len(sys.argv) < 2:
        sys.stderr.write("usage: fn_length_check.py <repo_root>\n")
        return 2
    root = sys.argv[1]
    targets_env = os.environ.get("QC_TARGETS", "")
    targets = [t for t in targets_env.split("\n") if t.strip()]
    advisory = os.environ.get("QC_FN_ADVISORY", "0") == "1"
    print(json.dumps(analyze(root, targets, advisory)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
