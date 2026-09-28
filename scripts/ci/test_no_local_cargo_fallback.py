#!/usr/bin/env python3
"""Regression guard for #802: the E2E harness must never silently compile.

Presenter is Tier-0 (CI-only builds). The E2E setup paths used to fall back to
`cargo run` when a prebuilt binary was missing or stale; that ran inside node /
a `.sh`, where the Bash-level `block-tier0-local-build.sh` hook cannot see it,
so a local E2E run quietly compiled the workspace. The rule now: use the
prebuilt binary (CI artifact) or FAIL loudly with a `gh run download` recipe;
compiling happens only with the explicit opt-in `PRESENTER_ALLOW_LOCAL_CARGO=1`.

Two layers:
  * behavioural — run the real `scripts/dev/refresh-dev-data.sh` in a sandbox
    repo with a fake `cargo` on PATH and assert it is never invoked unless the
    opt-in is set;
  * structural — every `cargo run` / `cargo build` in the TypeScript E2E
    harness lives ONLY in `tests/e2e/prebuilt-binary.ts`, which gates it on
    the opt-in.

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
REFRESH_SCRIPT = REPO_ROOT / "scripts" / "dev" / "refresh-dev-data.sh"
E2E_DIR = REPO_ROOT / "tests" / "e2e"
HELPER = E2E_DIR / "prebuilt-binary.ts"
OPT_IN = "PRESENTER_ALLOW_LOCAL_CARGO"
CARGO_COMPILE = re.compile(r"\bcargo\s+(run|build)\b")
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


class RefreshDevDataSandbox:
    """A throwaway repo root holding a copy of refresh-dev-data.sh, the crate
    source dirs its staleness check scans, and a fake `cargo` that only records
    that it was called."""

    def __init__(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="presenter-802-"))
        script = self.root / "scripts" / "dev" / "refresh-dev-data.sh"
        script.parent.mkdir(parents=True)
        shutil.copy2(REFRESH_SCRIPT, script)
        self.script = script
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

    def add_binaries(self, profile: str, stale: bool) -> None:
        for name in IMPORTER_BINS:
            binary = self.root / "target" / profile / name
            _write_executable(binary, "#!/usr/bin/env bash\nexit 0\n")
            if stale:
                old = time.time() - 3600
                os.utime(binary, (old, old))

    def run(self, opt_in: bool = False) -> subprocess.CompletedProcess[str]:
        env = {
            "PATH": f"{self.bin_dir}{os.pathsep}{os.environ['PATH']}",
            "HOME": str(self.root),
            "PRESENTER_DB_URL": f"sqlite://{self.root}/var/test.db",
        }
        if opt_in:
            env[OPT_IN] = "1"
        return subprocess.run(
            ["bash", str(self.script), str(self.root / "libs")],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )

    def cargo_calls(self) -> list[str]:
        if not self.cargo_log.exists():
            return []
        return self.cargo_log.read_text().splitlines()

    def cleanup(self) -> None:
        shutil.rmtree(self.root, ignore_errors=True)


class RefreshDevDataNoSilentCompile(unittest.TestCase):
    def setUp(self) -> None:
        self.sandbox = RefreshDevDataSandbox()

    def tearDown(self) -> None:
        self.sandbox.cleanup()

    def assert_refused(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.sandbox.cargo_calls(), [])
        self.assertIn("gh run download", result.stderr)
        self.assertIn(OPT_IN, result.stderr)

    def test_missing_binary_fails_loudly_without_compiling(self) -> None:
        self.assert_refused(self.sandbox.run())

    def test_stale_binary_fails_loudly_without_compiling(self) -> None:
        self.sandbox.add_binaries("release", stale=True)
        self.assert_refused(self.sandbox.run())

    def test_fresh_prebuilt_binary_is_used(self) -> None:
        self.sandbox.add_binaries("release", stale=False)
        result = self.sandbox.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.sandbox.cargo_calls(), [])

    def test_fresh_debug_binary_is_used(self) -> None:
        self.sandbox.add_binaries("debug", stale=False)
        result = self.sandbox.run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.sandbox.cargo_calls(), [])

    def test_explicit_opt_in_compiles(self) -> None:
        result = self.sandbox.run(opt_in=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            self.sandbox.cargo_calls(),
            [
                f"cargo run -p presenter-importer --bin {name} --"
                + (f" --root {self.sandbox.root / 'libs'}" if name == "import_propresenter" else "")
                for name in IMPORTER_BINS
            ],
        )


class E2eHarnessNoUngatedCargo(unittest.TestCase):
    def test_cargo_compile_only_in_gated_helper(self) -> None:
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


if __name__ == "__main__":
    unittest.main()
