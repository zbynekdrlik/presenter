#!/usr/bin/env python3
"""Guard deploy workflows against reaching the WRONG host over SSH (#795).

The self-hosted runner shares ONE ``~/.ssh/config`` across every job. A deploy
job's SSH setup step writes a ``Host deploy-target`` alias pointing at its own
``DEPLOY_HOST``. #795: release.yml wrote that alias only AFTER ``ssh-keyscan``
succeeded; when keyscan failed (PP offline) the step aborted before rewriting,
so the alias still held the PREVIOUS job's host (SNV prod), and the
``if: always()`` "Start service" step then ran ``ssh deploy-target`` against
SNV instead of PP.

Two invariants are enforced for every job whose step writes the
``Host deploy-target`` alias:

1. The setup step has an ``id`` and writes the alias (truncating ``>``, never
   appending ``>>``) BEFORE its first ``ssh-keyscan`` — so a keyscan failure can
   never leave a stale alias from another job behind.
2. Every later step that talks to ``deploy-target`` (ssh/scp/rsync) and would
   run after an earlier failure (its ``if:`` uses ``always()``, ``failure()`` or
   ``cancelled()``) is ALSO gated on
   ``steps.<setup-id>.outcome == 'success'`` — so a recovery step (#469) only
   touches a host the job actually set up.

Usage: check_deploy_workflows.py [WORKFLOW.yml ...]
       (default: every .github/workflows/*.yml)
Exit 0 when clean, 1 on any violation (each printed as a GitHub ::error::).
"""

from __future__ import annotations

import glob
import re
import sys
from dataclasses import dataclass
from typing import Any

import yaml

ALIAS = "deploy-target"
ALIAS_HOST_RE = re.compile(r"^\s*Host\s+deploy-target\s*$", re.MULTILINE)
CONFIG_WRITE_RE = re.compile(r"cat\s+(>>?)\s*~/\.ssh/config")
KEYSCAN_RE = re.compile(r"\bssh-keyscan\b")
# A step "talks to deploy-target" when the alias appears anywhere on an
# executable line (`ssh deploy-target`, `ssh -o X \` + `deploy-target "..."`,
# scp/rsync `deploy-target:/path`). Deliberately broad: a false positive fails
# loudly and is trivially gated; a false negative is the #795 incident.
REMOTE_USE_RE = re.compile(r"\bdeploy-target\b")
STATUS_FN_RE = re.compile(r"\b(always|failure|cancelled)\s*\(\s*\)")


@dataclass(frozen=True)
class Violation:
    workflow: str
    job: str
    step: str
    message: str

    def render(self) -> str:
        return f"::error file={self.workflow}::[{self.job}] {self.step}: {self.message}"


def _script(step: dict[str, Any]) -> str:
    """The step's ``run`` script with shell comment lines blanked out.

    Comments routinely MENTION ``ssh-keyscan`` / ``ssh deploy-target`` while
    explaining the code; only executable lines may count. Blanking (not
    deleting) keeps the relative order of what remains intact. Backslash
    continuations are joined first so one logical command is one line.
    """
    joined = re.sub(r"\\\n", " ", str(step.get("run", "")))
    lines = joined.splitlines()
    return "\n".join("" if line.lstrip().startswith("#") else line for line in lines)


def _step_name(step: dict[str, Any], index: int) -> str:
    return str(step.get("name") or step.get("id") or f"step #{index + 1}")


def _setup_violations(workflow: str, job: str, name: str, step: dict[str, Any]) -> list[Violation]:
    out: list[Violation] = []
    script = _script(step)
    if not step.get("id"):
        out.append(Violation(workflow, job, name, "SSH setup step writing `Host deploy-target` must have an `id` so recovery steps can gate on its outcome (#795)"))
    alias_at = ALIAS_HOST_RE.search(script)
    write = None
    for m in CONFIG_WRITE_RE.finditer(script):
        if alias_at is None or m.start() < alias_at.start():
            write = m
    if write is None:
        out.append(Violation(workflow, job, name, "cannot find the `cat > ~/.ssh/config` write of the `Host deploy-target` block"))
        return out
    if write.group(1) == ">>":
        out.append(Violation(workflow, job, name, "`Host deploy-target` must be written with `cat > ~/.ssh/config` (truncate), not appended — an appended block leaves a stale alias from another job first in the file (#795)"))
    keyscan = KEYSCAN_RE.search(script)
    if keyscan is not None and keyscan.start() < write.start():
        out.append(Violation(workflow, job, name, "`~/.ssh/config` (Host deploy-target) is written AFTER ssh-keyscan — a keyscan failure leaves the previous job's alias (another host) in place (#795); write the config first"))
    return out


def check_job(workflow: str, job: str, spec: dict[str, Any]) -> list[Violation]:
    steps = spec.get("steps") or []
    violations: list[Violation] = []
    setup_id: str | None = None
    for index, step in enumerate(steps):
        if not isinstance(step, dict):
            continue
        name = _step_name(step, index)
        script = _script(step)
        if ALIAS_HOST_RE.search(script):
            violations.extend(_setup_violations(workflow, job, name, step))
            setup_id = str(step["id"]) if step.get("id") else None
            continue
        if not REMOTE_USE_RE.search(script):
            continue
        condition = str(step.get("if", ""))
        if not STATUS_FN_RE.search(condition):
            continue  # implicit success(): skipped once any earlier step failed
        if setup_id is None:
            violations.append(Violation(workflow, job, name, f"uses `{ALIAS}` with `if: {condition}` but no preceding SSH setup step with an `id` exists to gate on (#795)"))
            continue
        gate = re.compile(r"steps\." + re.escape(setup_id) + r"""\.outcome\s*==\s*(['"])success\1""")
        # `||` would let the status function alone satisfy the condition, so a
        # gate OR-ed in gates nothing — require a pure `&&` conjunction.
        if not gate.search(condition) or "||" in condition:
            violations.append(Violation(workflow, job, name, f"uses `{ALIAS}` with `if: {condition}` — must also require `&& steps.{setup_id}.outcome == 'success'` (no `||`) so it never runs after the SSH setup failed (#795)"))
    return violations


def check_workflow(path: str, document: Any) -> list[Violation]:
    if not isinstance(document, dict):
        return []
    jobs = document.get("jobs") or {}
    violations: list[Violation] = []
    for job, spec in jobs.items():
        if isinstance(spec, dict):
            violations.extend(check_job(path, str(job), spec))
    return violations


def main(argv: list[str]) -> int:
    paths = argv or sorted(glob.glob(".github/workflows/*.yml"))
    if not paths:
        print("::error::no workflow files found", file=sys.stderr)
        return 1
    violations: list[Violation] = []
    for path in paths:
        with open(path, encoding="utf-8") as handle:
            violations.extend(check_workflow(path, yaml.safe_load(handle)))
    for violation in violations:
        print(violation.render())
    if violations:
        print(f"{len(violations)} deploy-workflow SSH safety violation(s) (#795)", file=sys.stderr)
        return 1
    print(f"deploy-workflow SSH safety OK ({len(paths)} workflow file(s) checked)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
