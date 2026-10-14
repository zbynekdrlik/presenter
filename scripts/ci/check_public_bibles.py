#!/usr/bin/env python3
"""Keep licence-restricted Bible modules out of this PUBLIC repo (#859).

The SEB, Roháček, SEVP and Milosť modules may not be distributed publicly (the
SEB module itself says "Nešíriť bez povolenia"; the rights of the other three
are unclear). They live in the PRIVATE repo ``zbynekdrlik/presenter-bibles``;
every job that needs them checks that repo out with the read-only deploy key
(secret ``BIBLES_DEPLOY_KEY``) into ``data/bibles-private/`` (gitignored). Only
public-domain modules stay in ``data/bibles/``. Git history is never rewritten,
so commits before #859 still hold the four modules; this guard keeps them from
coming back.

Invariants:

1. Every tracked file under ``data/bibles/`` is on the public-domain allow-list
   (``PUBLIC_MODULES``). Anything else (a ``*.mybible.zip``, a nested copy, a
   stray file) fails: once pushed it stays in the public history for good.
2. Nothing under ``data/bibles-private/`` is tracked. The path is gitignored;
   a ``git add -f`` would publish the modules.
3. Nothing public is built from the private modules. A job that publishes a
   GitHub Release asset (``softprops/action-gh-release``, ``gh release
   create|upload``) never checks out the private repo and never names
   ``data/bibles-private`` (the release tarball is a public download), and no
   ``actions/upload-artifact`` step uploads a path under it (the artifacts of a
   public repo can be downloaded by any GitHub user).
4. A job that reads ``data/bibles-private`` checks the private repo out in an
   EARLIER step, into ``path: data/bibles-private`` with
   ``persist-credentials: false`` (the deploy key must not stay in the
   checkout's git config on the shared self-hosted runner). A forgotten
   checkout fails here instead of as a confusing "failed to read bible archive"
   deep inside an E2E run.

Usage: check_public_bibles.py [--root DIR]   (default: the current directory)
Exit 0 when clean, 1 on any violation (each printed as a GitHub ::error::).
"""

from __future__ import annotations

import argparse
import glob
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from typing import Any

import yaml

PUBLIC_DIR = "data/bibles"
PRIVATE_DIR = "data/bibles-private"
PRIVATE_REPO = "zbynekdrlik/presenter-bibles"
# Public-domain modules that may stay in the public repo, relative to PUBLIC_DIR.
PUBLIC_MODULES = frozenset({"kjv.usfm.zip"})
PRIVATE_MARK = "bibles-private"
GH_RELEASE_RE = re.compile(r"\bgh\s+release\s+(?:create|upload)\b")


@dataclass(frozen=True)
class Violation:
    file: str
    message: str

    def render(self) -> str:
        return f"::error file={self.file}::{self.message}"


def tracked_files(root: str, pathspec: str) -> list[str]:
    """Tracked files under `pathspec`, as git prints them (repo-relative, `/`)."""
    result = subprocess.run(
        ["git", "-C", root, "ls-files", "-z", "--", pathspec],
        check=True,
        capture_output=True,
        text=True,
    )
    return [path for path in result.stdout.split("\0") if path]


def public_dir_violations(paths: list[str]) -> list[Violation]:
    """Invariant 1: only allow-listed modules are tracked under data/bibles/."""
    prefix = PUBLIC_DIR + "/"
    violations: list[Violation] = []
    for path in paths:
        if not path.startswith(prefix):
            continue
        if path[len(prefix):] in PUBLIC_MODULES:
            continue
        violations.append(Violation(path, f"only public-domain Bible modules may be in this public repo ({', '.join(sorted(PUBLIC_MODULES))}); this file belongs in the private repo {PRIVATE_REPO} (checked out to {PRIVATE_DIR}/), or on the allow-list in scripts/ci/check_public_bibles.py if it is public domain (#859)"))
    return violations


def private_dir_violations(paths: list[str]) -> list[Violation]:
    """Invariant 2: nothing under data/bibles-private/ is tracked."""
    prefix = PRIVATE_DIR + "/"
    return [
        Violation(path, f"{PRIVATE_DIR}/ is the gitignored checkout of the private repo {PRIVATE_REPO}; a tracked file there publishes a licence-restricted module (#859)")
        for path in paths
        if path.startswith(prefix)
    ]


def _script(step: dict[str, Any]) -> str:
    """The step's `run` script without shell comment lines (a comment may
    explain the private modules without touching them)."""
    lines = str(step.get("run", "")).splitlines()
    return "\n".join(line for line in lines if not line.lstrip().startswith("#"))


def _text(value: Any) -> str:
    """Every string inside a mapping / list (env, with), joined."""
    if isinstance(value, dict):
        return "\n".join(_text(item) for item in value.values())
    if isinstance(value, list):
        return "\n".join(_text(item) for item in value)
    return "" if value is None else str(value)


def _uses(step: dict[str, Any], action: str) -> bool:
    return str(step.get("uses", "")).split("@", 1)[0] == action


def _with(step: dict[str, Any]) -> dict[str, Any]:
    spec = step.get("with")
    return spec if isinstance(spec, dict) else {}


def _is_private_checkout(step: dict[str, Any]) -> bool:
    return _uses(step, "actions/checkout") and str(_with(step).get("repository", "")) == PRIVATE_REPO


def _mentions_private(step: dict[str, Any]) -> bool:
    text = "\n".join((_script(step), _text(step.get("env")), _text(_with(step))))
    return PRIVATE_MARK in text


def _publishes_release(step: dict[str, Any]) -> bool:
    return _uses(step, "softprops/action-gh-release") or bool(GH_RELEASE_RE.search(_script(step)))


def _step_name(step: dict[str, Any], index: int) -> str:
    return str(step.get("name") or step.get("id") or step.get("uses") or f"step #{index + 1}")


def check_job(workflow: str, job: str, spec: dict[str, Any]) -> list[Violation]:
    """Invariants 3 and 4 for one job."""
    steps = [step for step in spec.get("steps") or [] if isinstance(step, dict)]
    violations: list[Violation] = []

    if any(_publishes_release(step) for step in steps):
        for index, step in enumerate(steps):
            if _is_private_checkout(step) or _mentions_private(step):
                violations.append(Violation(workflow, f"[{job}] {_step_name(step, index)}: this job publishes a GitHub Release asset (a public download) and must never check out {PRIVATE_REPO} or read {PRIVATE_DIR}/ (#859)"))
    for index, step in enumerate(steps):
        if _uses(step, "actions/upload-artifact") and PRIVATE_MARK in _text(_with(step).get("path")):
            violations.append(Violation(workflow, f"[{job}] {_step_name(step, index)}: uploads {PRIVATE_DIR}/ as a workflow artifact, which any GitHub user can download from a public repo (#859)"))

    checkout_at: int | None = None
    for index, step in enumerate(steps):
        name = _step_name(step, index)
        if _is_private_checkout(step):
            options = _with(step)
            if str(options.get("path", "")).rstrip("/") != PRIVATE_DIR:
                violations.append(Violation(workflow, f"[{job}] {name}: check {PRIVATE_REPO} out into `path: {PRIVATE_DIR}` (the gitignored path every consumer reads, #859)"))
            if options.get("persist-credentials") is not False:
                violations.append(Violation(workflow, f"[{job}] {name}: set `persist-credentials: false` so the deploy key does not stay in the checkout's git config on the shared runner (#859)"))
            if checkout_at is None:
                checkout_at = index
            continue
        if _mentions_private(step) and checkout_at is None:
            violations.append(Violation(workflow, f"[{job}] {name}: reads {PRIVATE_DIR}/ but no earlier step checks out {PRIVATE_REPO} into it (add the 'Check out the private Bible modules (#859)' step after the main checkout)"))
    if PRIVATE_MARK in _text(spec.get("env")) and checkout_at is None:
        violations.append(Violation(workflow, f"[{job}]: its env reads {PRIVATE_DIR}/ but no step checks out {PRIVATE_REPO} (#859)"))
    return violations


def check_workflow(path: str, document: Any) -> list[Violation]:
    if not isinstance(document, dict):
        return []
    violations: list[Violation] = []
    for job, spec in (document.get("jobs") or {}).items():
        if isinstance(spec, dict):
            violations.extend(check_job(path, str(job), spec))
    return violations


def check_repository(root: str) -> list[Violation]:
    violations = public_dir_violations(tracked_files(root, PUBLIC_DIR))
    violations.extend(private_dir_violations(tracked_files(root, PRIVATE_DIR)))
    for path in sorted(glob.glob(os.path.join(root, ".github", "workflows", "*.yml"))):
        with open(path, encoding="utf-8") as handle:
            document = yaml.safe_load(handle)
        violations.extend(check_workflow(os.path.relpath(path, root), document))
    return violations


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="repository root (default: the current directory)")
    args = parser.parse_args(argv)
    violations = check_repository(args.root)
    for violation in violations:
        print(violation.render())
    if violations:
        print(f"{len(violations)} public-Bible-module violation(s) (#859)", file=sys.stderr)
        return 1
    print(f"public Bible modules OK: {PUBLIC_DIR}/ holds only {', '.join(sorted(PUBLIC_MODULES))}; {PRIVATE_DIR}/ untracked and never published")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
