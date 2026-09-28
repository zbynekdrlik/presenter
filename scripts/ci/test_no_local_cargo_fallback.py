#!/usr/bin/env python3
"""Regression guard for #802: no repo script or E2E helper compiles Rust locally
unless the caller explicitly opted in.

Presenter is Tier-0 (CI-only builds). The E2E setup paths used to fall back to
`cargo run` when a prebuilt binary was missing or stale, and several dev/ops
scripts (run-dev-server, ingest-default-bibles, ai-eval, run-env, watch-demo,
quality-check, verify-and-refresh) called compiling cargo directly. All of that
ran inside node / a `.sh`, where the Bash-level `block-tier0-local-build.sh`
hook cannot see it, so a script run quietly compiled the workspace. The rule
now: use the prebuilt binary (CI artifact) or FAIL loudly with a
`gh run download` recipe; compiling happens only with the explicit opt-in
`PRESENTER_ALLOW_LOCAL_CARGO=1` (the shared shell guard lives in
`scripts/dev/lib/local-cargo.sh`).

Two layers:
  * behavioural — run the real scripts in a sandbox repo with a fake `cargo`
    on PATH and assert it is never invoked unless the opt-in is set;
  * structural — every compiling cargo invocation in the TypeScript E2E
    harness lives ONLY in `tests/e2e/prebuilt-binary.ts`, and every one in
    `scripts/dev/` + `scripts/ops/` sits behind the opt-in guard.

Run: python3 -m unittest discover -s scripts/ci -p 'test_no_local_cargo_fallback.py' -v
"""

from __future__ import annotations

import os
import re
import shutil
import stat
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
E2E_DIR = REPO_ROOT / "tests" / "e2e"
HELPER = E2E_DIR / "prebuilt-binary.ts"
SCRIPT_DIRS = (REPO_ROOT / "scripts" / "dev", REPO_ROOT / "scripts" / "ops")
OPT_IN = "PRESENTER_ALLOW_LOCAL_CARGO"
CARGO_COMPILE = re.compile(r"\bcargo\s+(run|build)\b")
# Every cargo subcommand that compiles (Tier-0 bans all of them locally),
# tolerating a toolchain / flags before it (`cargo +nightly build`,
# `cargo --locked test`) and the `cargo-<sub>` binary spelling.
SHELL_CARGO_COMPILE = re.compile(
    r"\bcargo(?:\s+(?:\+\S+|-{1,2}[\w=-]+))*[\s-]+"
    r"(run|build|check|clippy|test|bench|doc|rustc|install|watch"
    r"|nextest|llvm-cov|mutants|fix|miri|tarpaulin)\b"
)
QUOTED = re.compile(r"\"(?:[^\"\\]|\\.)*\"|'[^']*'")
# A quoted string handed to a shell (`bash -lc "cargo test"`) IS code.
SHELL_C_ARG = re.compile(r"(?:^|\s)-l?c\s+(\"(?:[^\"\\]|\\.)*\"|'[^']*')")
SHELL_GUARDS = ("require_local_cargo", "local_cargo_allowed")
NEGATED_GUARD = re.compile(r"!\s*(?:require_local_cargo|local_cargo_allowed)")
GUARD_LOOKBACK = 8
NON_CODE_SUFFIXES = {".md", ".json", ".yaml", ".yml", ".txt"}
IMPORTER_BINS = ("import_propresenter", "ingest_bibles")
SOURCE_CRATES = (
    "presenter-importer",
    "presenter-persistence",
    "presenter-migration",
    "presenter-core",
)


def _write_executable(path: Path, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


class ScriptSandbox:
    """A throwaway repo root holding copies of scripts/dev + scripts/ops, the
    crate source dirs the importer staleness check scans, and a fake `cargo`
    that only records that it was called."""

    def __init__(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="presenter-802-"))
        for src in SCRIPT_DIRS:
            shutil.copytree(src, self.root / "scripts" / src.name)
        for crate in SOURCE_CRATES:
            src = self.root / "crates" / crate / "src" / "lib.rs"
            src.parent.mkdir(parents=True)
            src.write_text("// source\n")
        self.cargo_log = self.root / "cargo-invoked.log"
        self.bin_dir = self.root / "fake-bin"
        _write_executable(
            self.bin_dir / "cargo",
            f'#!/usr/bin/env bash\necho "cargo $*" >> "{self.cargo_log}"\nexit 0\n',
        )
        (self.root / "libs").mkdir()

    def add_binaries(self, profile: str, stale: bool, names=IMPORTER_BINS) -> None:
        for name in names:
            binary = self.root / "target" / profile / name
            _write_executable(binary, "#!/usr/bin/env bash\nexit 0\n")
            if stale:
                old = time.time() - 3600
                os.utime(binary, (old, old))

    def run(
        self, script: str, *args: str, opt_in: bool = False, env_extra=None
    ) -> subprocess.CompletedProcess[str]:
        env = {
            "PATH": f"{self.bin_dir}{os.pathsep}{os.environ['PATH']}",
            "HOME": str(self.root),
            "PRESENTER_DB_URL": f"sqlite://{self.root}/var/test.db",
        }
        if env_extra:
            env.update(env_extra)
        if opt_in:
            env[OPT_IN] = "1"
        return subprocess.run(
            ["bash", str(self.root / script), *args],
            env=env,
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )

    def run_refresh(self, opt_in: bool = False) -> subprocess.CompletedProcess[str]:
        return self.run(
            "scripts/dev/refresh-dev-data.sh", str(self.root / "libs"), opt_in=opt_in
        )

    def cargo_calls(self) -> list[str]:
        if not self.cargo_log.exists():
            return []
        return self.cargo_log.read_text().splitlines()

    def cleanup(self) -> None:
        shutil.rmtree(self.root, ignore_errors=True)


class SandboxCase(unittest.TestCase):
    def setUp(self) -> None:
        self.sandbox = ScriptSandbox()

    def tearDown(self) -> None:
        self.sandbox.cleanup()

    def assert_refused(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.sandbox.cargo_calls(), [])
        self.assertIn("gh run download", result.stderr)
        self.assertIn(OPT_IN, result.stderr)

    def assert_ran_without_cargo(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.sandbox.cargo_calls(), [])


class RefreshDevDataNoSilentCompile(SandboxCase):
    def test_missing_binary_fails_loudly_without_compiling(self) -> None:
        self.assert_refused(self.sandbox.run_refresh())

    def test_stale_binary_fails_loudly_without_compiling(self) -> None:
        self.sandbox.add_binaries("release", stale=True)
        self.assert_refused(self.sandbox.run_refresh())

    def test_fresh_prebuilt_binary_is_used(self) -> None:
        self.sandbox.add_binaries("release", stale=False)
        self.assert_ran_without_cargo(self.sandbox.run_refresh())

    def test_fresh_debug_binary_is_used(self) -> None:
        self.sandbox.add_binaries("debug", stale=False)
        self.assert_ran_without_cargo(self.sandbox.run_refresh())

    def test_stale_release_does_not_hide_fresh_debug(self) -> None:
        self.sandbox.add_binaries("release", stale=True)
        self.sandbox.add_binaries("debug", stale=False)
        self.assert_ran_without_cargo(self.sandbox.run_refresh())

    def test_explicit_opt_in_compiles(self) -> None:
        result = self.sandbox.run_refresh(opt_in=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            self.sandbox.cargo_calls(),
            [
                f"cargo run -p presenter-importer --bin {name} --"
                + (f" --root {self.sandbox.root / 'libs'}" if name == "import_propresenter" else "")
                for name in IMPORTER_BINS
            ],
        )


class DevOpsScriptsNoSilentCompile(SandboxCase):
    def test_run_dev_server_refuses_without_opt_in(self) -> None:
        self.assert_refused(self.sandbox.run("scripts/dev/run-dev-server.sh"))

    def test_run_dev_server_compiles_with_opt_in(self) -> None:
        result = self.sandbox.run("scripts/dev/run-dev-server.sh", opt_in=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.sandbox.cargo_calls(), ["cargo run -p presenter-server"])

    def test_ingest_default_bibles_refuses_before_wiping_db(self) -> None:
        db = self.sandbox.root / "var" / "test.db"
        db.parent.mkdir(parents=True)
        db.write_text("keep me")
        self.assert_refused(self.sandbox.run("scripts/dev/ingest-default-bibles.sh"))
        self.assertEqual(db.read_text(), "keep me")

    def test_ingest_default_bibles_compiles_with_opt_in(self) -> None:
        result = self.sandbox.run("scripts/dev/ingest-default-bibles.sh", opt_in=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            self.sandbox.cargo_calls(),
            ["cargo run -p presenter-importer --bin ingest_bibles"],
        )

    def test_run_env_test_refuses_without_opt_in(self) -> None:
        self.assert_refused(
            self.sandbox.run(
                "scripts/ops/run-env.sh", "test", env_extra={"PRESENTER_RESET_DB": "0"}
            )
        )

    def test_run_env_prod_refuses_without_opt_in(self) -> None:
        self.assert_refused(self.sandbox.run("scripts/ops/run-env.sh", "prod"))

    def test_run_env_prod_compiles_with_opt_in(self) -> None:
        result = self.sandbox.run("scripts/ops/run-env.sh", "prod", opt_in=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            self.sandbox.cargo_calls(), ["cargo run --release -p presenter-server"]
        )

    def test_watch_demo_refuses_without_opt_in(self) -> None:
        # Fresh importer bins so the data refresh succeeds and only the
        # compile-watch step is left to refuse.
        self.sandbox.add_binaries("release", stale=False)
        self.assert_refused(self.sandbox.run("scripts/dev/watch-demo.sh"))

    def _ai_eval_score(self, opt_in: bool = False) -> subprocess.CompletedProcess[str]:
        traces = self.sandbox.root / "scripts" / "dev" / "ai-eval" / "traces"
        traces.mkdir(parents=True, exist_ok=True)
        (traces / "case.json").write_text("{}")
        return self.sandbox.run(
            "scripts/dev/ai-eval/run.sh", "--stage", "score-l1", opt_in=opt_in
        )

    def test_ai_eval_refuses_without_binary_or_opt_in(self) -> None:
        self.assert_refused(self._ai_eval_score())

    def test_ai_eval_uses_prebuilt_binary(self) -> None:
        self.sandbox.add_binaries("release", stale=False, names=("ai_eval",))
        self.assert_ran_without_cargo(self._ai_eval_score())

    def test_ai_eval_refuses_stale_prebuilt_binary(self) -> None:
        self.sandbox.add_binaries("release", stale=True, names=("ai_eval",))
        result = self._ai_eval_score()
        self.assert_refused(result)
        self.assertIn("older than", result.stderr)
        # The ai-eval recipe is its own artifact, never the build-artifacts one.
        self.assertIn("ai-eval-<short-sha>", result.stderr)
        self.assertNotIn("-n build-artifacts", result.stderr)

    def test_ai_eval_compiles_with_opt_in(self) -> None:
        result = self._ai_eval_score(opt_in=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.sandbox.cargo_calls()
        self.assertEqual(len(calls), 1, calls)
        self.assertTrue(
            calls[0].startswith("cargo run --bin ai_eval --features ai-eval -- score-l1"),
            calls,
        )


def _code_part(line: str) -> str:
    """The line with quoted strings and trailing comments removed — a cargo
    word inside a message or comment is not an invocation — plus the bodies of
    strings passed to `sh -c` / `bash -lc`, which are executed."""
    shell_bodies = " ".join(m.group(1)[1:-1] for m in SHELL_C_ARG.finditer(line))
    stripped = QUOTED.sub('""', line)
    code = stripped.split("#", 1)[0] if not stripped.lstrip().startswith("#!") else ""
    return f"{code} {shell_bodies}".rstrip()


def _is_guard(code: str) -> bool:
    return any(g in code for g in SHELL_GUARDS) and not NEGATED_GUARD.search(code)


def ungated_shell_cargo(path: Path) -> list[str]:
    """Compiling cargo invocations in `path` not behind the opt-in guard: a
    top-level (unindented) `require_local_cargo` call earlier in the file
    guards everything after it (it exits the script); otherwise a guard token
    must appear within the preceding GUARD_LOOKBACK lines."""
    lines = path.read_text().splitlines()
    offenders = []
    top_level_guard = False
    for idx, line in enumerate(lines):
        code = _code_part(line)
        if code.startswith("require_local_cargo"):
            top_level_guard = True
        if not SHELL_CARGO_COMPILE.search(code):
            continue
        window = lines[max(0, idx - GUARD_LOOKBACK) : idx + 1]
        guarded = top_level_guard or any(_is_guard(_code_part(prev)) for prev in window)
        if not guarded:
            offenders.append(f"{path.relative_to(REPO_ROOT)}:{idx + 1}: {line.strip()}")
    return offenders


class NoUngatedCargo(unittest.TestCase):
    def test_e2e_cargo_compile_only_in_gated_helper(self) -> None:
        offenders = []
        for ts in sorted(E2E_DIR.rglob("*.ts")):
            if ts == HELPER:
                continue
            for lineno, line in enumerate(ts.read_text().splitlines(), start=1):
                if CARGO_COMPILE.search(line):
                    offenders.append(f"{ts.relative_to(REPO_ROOT)}:{lineno}: {line.strip()}")
        self.assertEqual(
            offenders,
            [],
            "E2E harness must resolve binaries via tests/e2e/prebuilt-binary.ts "
            "(no silent local cargo compile, #802)",
        )

    def test_helper_gates_cargo_on_opt_in(self) -> None:
        self.assertTrue(HELPER.exists(), f"{HELPER} missing")
        text = HELPER.read_text()
        self.assertIn(OPT_IN, text)
        self.assertIn("gh run download", text)
        self.assertRegex(text, r"throw new Error")

    def test_dev_and_ops_scripts_gate_every_cargo_compile(self) -> None:
        offenders = []
        for base in SCRIPT_DIRS:
            for path in sorted(p for p in base.rglob("*") if p.is_file()):
                if path.suffix in NON_CODE_SUFFIXES:
                    continue
                offenders.extend(ungated_shell_cargo(path))
        self.assertEqual(
            offenders,
            [],
            "every compiling cargo call in scripts/dev + scripts/ops must sit behind "
            "require_local_cargo / local_cargo_allowed (scripts/dev/lib/local-cargo.sh, #802)",
        )

    def test_scanner_flags_an_ungated_call(self) -> None:
        with tempfile.TemporaryDirectory(dir=REPO_ROOT / "scripts" / "ci") as tmp:
            probe = Path(tmp) / "probe.sh"
            probe.write_text(
                '#!/usr/bin/env bash\necho "cargo run is only text"\n# cargo build comment\n'
                "if ! cargo clippy -q; then :; fi\n"
            )
            self.assertEqual(len(ungated_shell_cargo(probe)), 1)
            probe.write_text(
                "#!/usr/bin/env bash\ncargo +nightly build\ncargo --locked test\n"
                'bash -lc "cargo nextest run"\n'
                "if ! local_cargo_allowed; then echo skip; fi\ncargo check\n"
            )
            self.assertEqual(len(ungated_shell_cargo(probe)), 4)
            probe.write_text(
                "#!/usr/bin/env bash\nif local_cargo_allowed; then\n  cargo check\nfi\n"
            )
            self.assertEqual(ungated_shell_cargo(probe), [])


if __name__ == "__main__":
    unittest.main()
